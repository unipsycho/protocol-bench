//! Dashboard pack worker.

use std::collections::BTreeMap;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use pb_dash_model::{PointState, ServerMsg};
use pb_extkit::{now_ms, ConnExt, LinkCtx, Val};
use pb_webserve::{Asset, Incoming, Request, Response};
use serde_json::{Map, Value};
use tungstenite::{Message, WebSocket};

use crate::store::{History, Store};

pub struct PackState(pub pb_extkit::ExtPack);

/// The browser app. Rebuild with `scripts/build-dashboard.sh`.
static ASSETS: &[Asset] = &[
    Asset { path: "/index.html", content_type: "text/html; charset=utf-8", bytes: include_bytes!("../dist/index.html") },
    Asset { path: "/pb-dash-web.js", content_type: "text/javascript; charset=utf-8", bytes: include_bytes!("../dist/pb-dash-web.js") },
    Asset { path: "/pb-dash-web_bg.wasm", content_type: "application/wasm", bytes: include_bytes!("../dist/pb-dash-web_bg.wasm") },
];

pub fn endpoint(c: &Map<String, Value>) -> String {
    format!("http://{}:{}/", c.s("host", "0.0.0.0"), c.n("port", 8780.0) as u16)
}

pub fn check(c: &Map<String, Value>) -> Result<(), String> {
    let p = c.n("port", 8780.0);
    if !(1.0..=65535.0).contains(&p) {
        return Err(format!("port {p} out of range"));
    }
    Ok(())
}

/// What a browser sees of one point.
fn point_state(ctx: &LinkCtx, name: &str, value: &str) -> Option<PointState> {
    if value.trim().is_empty() {
        return None;
    }
    let units = ctx.outputs_for(name).map(|b| b.text("units")).find(|u| !u.is_empty()).unwrap_or_default();
    Some(PointState { v: Val::parse_text(value).to_json(), q: 0, ts: now_ms(), u: units })
}

/// Every exposed point (Output binds only) with a value so far.
fn snapshot(ctx: &LinkCtx, title: &str) -> ServerMsg {
    let mut points = BTreeMap::new();
    for b in &ctx.outputs {
        if let Some(v) = ctx.latest.get(&b.point) {
            if let Some(s) = point_state(ctx, &b.point, v) {
                points.insert(b.point.clone(), s);
            }
        }
    }
    ServerMsg::Snap { title: title.to_string(), points }
}

/// State shared by the worker and the HTTP threads of one connection.
pub struct Shared {
    pub title: String,
    /// False = layouts are read-only (kiosk displays).
    pub allow_edit: bool,
    pub store: Mutex<Store>,
    pub history: Mutex<History>,
}

fn route(req: &Request, sh: &Shared) -> Response {
    let json = |v: Value| Response::json(200, v.to_string());
    if let Some(name) = req.path.strip_prefix("/api/dashboards/") {
        return match req.method.as_str() {
            "GET" | "HEAD" => match sh.store.lock().ok().and_then(|s| s.get(name)) {
                Some(text) => Response::json(200, text),
                None => Response::not_found(),
            },
            "PUT" => {
                if !sh.allow_edit {
                    return Response::text(403, "This dashboard is read-only");
                }
                let text = String::from_utf8_lossy(&req.body).into_owned();
                match sh.store.lock().map_err(|_| "busy".to_string()).and_then(|mut s| s.put(name, &text)) {
                    Ok(()) => Response::json(200, r#"{"ok":true}"#),
                    Err(e) => Response::bad_request(&e),
                }
            }
            "DELETE" => {
                if !sh.allow_edit {
                    return Response::text(403, "This dashboard is read-only");
                }
                if sh.store.lock().map(|mut s| s.delete(name)).unwrap_or(false) {
                    Response::json(200, r#"{"ok":true}"#)
                } else {
                    Response::not_found()
                }
            }
            _ => Response::method_not_allowed(),
        };
    }
    if req.method != "GET" && req.method != "HEAD" {
        return Response::method_not_allowed();
    }
    match req.path.as_str() {
        "/api/health" => json(serde_json::json!({"ok": true, "title": sh.title})),
        "/api/dashboards" => {
            let names = sh.store.lock().map(|s| s.list()).unwrap_or_default();
            json(serde_json::json!({"names": names, "title": sh.title, "can_edit": sh.allow_edit}))
        }
        "/api/history" => {
            let since = req.query_param("since").and_then(|v| v.parse().ok()).unwrap_or(0u64);
            let max = req.query_param("max").and_then(|v| v.parse().ok()).unwrap_or(600usize).clamp(4, 5000);
            let mut out = serde_json::Map::new();
            if let (Some(points), Ok(h)) = (req.query_param("points"), sh.history.lock()) {
                for p in points.split(',').filter(|p| !p.is_empty()).take(32) {
                    let rows: Vec<Value> = h.query(p, since, max).into_iter().map(|(t, v)| serde_json::json!([t, v])).collect();
                    out.insert(p.to_string(), Value::Array(rows));
                }
            }
            json(Value::Object(out))
        }
        p => pb_webserve::serve_asset(ASSETS, p),
    }
}

pub fn run(mut ctx: LinkCtx) {
    let host = ctx.conn.s("host", "0.0.0.0");
    let port = ctx.conn.n("port", 8780.0) as u16;
    let title = ctx.conn.s("title", "Dashboard");
    let every = ctx.output_interval(Duration::from_millis(250));
    let dir = Some(ctx.conn.s("data_dir", "")).filter(|d| !d.is_empty()).map(PathBuf::from);
    let shared = Arc::new(Shared {
        title: title.clone(),
        allow_edit: !matches!(ctx.conn.get("allow_edit"), Some(Value::Bool(false))),
        store: Mutex::new(Store::new(dir)),
        history: Mutex::new(History::new(ctx.conn.n("history_minutes", 60.0) as u32, 1000)),
    });
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
        ctx.up(format!("serving http://{host}:{port}/"));
        let mut peers: Vec<WebSocket<TcpStream>> = Vec::new();
        let (joined_tx, joined_rx) = mpsc::channel::<WebSocket<TcpStream>>();
        let mut pending: Vec<String> = Vec::new();
        let mut next = Instant::now();
        while !ctx.stopped() {
            while let Ok((s, _)) = listener.accept() {
                let tx = joined_tx.clone();
                let shared = shared.clone();
                // Classification and the WS handshake happen off this thread so a slow client
                // can't stall the live feed.
                std::thread::spawn(move || {
                    let _ = s.set_nonblocking(false);
                    match pb_webserve::classify(s, Duration::from_secs(3)) {
                        Incoming::WebSocket(s) => {
                            if let Ok(ws) = tungstenite::accept(s) {
                                let _ = tx.send(ws);
                            }
                        }
                        Incoming::Http(req, mut s) => {
                            let _ = pb_webserve::respond(&mut s, route(&req, &shared));
                        }
                        Incoming::Dropped => {}
                    }
                });
            }
            while let Ok(mut ws) = joined_rx.try_recv() {
                let _ = ws.get_ref().set_read_timeout(Some(Duration::from_millis(5)));
                let snap = serde_json::to_string(&snapshot(&ctx, &title)).unwrap_or_default();
                if ws.send(Message::text(snap)).is_ok() {
                    peers.push(ws);
                }
            }
            // Drop closed peers; browsers send nothing yet (control writes come in phase 5).
            peers.retain_mut(|ws| loop {
                match ws.read() {
                    Ok(Message::Close(_)) => return false,
                    Ok(_) => {}
                    Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => return true,
                    Err(_) => return false,
                }
            });
            let n = peers.len() as u64;
            ctx.status(|s| {
                s.peers = Some(n);
                s.state = format!("serving http://{host}:{port}/ — {n} viewer{}", if n == 1 { "" } else { "s" });
            });
            let got = ctx.wait_writes(Duration::from_millis(20));
            if !got.is_empty() {
                let mut points = BTreeMap::new();
                for (p, v) in &got {
                    if ctx.outputs_for(p).next().is_some() {
                        if let Some(s) = point_state(&ctx, p, v) {
                            if let (Some(x), Ok(mut h)) = (Val::parse_text(v).as_f64(), shared.history.lock()) {
                                h.record(p, s.ts, x);
                            }
                            points.insert(p.clone(), s);
                        }
                    }
                }
                if !points.is_empty() {
                    // Latest value per point wins within a batch (BTreeMap insert order = arrival).
                    pending.push(serde_json::to_string(&ServerMsg::Chg { points }).unwrap_or_default());
                }
            }
            if !pending.is_empty() && Instant::now() >= next {
                next = Instant::now() + every;
                let msgs = std::mem::take(&mut pending);
                peers.retain_mut(|ws| msgs.iter().all(|m| ws.send(Message::text(m.clone())).is_ok()));
                ctx.status(|s| s.tx += msgs.len() as u64);
            }
        }
        for mut ws in peers {
            let _ = ws.close(None);
        }
    }
}

pb_extkit::ext_pack_impl!(PackState, b"dashboard\0", b"Dashboard\0", "dashboard", run, endpoint, check);
pb_abi::export_pack!(PackState);

#[cfg(test)]
mod tests {
    use super::*;
    use pb_extkit::{testhost, ExtPack};
    use std::io::{Read, Write};

    fn http(port: u16, method: &str, path: &str, body: &str) -> (u16, String) {
        let mut c = TcpStream::connect(("127.0.0.1", port)).unwrap();
        c.write_all(format!("{method} {path} HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n{body}", body.len()).as_bytes()).unwrap();
        let mut out = String::new();
        let _ = c.read_to_string(&mut out);
        let status = out.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
        (status, out.split("\r\n\r\n").nth(1).unwrap_or("").to_string())
    }

    fn http_get(port: u16, path: &str) -> Vec<u8> {
        let mut c = TcpStream::connect(("127.0.0.1", port)).unwrap();
        c.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes()).unwrap();
        let mut out = Vec::new();
        c.read_to_end(&mut out).unwrap();
        out
    }

    fn started_with(extra: Value, outputs: &str) -> (u16, ExtPack) {
        let (_log, host) = testhost::host();
        let port = testhost::free_port();
        let mut conn = serde_json::json!({"id":"d","host":"127.0.0.1","port":port,"title":"Pumps"});
        for (k, v) in extra.as_object().unwrap() {
            conn[k] = v.clone();
        }
        let mut p = ExtPack::new(host, "dashboard", run, endpoint, check);
        p.configure(
            &serde_json::json!({
                "connections":[conn],
                "outputs":[{"connection":"d","interval_secs":0.05,"binds":serde_json::from_str::<Value>(outputs).unwrap()}]
            })
            .to_string(),
        )
        .unwrap();
        let end = Instant::now() + Duration::from_secs(5);
        while TcpStream::connect(("127.0.0.1", port)).is_err() {
            assert!(Instant::now() < end, "server never listened");
            std::thread::sleep(Duration::from_millis(20));
        }
        (port, p)
    }

    fn started(outputs: &str) -> (u16, ExtPack) {
        started_with(serde_json::json!({}), outputs)
    }

    #[test]
    fn serves_the_app_and_health() {
        let (port, _p) = started("[]");
        let index = String::from_utf8_lossy(&http_get(port, "/")).into_owned();
        assert!(index.starts_with("HTTP/1.1 200") && index.contains("pb-dash-web"), "{}", &index[..index.len().min(300)]);
        let wasm = http_get(port, "/pb-dash-web_bg.wasm");
        assert!(wasm.starts_with(b"HTTP/1.1 200") && wasm.windows(5).any(|w| w == b"\0asm\x01"));
        assert!(String::from_utf8_lossy(&http_get(port, "/api/health")).contains(r#""title":"Pumps""#));
        assert!(String::from_utf8_lossy(&http_get(port, "/nope")).starts_with("HTTP/1.1 404"));
        assert_eq!(http(port, "DELETE", "/", "").0, 405);
    }

    #[test]
    fn layouts_are_saved_listed_loaded_and_deleted() {
        let dir = std::env::temp_dir().join(format!("pb-dash-pack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (port, _p) = started_with(serde_json::json!({"data_dir": dir.to_string_lossy()}), "[]");
        assert_eq!(http(port, "GET", "/api/dashboards", "").1, r#"{"can_edit":true,"names":[],"title":"Pumps"}"#);
        assert_eq!(http(port, "GET", "/api/dashboards/Main", "").0, 404);
        let layout = r#"{"v":1,"tiles":[{"id":1,"x":0,"y":0,"w":4,"h":3,"widget":"numeric","point":"psi"}]}"#;
        assert_eq!(http(port, "PUT", "/api/dashboards/Main", layout).0, 200);
        assert_eq!(http(port, "PUT", "/api/dashboards/Pump%20room", layout).0, 200, "percent-encoded names");
        assert!(dir.join("Pump room.json").is_file());
        let listed: Value = serde_json::from_str(&http(port, "GET", "/api/dashboards", "").1).unwrap();
        assert_eq!(listed["names"], serde_json::json!(["Main", "Pump room"]));
        let got: Value = serde_json::from_str(&http(port, "GET", "/api/dashboards/Main", "").1).unwrap();
        assert_eq!(got["tiles"][0]["point"], "psi");
        // Bad input is refused and never stored.
        assert_eq!(http(port, "PUT", "/api/dashboards/Main", "junk").0, 400);
        assert_eq!(http(port, "PUT", "/api/dashboards/..%2Fevil", layout).0, 400);
        assert_eq!(http(port, "PUT", "/api/dashboards/Bad", r#"{"v":1,"tiles":[{"id":1,"x":23,"y":0,"w":4,"h":3,"widget":"numeric"}]}"#).0, 400);
        assert_eq!(http(port, "DELETE", "/api/dashboards/Main", "").0, 200);
        assert_eq!(http(port, "DELETE", "/api/dashboards/Main", "").0, 404);
        assert_eq!(http(port, "GET", "/api/dashboards", "").0, 200);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_only_dashboards_refuse_edits() {
        let (port, _p) = started_with(serde_json::json!({"allow_edit": false}), "[]");
        assert!(http(port, "GET", "/api/dashboards", "").1.contains(r#""can_edit":false"#));
        assert_eq!(http(port, "PUT", "/api/dashboards/Main", r#"{"v":1}"#).0, 403);
        assert_eq!(http(port, "DELETE", "/api/dashboards/Main", "").0, 403);
    }

    #[test]
    fn history_serves_numeric_points_for_charts() {
        let (port, mut p) = started(r#"[{"point":"psi"},{"point":"mode"}]"#);
        for i in 0..5 {
            p.write_point("psi", &(3000 + i * 10).to_string());
            p.write_point("mode", "auto");
            std::thread::sleep(Duration::from_millis(1100)); // history keeps one sample per second
        }
        let h: Value = serde_json::from_str(&http(port, "GET", "/api/history?points=psi,mode,nope&max=100", "").1).unwrap();
        let psi = h["psi"].as_array().unwrap();
        assert!(psi.len() >= 4, "{psi:?}");
        assert_eq!(psi.last().unwrap()[1], 3040.0);
        assert!(psi.windows(2).all(|w| w[0][0].as_u64() <= w[1][0].as_u64()));
        assert!(h["mode"].as_array().unwrap().is_empty(), "text points have no history");
        assert!(h["nope"].as_array().unwrap().is_empty());
        let since = psi[2][0].as_u64().unwrap();
        let h2: Value = serde_json::from_str(&http(port, "GET", &format!("/api/history?points=psi&since={since}"), "").1).unwrap();
        assert_eq!(h2["psi"].as_array().unwrap().len(), psi.len() - 2);
    }

    #[test]
    fn websocket_gets_snapshot_then_changes_for_bound_points_only() {
        let (port, mut p) = started(r#"[{"point":"psi","units":"psi"},{"point":"rate"}]"#);
        p.write_point("psi", "3400");
        p.write_point("secret", "1"); // not an Output bind: must never reach a browser
        std::thread::sleep(Duration::from_millis(150));
        let (mut ws, _) = tungstenite::connect(format!("ws://127.0.0.1:{port}/ws")).unwrap();
        let snap: ServerMsg = serde_json::from_str(&ws.read().unwrap().into_text().unwrap()).unwrap();
        let ServerMsg::Snap { title, points } = snap else { panic!("expected snap") };
        assert_eq!(title, "Pumps");
        assert_eq!(points.keys().map(String::as_str).collect::<Vec<_>>(), vec!["psi"]);
        assert_eq!((points["psi"].v.clone(), points["psi"].u.as_str()), (serde_json::json!(3400), "psi"));
        p.write_point("rate", "8.5");
        p.write_point("secret", "2");
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            let ServerMsg::Chg { points } = serde_json::from_str(&ws.read().unwrap().into_text().unwrap()).unwrap() else { panic!("expected chg") };
            assert!(!points.contains_key("secret"));
            if points.get("rate").map(|s| s.v.clone()) == Some(serde_json::json!(8.5)) {
                break;
            }
            assert!(Instant::now() < end);
        }
        let d: Value = serde_json::from_str(&p.detail_json()).unwrap();
        assert_eq!(d["connections"][0]["peers"], 1);
    }

    #[test]
    fn port_is_checked() {
        assert!(check(serde_json::json!({"port": 0}).as_object().unwrap()).is_err());
        assert!(check(serde_json::json!({"port": 8780}).as_object().unwrap()).is_ok());
    }
}
