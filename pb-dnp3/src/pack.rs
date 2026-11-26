//! DNP3 master worker.

use std::io::Write;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use pb_abi::PB_LOG_INFO;
use pb_extkit::{ConnExt, LinkCtx, Val};
use pb_proto::dnp3::{self, PValue, Response};
use serde_json::{Map, Value};

pub struct PackState(pub pb_extkit::ExtPack);

fn endpoint(c: &Map<String, Value>) -> String {
    format!(
        "{}:{} · master {} → outstation {}",
        c.s("host", "127.0.0.1"),
        c.n("port", 20000.0) as u16,
        c.n("master_address", 1.0) as u16,
        c.n("outstation_address", 10.0) as u16
    )
}

fn check(c: &Map<String, Value>) -> Result<(), String> {
    for k in ["master_address", "outstation_address"] {
        let v = c.n(k, 1.0);
        if !(0.0..=65519.0).contains(&v) {
            return Err(format!("{k} {v} out of range (0–65519)"));
        }
    }
    Ok(())
}

struct Master {
    s: TcpStream,
    me: u16,
    out: u16,
    app_seq: u8,
    tp_seq: u8,
}

impl Master {
    /// Send a request and wait for its response (unsolicited messages are confirmed and
    /// handed to `on_uns` meanwhile).
    fn transact(&mut self, apdu_for: impl Fn(u8) -> Vec<u8>, timeout: Duration, mut on_uns: impl FnMut(&Response)) -> Result<Response, String> {
        let seq = self.app_seq;
        self.app_seq = (self.app_seq + 1) & 0x0f;
        let req = apdu_for(seq);
        let bytes = dnp3::encode_fragment(true, self.out, self.me, &req, &mut self.tp_seq);
        self.s.write_all(&bytes).map_err(|e| format!("send: {e}"))?;
        let end = Instant::now() + timeout;
        loop {
            if Instant::now() > end {
                return Err("response timeout".into());
            }
            let (_, apdu) = dnp3::read_fragment(&mut self.s, self.me).map_err(|e| match e.kind() {
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => "response timeout".to_string(),
                _ => e.to_string(),
            })?;
            let r = dnp3::parse_response(&apdu)?;
            if r.needs_confirm() {
                let c = dnp3::confirm(r.seq(), r.unsolicited());
                let b = dnp3::encode_fragment(true, self.out, self.me, &c, &mut self.tp_seq);
                self.s.write_all(&b).map_err(|e| format!("send: {e}"))?;
            }
            if r.unsolicited() {
                on_uns(&r);
                continue;
            }
            if r.seq() == seq {
                return Ok(r);
            }
        }
    }
}

fn apply(ctx: &LinkCtx, r: &Response) {
    ctx.status(|s| {
        s.rx += 1;
        for p in &r.points {
            let mut leaf = Map::new();
            leaf.insert("kind".into(), Value::from(p.kind));
            leaf.insert("index".into(), Value::from(p.index));
            let v = match &p.value {
                PValue::Bool(b) => b.to_string(),
                PValue::Int(i) => i.to_string(),
                PValue::Float(f) => format!("{f}"),
            };
            s.see(&format!("{}:{:05}", p.kind, p.index), leaf, format!("{} {}", p.kind, p.index), if p.online { v } else { format!("{v} (offline)") });
        }
    });
    for p in &r.points {
        for b in ctx.inputs.iter().filter(|b| b.text("kind") == p.kind && b.num("index") == Some(p.index as f64)) {
            if !p.online {
                ctx.publish_bad(b);
                continue;
            }
            let (s, o) = b.scaling();
            let v = match &p.value {
                PValue::Bool(x) => Val::Bool(*x),
                PValue::Int(i) if s != 1.0 || o != 0.0 => Val::Float(*i as f64 * s + o),
                PValue::Int(i) => Val::Int(*i),
                PValue::Float(f) => Val::Float(f * s + o),
            };
            ctx.publish(b, &v);
        }
    }
}

pub fn run(mut ctx: LinkCtx) {
    let host = ctx.conn.s("host", "127.0.0.1");
    let port = ctx.conn.n("port", 20000.0) as u16;
    let me = ctx.conn.n("master_address", 1.0) as u16;
    let out = ctx.conn.n("outstation_address", 10.0) as u16;
    let timeout = Duration::from_millis(ctx.conn.n("timeout_ms", 3000.0).clamp(100.0, 60_000.0) as u64);
    let poll = ctx.input_interval(Duration::from_secs(2));
    let mut backoff = Duration::from_millis(500);
    while !ctx.stopped() {
        ctx.status(|s| s.state = "connecting".into());
        let stream = (host.as_str(), port)
            .to_socket_addrs()
            .map_err(|e| e.to_string())
            .and_then(|mut a| a.next().ok_or_else(|| format!("cannot resolve {host}")))
            .and_then(|a| TcpStream::connect_timeout(&a, Duration::from_secs(3)).map_err(|e| e.to_string()));
        let s = match stream {
            Ok(s) => s,
            Err(e) => {
                ctx.down(format!("{host}:{port}: {e}"));
                if !pb_extkit::sleep_stop(&ctx.stop, backoff) {
                    break;
                }
                backoff = (backoff * 2).min(Duration::from_secs(10));
                continue;
            }
        };
        let _ = s.set_read_timeout(Some(timeout));
        let _ = s.set_nodelay(true);
        backoff = Duration::from_millis(500);
        let mut m = Master { s, me, out, app_seq: 0, tp_seq: 0 };
        ctx.up(format!("connected {host}:{port}"));
        ctx.log(PB_LOG_INFO, &format!("DNP3 master connected {host}:{port}"));
        let mut next_poll = Instant::now();
        let err: Option<String> = loop {
            if ctx.stopped() {
                break None;
            }
            // Controls first.
            let writes = ctx.wait_writes(next_poll.saturating_duration_since(Instant::now()).min(Duration::from_millis(100)));
            let mut failed = None;
            for (point, value) in writes {
                let binds: Vec<pb_extkit::Bind> = ctx.outputs_for(&point).cloned().collect();
                for b in binds {
                    let Some(index) = b.num("index").map(|i| i as u16) else { continue };
                    let v = Val::parse_text(&value);
                    let res = match b.text("kind").as_str() {
                        "binary_output" | "binary" => {
                            let Some(on) = v.as_bool() else { continue };
                            m.transact(|q| dnp3::operate_binary(q, index, on), timeout, |r| apply(&ctx, r))
                        }
                        _ => {
                            let Some(x) = v.as_f64() else { continue };
                            let (s, o) = b.scaling();
                            let raw = (x - o) / s;
                            let float = !matches!(b.fields.get("float"), Some(Value::Bool(false)));
                            m.transact(|q| dnp3::operate_analog(q, index, raw, float), timeout, |r| apply(&ctx, r))
                        }
                    };
                    match res {
                        Ok(r) if r.control_status.iter().all(|c| *c == 0) => ctx.status(|s| s.tx += 1),
                        Ok(r) => ctx.status(|s| {
                            s.errors += 1;
                            s.error = format!("control {} {index} refused (status {:?})", b.text("kind"), r.control_status);
                        }),
                        Err(e) => {
                            failed = Some(e);
                            break;
                        }
                    }
                }
            }
            if let Some(e) = failed {
                break Some(e);
            }
            if Instant::now() >= next_poll {
                next_poll = Instant::now() + poll;
                match m.transact(|q| dnp3::read_classes(q, true, true), timeout, |r| apply(&ctx, r)) {
                    Ok(r) => {
                        apply(&ctx, &r);
                        if r.iin1 & dnp3::IIN1_RESTART != 0 {
                            let _ = m.transact(dnp3::clear_restart, timeout, |r| apply(&ctx, r));
                        }
                        ctx.status(|s| {
                            if s.state != "error" {
                                s.state = format!("polling {host}:{port}");
                            }
                        });
                    }
                    Err(e) => break Some(e),
                }
            }
        };
        match err {
            None => break,
            Some(e) => {
                ctx.down(format!("{host}:{port}: {e}"));
                if !pb_extkit::sleep_stop(&ctx.stop, backoff) {
                    break;
                }
            }
        }
    }
}

pb_extkit::ext_pack_impl!(PackState, b"dnp3\0", b"DNP3\0", "dnp3", run, endpoint, check);
pb_abi::export_pack!(PackState);

#[cfg(test)]
mod tests {
    use super::*;
    use pb_extkit::{testhost, ExtPack};
    use pb_proto::dnp3::Outstation;
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    fn outstation() -> (u16, Arc<Mutex<Outstation>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let mut o = Outstation::new(10);
        o.analogs.insert(0, 1500.0);
        o.analogs.insert(1, 20.0);
        o.analogs.insert(2, 0.0);
        o.offline_analogs.push(2);
        o.binaries.insert(0, true);
        o.counters.insert(3, 99);
        let db = Arc::new(Mutex::new(o));
        let d2 = db.clone();
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let mut s = s;
                // Serve with a short lock per request so the test can change values.
                loop {
                    let (master, apdu) = match dnp3::read_fragment(&mut s, 10) {
                        Ok(x) => x,
                        Err(_) => break,
                    };
                    let mut o = d2.lock().unwrap();
                    if let Some(resp) = o.handle(&apdu) {
                        let mut seq = 0;
                        let b = dnp3::encode_fragment(false, master, 10, &resp, &mut seq);
                        drop(o);
                        if s.write_all(&b).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        (port, db)
    }

    #[test]
    fn master_polls_points_clears_restart_and_operates_controls() {
        let (port, db) = outstation();
        let (log, host) = testhost::host();
        let mut p = ExtPack::new(host, "dnp3", run, endpoint, check);
        p.configure(
            &serde_json::json!({
                "connections":[{"id":"rtu","host":"127.0.0.1","port":port,"master_address":1,"outstation_address":10,"timeout_ms":1000}],
                "inputs":[{"connection":"rtu","interval_secs":0.1,"binds":[
                    {"kind":"analog","index":0,"point":"psi","source":"dnp3/rtu::analog:0"},
                    {"kind":"analog","index":1,"point":"temp","source":"dnp3/rtu::analog:1","scale":1.8,"offset":32},
                    {"kind":"analog","index":2,"point":"off","source":"dnp3/rtu::analog:2"},
                    {"kind":"binary","index":0,"point":"run","source":"dnp3/rtu::binary:0"},
                    {"kind":"counter","index":3,"point":"strokes","source":"dnp3/rtu::counter:3"}]}],
                "outputs":[{"connection":"rtu","binds":[
                    {"kind":"binary_output","index":1,"point":"start"},
                    {"kind":"analog_output","index":0,"point":"choke"}]}]
            })
            .to_string(),
        )
        .unwrap();
        testhost::wait(log, "analog", |l| l.iter().any(|r| r.name == "psi" && r.value == Val::Float(1500.0)));
        testhost::wait(log, "scaled", |l| l.iter().any(|r| r.name == "temp" && r.value == Val::Float(68.0)));
        testhost::wait(log, "offline → bad", |l| l.iter().any(|r| r.name == "off" && r.quality == pb_abi::PB_QUALITY_BAD));
        assert_eq!(testhost::last_good(log, "run"), Some(Val::Bool(true)));
        assert_eq!(testhost::last_good(log, "strokes"), Some(Val::Int(99)));
        // Restart cleared by the master.
        let end = Instant::now() + Duration::from_secs(5);
        while db.lock().unwrap().restart {
            assert!(Instant::now() < end, "restart never cleared");
            std::thread::sleep(Duration::from_millis(20));
        }
        p.write_point("start", "true");
        p.write_point("choke", "42.5");
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            let o = db.lock().unwrap();
            if o.binary_outputs.get(&1) == Some(&true) && o.analog_outputs.get(&0) == Some(&42.5) {
                break;
            }
            drop(o);
            assert!(Instant::now() < end, "controls never landed");
            std::thread::sleep(Duration::from_millis(20));
        }
        db.lock().unwrap().analogs.insert(0, 1600.0);
        testhost::wait(log, "update", |l| l.iter().any(|r| r.name == "psi" && r.value == Val::Float(1600.0)));
    }

    #[test]
    fn dead_outstation_goes_bad() {
        let port = testhost::free_port();
        let (log, host) = testhost::host();
        let mut p = ExtPack::new(host, "dnp3", run, endpoint, check);
        p.configure(
            &serde_json::json!({
                "connections":[{"id":"rtu","host":"127.0.0.1","port":port}],
                "inputs":[{"connection":"rtu","binds":[{"kind":"analog","index":0,"point":"psi","source":"dnp3/rtu::analog:0"}]}]
            })
            .to_string(),
        )
        .unwrap();
        testhost::wait(log, "bad", |l| l.iter().any(|r| r.name == "psi" && r.quality == pb_abi::PB_QUALITY_BAD));
        assert!(p.last_error().contains("127.0.0.1"));
        let c = serde_json::json!({"master_address":70000});
        assert!(check(c.as_object().unwrap()).is_err());
    }
}
