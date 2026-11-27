//! WebSocket pack worker.

use std::io::ErrorKind;
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use pb_abi::PB_LOG_INFO;
use pb_extkit::{leaf1, now_ms, ConnExt, LinkCtx, Val};
use serde_json::{Map, Value};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

pub struct PackState(pub pb_extkit::ExtPack);

fn is_server(c: &Map<String, Value>) -> bool {
    matches!(c.s("mode", "client").to_ascii_lowercase().as_str(), "server" | "listen")
}

fn endpoint(c: &Map<String, Value>) -> String {
    if is_server(c) {
        format!("listen ws://{}:{}{}", c.s("host", "0.0.0.0"), c.n("port", 8765.0) as u16, c.s("path", "/"))
    } else {
        c.s("url", "ws://127.0.0.1:8765/")
    }
}

fn check(c: &Map<String, Value>) -> Result<(), String> {
    if is_server(c) {
        let p = c.n("port", 8765.0);
        if !(1.0..=65535.0).contains(&p) {
            return Err(format!("port {p} out of range"));
        }
        Ok(())
    } else {
        let url = c.s("url", "");
        if url.starts_with("ws://") || url.starts_with("wss://") {
            Ok(())
        } else {
            Err("url must start with ws:// or wss://".into())
        }
    }
}

/// Every leaf of a JSON document as (dotted path, text), for the bind picker.
pub fn leaves(v: &Value, prefix: &str, out: &mut Vec<(String, String)>) {
    if out.len() > 200 {
        return;
    }
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                let p = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
                leaves(x, &p, out);
            }
        }
        Value::Array(a) => {
            for (i, x) in a.iter().enumerate().take(20) {
                leaves(x, &format!("{prefix}[{i}]"), out);
            }
        }
        other => out.push((prefix.to_string(), pb_json::value_text(other))),
    }
}

/// Apply one received text message to the Input binds.
fn on_message(ctx: &LinkCtx, text: &str) {
    let doc: Option<Value> = serde_json::from_str(text).ok();
    ctx.status(|s| {
        s.rx += 1;
        match &doc {
            Some(d) => {
                let mut l = Vec::new();
                leaves(d, "", &mut l);
                for (p, v) in l {
                    if !p.is_empty() {
                        s.see(&p, leaf1("path", p.clone()), p.clone(), v);
                    }
                }
            }
            None => s.see("", leaf1("path", ""), "(whole message)", text.chars().take(80).collect::<String>()),
        }
    });
    for b in &ctx.inputs {
        let path = b.text("path");
        let val = if path.trim().is_empty() {
            Some(match &doc {
                Some(Value::Object(_)) | Some(Value::Array(_)) | None => Val::parse_text(text),
                Some(d) => Val::from_json(d),
            })
        } else {
            doc.as_ref().and_then(|d| pb_json::get_path(d, &path)).map(Val::from_json)
        };
        // A message without this path leaves the point as it was (feeds often mix message types).
        let Some(v) = val else { continue };
        let (scale, offset) = b.scaling();
        let v = match v.as_f64() {
            Some(x) if (scale != 1.0 || offset != 0.0) && !matches!(v, Val::Bool(_)) => Val::Float(x * scale + offset),
            _ => v,
        };
        ctx.publish(b, &v);
    }
}

/// Output messages for writes that arrived. `object`: one object with every key (when anything
/// changed); `change`: one message per write.
pub fn output_messages(ctx: &LinkCtx, changed: &[(String, String)], format: &str) -> Vec<String> {
    if changed.is_empty() {
        return Vec::new();
    }
    let key_of = |b: &pb_extkit::Bind| {
        let k = b.text("key");
        if k.trim().is_empty() { b.point.clone() } else { k.trim().to_string() }
    };
    if format == "change" {
        let mut out = Vec::new();
        for (p, v) in changed {
            for b in ctx.outputs_for(p) {
                out.push(
                    serde_json::json!({"point": p, "key": key_of(b), "value": Val::parse_text(v).to_json(), "ts": now_ms()})
                        .to_string(),
                );
            }
        }
        out
    } else {
        let mut m = Map::new();
        m.insert("ts".into(), Value::from(now_ms()));
        for b in &ctx.outputs {
            if let Some(v) = ctx.latest.get(&b.point) {
                if !v.trim().is_empty() {
                    m.insert(key_of(b), Val::parse_text(v).to_json());
                }
            }
        }
        vec![Value::Object(m).to_string()]
    }
}

fn set_timeout(ws: &WebSocket<MaybeTlsStream<TcpStream>>, d: Duration) {
    match ws.get_ref() {
        MaybeTlsStream::Plain(s) => {
            let _ = s.set_read_timeout(Some(d));
        }
        MaybeTlsStream::Rustls(s) => {
            let _ = s.get_ref().set_read_timeout(Some(d));
        }
        _ => {}
    }
}

fn would_block(e: &tungstenite::Error) -> bool {
    matches!(e, tungstenite::Error::Io(io) if matches!(io.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut))
}

pub fn run(ctx: LinkCtx) {
    if is_server(&ctx.conn) {
        run_server(ctx)
    } else {
        run_client(ctx)
    }
}

fn run_client(mut ctx: LinkCtx) {
    let url = ctx.conn.s("url", "");
    let sub = ctx.conn.s("subscribe_message", "");
    let format = ctx.conn.s("out_format", "object");
    let every = ctx.output_interval(Duration::from_millis(500));
    let mut backoff = Duration::from_millis(500);
    while !ctx.stopped() {
        ctx.status(|s| s.state = "connecting".into());
        let mut ws = match tungstenite::connect(url.as_str()) {
            Ok((ws, _)) => ws,
            Err(e) => {
                ctx.down(format!("{url}: {e}"));
                if !pb_extkit::sleep_stop(&ctx.stop, backoff) {
                    break;
                }
                backoff = (backoff * 2).min(Duration::from_secs(10));
                continue;
            }
        };
        backoff = Duration::from_millis(500);
        set_timeout(&ws, Duration::from_millis(30));
        ctx.up(format!("connected {url}"));
        ctx.log(PB_LOG_INFO, &format!("WebSocket connected {url}"));
        if !sub.trim().is_empty() && ws.send(Message::text(sub.clone())).is_err() {
            ctx.down("subscribe send failed");
            continue;
        }
        let mut pending: Vec<(String, String)> = Vec::new();
        let mut next = Instant::now();
        let err: Option<String> = loop {
            if ctx.stopped() {
                let _ = ws.close(None);
                break None;
            }
            match ws.read() {
                Ok(Message::Text(t)) => on_message(&ctx, &t),
                Ok(Message::Binary(b)) => on_message(&ctx, &String::from_utf8_lossy(&b)),
                Ok(Message::Close(_)) => break Some("server closed the connection".into()),
                Ok(_) => {}
                Err(e) if would_block(&e) => {}
                Err(e) => break Some(e.to_string()),
            }
            pending.extend(ctx.drain_writes());
            if !pending.is_empty() && Instant::now() >= next {
                next = Instant::now() + every;
                let mut failed = None;
                for m in output_messages(&ctx, &pending, &format) {
                    if let Err(e) = ws.send(Message::text(m)) {
                        failed = Some(e.to_string());
                        break;
                    }
                    ctx.status(|s| s.tx += 1);
                }
                pending.clear();
                if let Some(e) = failed {
                    break Some(e);
                }
            }
        };
        match err {
            None => break,
            Some(e) => {
                ctx.down(format!("{url}: {e}"));
                if !pb_extkit::sleep_stop(&ctx.stop, backoff) {
                    break;
                }
            }
        }
    }
}

/// Built-in viewer page: any plain HTTP GET on the server port gets this, so a browser
/// can confirm the feed by opening `http://host:port/`.
const VIEWER_HTML: &str = include_str!("viewer.html");

fn route(req: &pb_webserve::Request) -> pb_webserve::Response {
    use pb_webserve::Response;
    if req.method != "GET" && req.method != "HEAD" {
        return Response::method_not_allowed();
    }
    if req.path == "/" || req.path.starts_with("/viewer") {
        Response::html(VIEWER_HTML)
    } else {
        Response::text(404, "Not found — the viewer lives at /")
    }
}

fn run_server(mut ctx: LinkCtx) {
    let host = ctx.conn.s("host", "0.0.0.0");
    let port = ctx.conn.n("port", 8765.0) as u16;
    let format = ctx.conn.s("out_format", "object");
    let every = ctx.output_interval(Duration::from_millis(500));
    while !ctx.stopped() {
        let listener = match TcpListener::bind((host.as_str(), port)) {
            Ok(l) => l,
            Err(e) => {
                ctx.down(format!("bind {host}:{port}: {e}"));
                if !pb_extkit::sleep_stop(&ctx.stop, Duration::from_secs(2)) {
                    return;
                }
                continue;
            }
        };
        let _ = listener.set_nonblocking(true);
        ctx.up(format!("listening ws://{host}:{port}"));
        let mut peers: Vec<WebSocket<TcpStream>> = Vec::new();
        let (joined_tx, joined_rx) = std::sync::mpsc::channel::<WebSocket<TcpStream>>();
        let mut pending: Vec<(String, String)> = Vec::new();
        let mut next = Instant::now();
        while !ctx.stopped() {
            while let Ok((s, _)) = listener.accept() {
                // Classify + handshake off the worker thread so a slow or silent client can't
                // stall the live feed to everyone else.
                let tx = joined_tx.clone();
                std::thread::spawn(move || {
                    let _ = s.set_nonblocking(false);
                    let s = match pb_webserve::classify(s, Duration::from_secs(3)) {
                        pb_webserve::Incoming::WebSocket(s) => s,
                        pb_webserve::Incoming::Http(req, mut s) => {
                            let _ = pb_webserve::respond(&mut s, route(&req));
                            return;
                        }
                        pb_webserve::Incoming::Dropped => return,
                    };
                    if let Ok(ws) = tungstenite::accept(s) {
                        let _ = tx.send(ws);
                    }
                });
            }
            while let Ok(mut ws) = joined_rx.try_recv() {
                let _ = ws.get_ref().set_read_timeout(Some(Duration::from_millis(5)));
                // A new viewer gets the current picture straight away.
                if !ctx.outputs.is_empty() {
                    let all: Vec<(String, String)> = ctx.latest.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                    for m in output_messages(&ctx, &all, "object") {
                        let _ = ws.send(Message::text(m));
                    }
                }
                peers.push(ws);
            }
            let mut texts = Vec::new();
            peers.retain_mut(|ws| loop {
                match ws.read() {
                    Ok(Message::Text(t)) => texts.push(t.to_string()),
                    Ok(Message::Binary(b)) => texts.push(String::from_utf8_lossy(&b).into_owned()),
                    Ok(Message::Close(_)) => return false,
                    Ok(_) => {}
                    Err(e) if would_block(&e) => return true,
                    Err(_) => return false,
                }
            });
            for t in texts {
                on_message(&ctx, &t);
            }
            let n = peers.len() as u64;
            ctx.status(|s| {
                s.peers = Some(n);
                s.state = format!("listening ws://{host}:{port} — {n} client{}", if n == 1 { "" } else { "s" });
            });
            pending.extend(ctx.wait_writes(Duration::from_millis(20)));
            if !pending.is_empty() && Instant::now() >= next {
                next = Instant::now() + every;
                let msgs = output_messages(&ctx, &pending, &format);
                pending.clear();
                peers.retain_mut(|ws| msgs.iter().all(|m| ws.send(Message::text(m.clone())).is_ok()));
                if !msgs.is_empty() {
                    ctx.status(|s| s.tx += msgs.len() as u64);
                }
            }
        }
        for mut ws in peers {
            let _ = ws.close(None);
        }
    }
}

pb_extkit::ext_pack_impl!(PackState, b"websocket\0", b"WebSocket\0", "websocket", run, endpoint, check);
pb_abi::export_pack!(PackState);

#[cfg(test)]
mod tests {
    use super::*;
    use pb_extkit::{testhost, ExtPack};

    #[test]
    fn server_feeds_browsers_and_reads_their_messages() {
        let (log, host) = testhost::host();
        let port = testhost::free_port();
        let mut p = ExtPack::new(host, "websocket", run, endpoint, check);
        p.configure(
            &serde_json::json!({
                "connections":[{"id":"dash","mode":"server","host":"127.0.0.1","port":port}],
                "inputs":[{"connection":"dash","binds":[{"path":"cmd.speed","point":"speed","source":"websocket/dash::cmd.speed","scale":2}]}],
                "outputs":[{"connection":"dash","interval_secs":0.05,"binds":[{"point":"psi","key":"pressure"},{"point":"rate"}]}]
            })
            .to_string(),
        )
        .unwrap();
        p.write_point("psi", "3400");
        let url = format!("ws://127.0.0.1:{port}/");
        let mut ws = loop {
            if let Ok((ws, _)) = tungstenite::connect(url.as_str()) {
                break ws;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        // First message: the current picture.
        let first: Value = serde_json::from_str(&ws.read().unwrap().into_text().unwrap()).unwrap();
        assert_eq!(first["pressure"], 3400);
        p.write_point("rate", "82.5");
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            let m: Value = serde_json::from_str(&ws.read().unwrap().into_text().unwrap()).unwrap();
            if m["rate"] == 82.5 {
                assert_eq!(m["pressure"], 3400);
                break;
            }
            assert!(Instant::now() < end);
        }
        ws.send(Message::text(r#"{"cmd":{"speed":21}}"#)).unwrap();
        testhost::wait(log, "speed", |l| l.iter().any(|r| r.name == "speed" && r.value == Val::Float(42.0)));
        let d: Value = serde_json::from_str(&p.detail_json()).unwrap();
        assert_eq!(d["connections"][0]["peers"], 1);
        assert!(d["connections"][0]["seen"].as_array().unwrap().iter().any(|x| x["leaf"]["path"] == "cmd.speed"));
    }

    #[test]
    fn plain_http_get_serves_the_viewer_page() {
        use std::io::{Read, Write};
        let (_log, host) = testhost::host();
        let port = testhost::free_port();
        let mut p = ExtPack::new(host, "websocket", run, endpoint, check);
        p.configure(&serde_json::json!({"connections":[{"id":"dash","mode":"server","host":"127.0.0.1","port":port}]}).to_string()).unwrap();
        let mut c = loop {
            if let Ok(c) = TcpStream::connect(("127.0.0.1", port)) {
                break c;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        c.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
        let mut out = String::new();
        c.read_to_string(&mut out).unwrap();
        assert!(out.starts_with("HTTP/1.1 200"), "{out}");
        assert!(out.contains("new WebSocket"));
    }

    #[test]
    fn client_reconnects_and_change_format() {
        let (log, host) = testhost::host();
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let mut p = ExtPack::new(host, "websocket", run, endpoint, check);
        p.configure(
            &serde_json::json!({
                "connections":[{"id":"feed","url":format!("ws://127.0.0.1:{port}/"),"out_format":"change","subscribe_message":"{\"sub\":\"all\"}"}],
                "inputs":[{"connection":"feed","binds":[{"path":"","point":"raw","source":"websocket/feed"},{"path":"t","point":"t","source":"websocket/feed::t"}]}],
                "outputs":[{"connection":"feed","interval_secs":0.05,"binds":[{"point":"x"}]}]
            })
            .to_string(),
        )
        .unwrap();
        let (s, _) = l.accept().unwrap();
        let mut ws = tungstenite::accept(s).unwrap();
        assert_eq!(ws.read().unwrap().into_text().unwrap(), r#"{"sub":"all"}"#);
        ws.send(Message::text(r#"{"t":21.5}"#)).unwrap();
        testhost::wait(log, "t", |l| l.iter().any(|r| r.name == "t" && r.value == Val::Float(21.5)));
        ws.send(Message::text("17")).unwrap();
        testhost::wait(log, "raw", |l| l.iter().any(|r| r.name == "raw" && r.value == Val::Int(17)));
        p.write_point("x", "true");
        let m: Value = serde_json::from_str(&ws.read().unwrap().into_text().unwrap()).unwrap();
        assert_eq!((m["point"].as_str(), m["value"].as_bool()), (Some("x"), Some(true)));
        drop(ws);
        testhost::wait(log, "bad after drop", |l| l.iter().any(|r| r.name == "t" && r.quality == pb_abi::PB_QUALITY_BAD));
        let (s, _) = l.accept().unwrap();
        let mut ws = tungstenite::accept(s).unwrap();
        assert_eq!(ws.read().unwrap().into_text().unwrap(), r#"{"sub":"all"}"#);
    }

    #[test]
    fn urls_are_checked_and_leaves_listed() {
        let c = serde_json::json!({"url":"http://x"});
        assert!(check(c.as_object().unwrap()).is_err());
        let mut out = Vec::new();
        leaves(&serde_json::json!({"a":{"b":1},"c":[true]}), "", &mut out);
        assert_eq!(out, vec![("a.b".to_string(), "1".to_string()), ("c[0]".to_string(), "true".to_string())]);
    }
}
