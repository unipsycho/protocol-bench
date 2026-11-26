//! WITS pack: one worker per connection (see the crate docs for behaviour).

use std::time::{Duration, Instant};

use pb_abi::{PB_LOG_INFO, PB_QUALITY_BAD};
use pb_extkit::link::{take_lines, ByteLink, Transport};
use pb_extkit::{leaf1, ConnExt, LinkCtx, Val};
use serde_json::{Map, Value};

use crate::codec::{self, Parser};

pub const DEFAULT_PORT: u16 = 5100;

pub struct PackState(pub pb_extkit::ExtPack);

fn endpoint(c: &Map<String, Value>) -> String {
    Transport::from_conn(c, DEFAULT_PORT).map(|t| t.endpoint()).unwrap_or_else(|e| e)
}

fn check(c: &Map<String, Value>) -> Result<(), String> {
    Transport::from_conn(c, DEFAULT_PORT).map(|_| ())
}

fn nulls(c: &Map<String, Value>) -> Vec<f64> {
    c.s("null_values", "-9999,-999.25").split([',', ' ', ';']).filter_map(|s| s.trim().parse().ok()).collect()
}

/// Run one WITS connection until stopped.
pub fn run(mut ctx: LinkCtx) {
    let Ok(transport) = Transport::from_conn(&ctx.conn, DEFAULT_PORT) else { return };
    let nulls = nulls(&ctx.conn);
    let stale = Duration::from_secs_f64(ctx.conn.n("stale_secs", 15.0).max(0.5));
    let send_every = ctx.output_interval(Duration::from_secs(1));
    let mut backoff = Duration::from_millis(500);
    while !ctx.stopped() {
        ctx.status(|s| s.state = "connecting".into());
        let mut link = match ByteLink::open(&transport) {
            Ok(l) => l,
            Err(e) => {
                ctx.down(format!("{}: {e}", transport.endpoint()));
                if !pb_extkit::sleep_stop(&ctx.stop, backoff) {
                    break;
                }
                backoff = (backoff * 2).min(Duration::from_secs(10));
                continue;
            }
        };
        backoff = Duration::from_millis(500);
        ctx.up(match &transport {
            Transport::TcpListen { .. } => format!("{} — waiting for peers", transport.endpoint()),
            _ => format!("connected {}", transport.endpoint()),
        });
        ctx.log(PB_LOG_INFO, &format!("WITS up on {}", transport.endpoint()));
        let mut buf = Vec::new();
        let mut parser = Parser::default();
        let mut last_rx = Instant::now();
        let mut stale_marked = false;
        let mut next_send = Instant::now();
        let result: Result<(), String> = loop {
            if ctx.stopped() {
                break Ok(());
            }
            match link.read(&mut buf) {
                Ok(_) => {}
                Err(e) => break Err(format!("{}: {e}", transport.endpoint())),
            }
            for line in take_lines(&mut buf) {
                if let Some(block) = parser.feed(&line) {
                    last_rx = Instant::now();
                    stale_marked = false;
                    on_block(&ctx, &block, &nulls);
                }
            }
            if let Some(n) = link.peers() {
                ctx.status(|s| {
                    s.peers = Some(n);
                    s.state = format!("{} — {n} peer{}", transport.endpoint(), if n == 1 { "" } else { "s" });
                });
            }
            // Inputs go bad when the sender goes quiet (link may still be "up").
            if !ctx.inputs.is_empty() && !stale_marked && last_rx.elapsed() > stale {
                for b in &ctx.inputs {
                    ctx.host.upsert_text(&b.point, &b.source, "", PB_QUALITY_BAD);
                }
                ctx.status(|s| s.error = format!("no WITS data for {:.0} s", stale.as_secs_f64()));
                stale_marked = true;
            }
            ctx.drain_writes();
            if !ctx.outputs.is_empty() && Instant::now() >= next_send {
                next_send = Instant::now() + send_every;
                let items = output_items(&ctx);
                if !items.is_empty() {
                    let text = codec::encode_block(&items);
                    if let Err(e) = link.write_all(text.as_bytes()) {
                        break Err(format!("send: {e}"));
                    }
                    ctx.status(|s| s.tx += 1);
                }
            }
        };
        match result {
            Ok(()) => break,
            Err(e) => {
                ctx.down(e);
                if !pb_extkit::sleep_stop(&ctx.stop, backoff) {
                    break;
                }
            }
        }
    }
}

fn on_block(ctx: &LinkCtx, block: &[codec::WitsItem], nulls: &[f64]) {
    ctx.status(|s| {
        s.rx += 1;
        if s.error.starts_with("no WITS data") {
            s.error.clear();
        }
        for it in block {
            let name = codec::item_name(&it.code);
            let label = if name.is_empty() { it.code.clone() } else { format!("{} {name}", it.code) };
            s.see(&it.code, leaf1("item", it.code.clone()), label, it.value.clone());
        }
    });
    for it in block {
        for b in ctx.inputs.iter().filter(|b| codec::normalize_code(&b.text("item")).as_deref() == Some(it.code.as_str())) {
            if codec::is_null(&it.value, nulls) {
                ctx.publish_bad(b);
                continue;
            }
            let (scale, offset) = b.scaling();
            let v = match Val::parse_text(&it.value) {
                Val::Int(i) if scale != 1.0 || offset != 0.0 => Val::Float(i as f64 * scale + offset),
                Val::Float(f) => Val::Float(f * scale + offset),
                other => other,
            };
            ctx.publish(b, &v);
        }
    }
}

/// `(code, formatted value)` for every Output bind whose point has a value.
fn output_items(ctx: &LinkCtx) -> Vec<(String, String)> {
    let mut items = Vec::new();
    for b in &ctx.outputs {
        let Some(code) = codec::normalize_code(&b.text("item")) else { continue };
        let Some(raw) = ctx.latest.get(&b.point) else { continue };
        if raw.trim().is_empty() {
            continue;
        }
        let decimals = b.num("decimals").filter(|d| *d >= 0.0).map(|d| d.min(9.0) as u32);
        items.push((code, codec::format_value(raw, decimals)));
    }
    items
}

pb_extkit::ext_pack_impl!(PackState, b"wits\0", b"WITS\0", "wits", run, endpoint, check);
pb_abi::export_pack!(PackState);

#[cfg(test)]
mod tests {
    use super::*;
    use pb_extkit::testhost;
    use pb_extkit::ExtPack;
    use std::io::{BufRead, BufReader, Write};
    use std::net::{TcpListener, TcpStream};

    fn pack(host: pb_abi::HostRef) -> ExtPack {
        ExtPack::new(host, "wits", run, endpoint, check)
    }

    #[test]
    fn listener_reads_blocks_marks_nulls_and_goes_stale() {
        let (log, host) = testhost::host();
        let port = testhost::free_port();
        let mut p = pack(host);
        p.configure(
            &serde_json::json!({
                "connections":[{"id":"van","transport":"tcp_listen","host":"127.0.0.1","port":port,"stale_secs":0.6}],
                "inputs":[{"connection":"van","binds":[
                    {"item":"0108","point":"depth","source":"wits/van::0108"},
                    {"item":"121","point":"spp","source":"wits/van::121","scale":0.001}]}]
            })
            .to_string(),
        )
        .unwrap();
        let mut s = loop {
            if let Ok(s) = TcpStream::connect(("127.0.0.1", port)) {
                break s;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        s.write_all(b"&&\r\n01081234.5\r\n01213500\r\n!!\r\n").unwrap();
        testhost::wait(log, "depth", |l| l.iter().any(|r| r.name == "depth" && r.value == Val::Float(1234.5)));
        testhost::wait(log, "scaled spp", |l| l.iter().any(|r| r.name == "spp" && r.value == Val::Float(3.5)));
        s.write_all(b"&&\r\n0108-9999.00\r\n!!\r\n").unwrap();
        testhost::wait(log, "null → bad", |l| l.iter().any(|r| r.name == "depth" && r.quality == PB_QUALITY_BAD));
        // Quiet sender: every Input goes bad after stale_secs.
        testhost::wait(log, "stale spp", |l| l.iter().any(|r| r.name == "spp" && r.quality == PB_QUALITY_BAD));
        let d: Value = serde_json::from_str(&p.detail_json()).unwrap();
        let seen = d["connections"][0]["seen"].as_array().unwrap();
        assert!(seen.iter().any(|x| x["leaf"]["item"] == "0108" && x["label"] == "0108 Depth bit (meas)"));
    }

    #[test]
    fn client_sends_output_blocks_and_reconnects() {
        let (_log, host) = testhost::host();
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let mut p = pack(host);
        p.configure(
            &serde_json::json!({
                "connections":[{"id":"out","transport":"tcp_client","host":"127.0.0.1","port":port}],
                "outputs":[{"connection":"out","interval_secs":0.1,"binds":[
                    {"item":"0108","point":"depth","decimals":1},
                    {"item":"0121","point":"spp"},
                    {"item":"0130","point":"never"}]}]
            })
            .to_string(),
        )
        .unwrap();
        p.write_point("depth", "100.26");
        p.write_point("spp", "3400");
        let (s, _) = l.accept().unwrap();
        let mut r = BufReader::new(s);
        let mut block = Vec::new();
        let mut line = String::new();
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end {
            line.clear();
            r.read_line(&mut line).unwrap();
            let t = line.trim().to_string();
            if t == "&&" {
                block.clear();
            } else if t == "!!" {
                if block.len() == 2 {
                    break;
                }
            } else {
                block.push(t);
            }
        }
        assert_eq!(block, vec!["0108100.3".to_string(), "01213400".to_string()]);
        // Peer goes away: worker reconnects to the next accept.
        drop(r);
        let (s2, _) = l.accept().unwrap();
        s2.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut r2 = BufReader::new(s2);
        line.clear();
        r2.read_line(&mut line).unwrap();
        assert!(!line.is_empty());
    }

    #[test]
    fn bad_transport_is_reported() {
        let (_log, host) = testhost::host();
        let mut p = pack(host);
        p.configure(r#"{"connections":[{"id":"s","transport":"serial"}]}"#).unwrap();
        assert!(p.last_error().contains("no serial port"));
    }
}
