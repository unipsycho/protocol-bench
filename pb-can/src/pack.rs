//! CAN worker.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use pb_extkit::{ConnExt, LinkCtx, Val};
use pb_proto::can::{Frame, IdMatch, Order, SigType, Signal};
use serde_json::{Map, Value};

pub struct PackState(pub pb_extkit::ExtPack);

fn endpoint(c: &Map<String, Value>) -> String {
    format!("SocketCAN {}", c.s("interface", "can0"))
}

fn check(c: &Map<String, Value>) -> Result<(), String> {
    if !cfg!(target_os = "linux") {
        return Err("SocketCAN needs Linux".into());
    }
    if c.s("interface", "").is_empty() {
        return Err("no CAN interface set".into());
    }
    Ok(())
}

/// Signal from bind fields (`start`, `len`, `order` intel|motorola, `type` unsigned|signed|float).
pub fn signal_of(b: &pb_extkit::Bind) -> Result<Signal, String> {
    let (scale, offset) = b.scaling();
    let s = Signal {
        start: b.num("start").ok_or("no start bit")? as u32,
        len: b.num("len").ok_or("no bit length")? as u32,
        order: if b.text("order").eq_ignore_ascii_case("motorola") { Order::Motorola } else { Order::Intel },
        kind: match b.text("type").to_ascii_lowercase().as_str() {
            "signed" => SigType::Signed,
            "float" => SigType::Float,
            _ => SigType::Unsigned,
        },
        scale,
        offset,
    };
    s.check()?;
    Ok(s)
}

#[cfg(target_os = "linux")]
pub fn run(mut ctx: LinkCtx) {
    use pb_proto::can::CanSocket;
    let iface = ctx.conn.s("interface", "can0");
    let stale = Duration::from_secs_f64(ctx.conn.n("stale_secs", 5.0).max(0.2));
    let send_every = ctx.output_interval(Duration::from_millis(100));
    // Pre-parse binds; bad ones are reported once.
    let mut inputs = Vec::new();
    for b in ctx.inputs.clone() {
        match (IdMatch::parse(&b.text("id")), signal_of(&b)) {
            (Some(m), Ok(s)) => inputs.push((b, m, s, Instant::now(), false)),
            (None, _) => ctx.status(|st| st.error = format!("{}: bad frame id \"{}\"", b.point, b.text("id"))),
            (_, Err(e)) => ctx.status(|st| st.error = format!("{}: {e}", b.point)),
        }
    }
    let mut outputs = Vec::new();
    for b in ctx.outputs.clone() {
        if let (Some(IdMatch::Exact { id, extended }), Ok(s)) = (IdMatch::parse(&b.text("id")), signal_of(&b)) {
            outputs.push((b, id, extended, s));
        }
    }
    let mut frames: BTreeMap<(u32, bool), [u8; 8]> = BTreeMap::new();
    let mut backoff = Duration::from_millis(500);
    while !ctx.stopped() {
        let sock = match CanSocket::open(&iface) {
            Ok(s) => s,
            Err(e) => {
                ctx.down(format!("{iface}: {e}"));
                if !pb_extkit::sleep_stop(&ctx.stop, backoff) {
                    break;
                }
                backoff = (backoff * 2).min(Duration::from_secs(10));
                continue;
            }
        };
        backoff = Duration::from_millis(500);
        ctx.up(format!("bound {iface}"));
        let mut next_send = Instant::now();
        let err = loop {
            if ctx.stopped() {
                break None;
            }
            match sock.read() {
                Ok(Some(f)) => on_frame(&ctx, &f, &mut inputs),
                Ok(None) => {}
                Err(e) => break Some(e.to_string()),
            }
            for (b, _, _, last, marked) in inputs.iter_mut() {
                if !*marked && last.elapsed() > stale {
                    ctx.publish_bad(b);
                    *marked = true;
                }
            }
            for (point, value) in ctx.drain_writes() {
                for (_, id, ext, s) in outputs.iter().filter(|(b, ..)| b.point == point) {
                    if let Some(x) = Val::parse_text(&value).as_f64() {
                        let buf = frames.entry((*id, *ext)).or_insert([0; 8]);
                        let _ = s.encode(buf, x);
                    }
                }
            }
            if !frames.is_empty() && Instant::now() >= next_send {
                next_send = Instant::now() + send_every;
                let mut failed = None;
                for ((id, ext), data) in &frames {
                    let dlc = outputs.iter().find(|(_, i, e, _)| i == id && e == ext).and_then(|(b, ..)| b.num("dlc")).unwrap_or(8.0).clamp(0.0, 8.0) as u8;
                    if let Err(e) = sock.write(&Frame { id: *id, extended: *ext, dlc, data: *data }) {
                        failed = Some(e.to_string());
                        break;
                    }
                    ctx.status(|s| s.tx += 1);
                }
                if let Some(e) = failed {
                    break Some(format!("send: {e}"));
                }
            }
        };
        match err {
            None => break,
            Some(e) => {
                ctx.down(format!("{iface}: {e}"));
                if !pb_extkit::sleep_stop(&ctx.stop, backoff) {
                    break;
                }
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub fn run(ctx: LinkCtx) {
    ctx.down("SocketCAN needs Linux");
}

#[allow(clippy::type_complexity)]
fn on_frame(ctx: &LinkCtx, f: &Frame, inputs: &mut [(pb_extkit::Bind, IdMatch, Signal, Instant, bool)]) {
    ctx.status(|s| {
        s.rx += 1;
        let id = if f.extended { format!("0x{:08X}", f.id) } else { format!("0x{:03X}", f.id) };
        let mut leaf = Map::new();
        leaf.insert("id".into(), Value::from(id.clone()));
        let label = if f.extended { format!("{id} (PGN {})", pb_proto::can::pgn_of(f.id)) } else { id.clone() };
        s.see(&id, leaf, label, f.display());
    });
    for (b, m, s, last, marked) in inputs.iter_mut() {
        if !m.matches(f.id, f.extended) {
            continue;
        }
        *last = Instant::now();
        *marked = false;
        match s.decode(&f.data) {
            Some(x) => {
                let v = if s.scale == 1.0 && s.offset == 0.0 && s.kind != SigType::Float { Val::Int(x as i64) } else { Val::Float(x) };
                ctx.publish(b, &v);
            }
            None => ctx.publish_bad(b),
        }
    }
}

pb_extkit::ext_pack_impl!(PackState, b"can\0", b"CAN\0", "can", run, endpoint, check);
pb_abi::export_pack!(PackState);

#[cfg(test)]
mod tests {
    use super::*;
    use pb_extkit::{testhost, ExtPack};

    fn bind(fields: serde_json::Value) -> pb_extkit::Bind {
        pb_extkit::Bind { fields: fields.as_object().unwrap().clone(), point: "p".into(), source: String::new(), interval: Duration::from_secs(1) }
    }

    #[test]
    fn signals_from_bind_fields() {
        let s = signal_of(&bind(serde_json::json!({"start":24,"len":16,"scale":0.125}))).unwrap();
        assert_eq!((s.start, s.len, s.order, s.kind, s.scale), (24, 16, Order::Intel, SigType::Unsigned, 0.125));
        let s = signal_of(&bind(serde_json::json!({"start":"7","len":"12","order":"Motorola","type":"signed"}))).unwrap();
        assert_eq!((s.order, s.kind), (Order::Motorola, SigType::Signed));
        assert!(signal_of(&bind(serde_json::json!({"len":8}))).is_err());
        assert!(signal_of(&bind(serde_json::json!({"start":0,"len":70}))).is_err());
    }

    #[test]
    fn missing_interface_is_reported() {
        let (log, host) = testhost::host();
        let mut p = ExtPack::new(host, "can", run, endpoint, check);
        p.configure(
            &serde_json::json!({"connections":[{"id":"bus","interface":"pbnocan9"}],
                "inputs":[{"connection":"bus","binds":[{"id":"0x100","start":0,"len":8,"point":"x","source":"can/bus::0x100:0:8"}]}]})
            .to_string(),
        )
        .unwrap();
        testhost::wait(log, "bad", |l| l.iter().any(|r| r.name == "x" && r.quality == pb_abi::PB_QUALITY_BAD));
        assert!(p.last_error().contains("pbnocan9"));
    }

    /// Runs only when a `vcan0` interface exists (see the crate docs).
    #[cfg(target_os = "linux")]
    #[test]
    fn vcan_loopback_when_available() {
        use pb_proto::can::CanSocket;
        let Ok(tx) = CanSocket::open("vcan0") else {
            eprintln!("vcan0 not present — skipping");
            return;
        };
        let (log, host) = testhost::host();
        let mut p = ExtPack::new(host, "can", run, endpoint, check);
        p.configure(
            &serde_json::json!({"connections":[{"id":"bus","interface":"vcan0"}],
                "inputs":[{"connection":"bus","binds":[{"id":"pgn:61444","start":24,"len":16,"scale":0.125,"point":"rpm","source":"can/bus::pgn:61444:24:16"}]}],
                "outputs":[{"connection":"bus","interval_secs":0.05,"binds":[{"id":"0x18FF0001","start":0,"len":16,"point":"sp"}]}]})
            .to_string(),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(200));
        let mut data = [0xFF; 8];
        data[3] = 0xE0;
        data[4] = 0x2E;
        tx.write(&Frame { id: 0x0CF00400, extended: true, dlc: 8, data }).unwrap();
        testhost::wait(log, "rpm", |l| l.iter().any(|r| r.name == "rpm" && r.value == Val::Float(1500.0)));
        p.write_point("sp", "513");
        let end = Instant::now() + Duration::from_secs(3);
        loop {
            if let Ok(Some(f)) = tx.read() {
                if f.id == 0x18FF0001 {
                    assert_eq!(f.data[..2], [0x01, 0x02]);
                    break;
                }
            }
            assert!(Instant::now() < end, "no output frame");
        }
    }
}
