//! MCP server on the service (PRD settled #14): an agent can inspect and set up Protocol Bench.
//!
//! Transport: MCP "Streamable HTTP" in its simplest form — `POST /mcp` with one JSON-RPC 2.0
//! message, answered with `application/json` (no SSE stream needed). Bound to 127.0.0.1 only,
//! and every request must carry `Authorization: Bearer <token>` with the same token as the UI
//! IPC (`ipc.token` in the config dir). The URL is written to `mcp.json` next to `ipc.json`.
//!
//! ```text
//! PB_MCP_PORT=7741 pb-service          # fixed port (default: any free port, see mcp.json)
//! PB_MCP=off pb-service                # do not start it
//! ```
//!
//! Tools: status, protocols, points (list / read / write), hubs, project (get / set a section /
//! add a connection), open project, stop all hubs. Edits are applied and saved straight away;
//! an open UI picks them up when the project is reopened.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use pb_engine::{EngineCommand, EngineId};
use serde_json::{json, Value};

use crate::runtime::Runtime;

const PROTOCOL_VERSION: &str = "2025-06-18";

/// Start the MCP listener on its own thread (unless `PB_MCP=off`).
pub fn spawn(runtime: Arc<Mutex<Runtime>>) {
    if std::env::var("PB_MCP").is_ok_and(|v| matches!(v.trim(), "off" | "0" | "false" | "no")) {
        return;
    }
    let port: u16 = std::env::var("PB_MCP_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(0);
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("pb-service: MCP not started (127.0.0.1:{port}: {e})");
            return;
        }
    };
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(port);
    let url = format!("http://127.0.0.1:{port}/mcp");
    if let Some(dir) = pb_ipc::config_dir() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("mcp.json"), json!({ "url": url, "auth": "Bearer <ipc.token>" }).to_string());
    }
    eprintln!("pb-service: MCP listening on {url}");
    std::thread::Builder::new()
        .name("pb-mcp".into())
        .spawn(move || {
            for s in listener.incoming().flatten() {
                let rt = Arc::clone(&runtime);
                std::thread::spawn(move || {
                    let _ = serve_http(s, rt);
                });
            }
        })
        .ok();
}

/// A parsed HTTP request (just what MCP needs).
#[derive(Debug, Default, PartialEq)]
pub struct HttpReq {
    pub method: String,
    pub path: String,
    pub auth: String,
    pub body: Vec<u8>,
}

pub fn read_http(r: &mut impl BufRead) -> Result<HttpReq, String> {
    let mut line = String::new();
    r.read_line(&mut line).map_err(|e| e.to_string())?;
    let mut parts = line.split_whitespace();
    let mut req = HttpReq { method: parts.next().unwrap_or("").to_string(), path: parts.next().unwrap_or("").to_string(), ..Default::default() };
    let mut len = 0usize;
    loop {
        line.clear();
        if r.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
            break;
        }
        let t = line.trim_end();
        if t.is_empty() {
            break;
        }
        if let Some((k, v)) = t.split_once(':') {
            match k.trim().to_ascii_lowercase().as_str() {
                "content-length" => len = v.trim().parse().map_err(|_| "bad content-length")?,
                "authorization" => req.auth = v.trim().to_string(),
                _ => {}
            }
        }
    }
    if len > 4 * 1024 * 1024 {
        return Err("body too large".into());
    }
    req.body = vec![0; len];
    r.read_exact(&mut req.body).map_err(|e| e.to_string())?;
    Ok(req)
}

fn respond(w: &mut impl Write, status: &str, body: &str) -> std::io::Result<()> {
    write!(
        w,
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn serve_http(s: TcpStream, rt: Arc<Mutex<Runtime>>) -> Result<(), String> {
    s.set_read_timeout(Some(std::time::Duration::from_secs(30))).ok();
    let mut w = s.try_clone().map_err(|e| e.to_string())?;
    let mut r = BufReader::new(s);
    let req = read_http(&mut r)?;
    let token = pb_ipc::read_token().unwrap_or_default();
    let (status, body) = route(&req, &token, &rt);
    respond(&mut w, status, &body).map_err(|e| e.to_string())
}

/// HTTP routing + auth. Returns (status line, body).
pub fn route(req: &HttpReq, token: &str, rt: &Arc<Mutex<Runtime>>) -> (&'static str, String) {
    if req.path.split('?').next() != Some("/mcp") {
        return ("404 Not Found", json!({"error": "use POST /mcp"}).to_string());
    }
    if token.is_empty() || req.auth != format!("Bearer {token}") {
        return ("401 Unauthorized", json!({"error": "Authorization: Bearer <ipc.token> required"}).to_string());
    }
    if req.method != "POST" {
        return ("405 Method Not Allowed", json!({"error": "POST JSON-RPC messages to /mcp"}).to_string());
    }
    let msg: Value = match serde_json::from_slice(&req.body) {
        Ok(v) => v,
        Err(e) => return ("400 Bad Request", json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":e.to_string()}}).to_string()),
    };
    match handle_rpc(&msg, rt) {
        Some(resp) => ("200 OK", resp.to_string()),
        None => ("202 Accepted", String::new()),
    }
}

/// One JSON-RPC message → response (`None` for notifications).
pub fn handle_rpc(msg: &Value, rt: &Arc<Mutex<Runtime>>) -> Option<Value> {
    let id = msg.get("id").cloned();
    let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let id = id?; // notifications (no id) get no reply
    let params = msg.get("params").cloned().unwrap_or(json!({}));
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": params.get("protocolVersion").and_then(|v| v.as_str()).unwrap_or(PROTOCOL_VERSION),
            "capabilities": {"tools": {"listChanged": false}},
            "serverInfo": {"name": "protocol-bench", "title": "Protocol Bench service", "version": env!("CARGO_PKG_VERSION")},
            "instructions": "Protocol Bench moves values between industrial protocols. Points are the shared values; \
                hubs route Inputs to Outputs; each protocol section of the project holds its connections. \
                Read with get_status / list_points / list_hubs / get_project, change with set_project_section, \
                add_connection or write_point."
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools() })),
        "tools/call" => {
            let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            Ok(match call_tool(name, &args, rt) {
                Ok(v) => json!({"content": [{"type": "text", "text": serde_json::to_string_pretty(&v).unwrap_or_default()}], "structuredContent": v, "isError": false}),
                Err(e) => json!({"content": [{"type": "text", "text": e}], "isError": true}),
            })
        }
        other => Err((-32601, format!("method not found: {other}"))),
    };
    Some(match result {
        Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
        Err((code, m)) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": m}}),
    })
}

fn tool(name: &str, desc: &str, props: Value, required: &[&str]) -> Value {
    json!({"name": name, "description": desc, "inputSchema": {"type": "object", "properties": props, "required": required}})
}

pub fn tools() -> Vec<Value> {
    vec![
        tool("get_status", "Service status: project, loaded protocol packs, point count, running hubs, last error.", json!({}), &[]),
        tool("list_protocols", "Every protocol slot: pack loaded, state, connected, last error.", json!({}), &[]),
        tool(
            "list_points",
            "Host points (id, value, quality, source address, updated). Optional case-insensitive filter on id or source.",
            json!({"filter": {"type": "string"}, "limit": {"type": "integer", "minimum": 1}}),
            &[],
        ),
        tool("read_point", "One point by id.", json!({"name": {"type": "string"}}), &["name"]),
        tool(
            "write_point",
            "Send a value for a point to every protocol Output bound to it (e.g. a Modbus register, a DNP3 control, a WITS item).",
            json!({"name": {"type": "string"}, "value": {"type": "string"}}),
            &["name", "value"],
        ),
        tool("list_hubs", "Hubs with their protocol, Input and Output nodes (protocol, label, connection, bind count).", json!({}), &[]),
        tool(
            "get_project",
            "The project JSON, or one top-level section (mqtt, modbus, opcua, streams, http, files, las, snmp, wits, websocket, sparkplug, dnp3, can, hubs, points, settings).",
            json!({"section": {"type": "string"}}),
            &[],
        ),
        tool(
            "set_project_section",
            "Replace one top-level project section with `value`, apply it to the running packs and save the project.",
            json!({"section": {"type": "string"}, "value": {}}),
            &["section", "value"],
        ),
        tool(
            "add_connection",
            "Add (or replace by id) a connection object in a protocol section's `connections` list (modbus, wits, websocket, sparkplug, dnp3, can), apply and save.",
            json!({"protocol": {"type": "string"}, "connection": {"type": "object"}}),
            &["protocol", "connection"],
        ),
        tool("open_project", "Stop what is running and open another project file.", json!({"path": {"type": "string"}, "auto_run_hubs": {"type": "boolean"}}), &["path"]),
        tool("stop_all_hubs", "Stop embedded brokers / servers and disconnect every client.", json!({}), &[]),
    ]
}

fn str_arg<'a>(args: &'a Value, k: &str) -> Result<&'a str, String> {
    args.get(k).and_then(|v| v.as_str()).filter(|s| !s.trim().is_empty()).ok_or_else(|| format!("missing \"{k}\""))
}

pub fn call_tool(name: &str, args: &Value, rt: &Arc<Mutex<Runtime>>) -> Result<Value, String> {
    let mut rt = rt.lock().map_err(|_| "runtime lock poisoned".to_string())?;
    match name {
        "get_status" => {
            let mut v = serde_json::to_value(rt.status()).map_err(|e| e.to_string())?;
            v["project_path"] = json!(rt.project_path.display().to_string());
            v["project_name"] = rt.project_json.pointer("/settings/name").cloned().unwrap_or(json!(""));
            Ok(v)
        }
        "list_protocols" => {
            let snap = rt.snapshot();
            Ok(json!(snap
                .slots
                .iter()
                .map(|s| json!({"id": s.id, "title": s.title, "pack_loaded": s.pack_loaded, "state": s.state, "connected": s.connected, "last_error": s.last_error}))
                .collect::<Vec<_>>()))
        }
        "list_points" => {
            let filter = args.get("filter").and_then(|v| v.as_str()).unwrap_or("").to_ascii_lowercase();
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(500) as usize;
            let snap = rt.snapshot();
            let rows: Vec<Value> = snap
                .points
                .iter()
                .filter(|p| filter.is_empty() || p.name.to_ascii_lowercase().contains(&filter) || p.source.to_ascii_lowercase().contains(&filter))
                .take(limit)
                .map(|p| json!({"id": p.name, "value": p.value, "quality": p.quality, "source": p.source, "updated_ms": p.updated_ms}))
                .collect();
            Ok(json!({"count": rows.len(), "points": rows}))
        }
        "read_point" => {
            let n = str_arg(args, "name")?;
            let snap = rt.snapshot();
            snap.points
                .iter()
                .find(|p| p.name == n)
                .map(|p| json!({"id": p.name, "value": p.value, "quality": p.quality, "source": p.source, "updated_ms": p.updated_ms}))
                .ok_or_else(|| format!("no point \"{n}\""))
        }
        "write_point" => {
            let n = str_arg(args, "name")?.to_string();
            let value = match args.get("value") {
                Some(Value::String(s)) => s.clone(),
                Some(v) if !v.is_null() => v.to_string(),
                _ => return Err("missing \"value\"".into()),
            };
            let mut sent = Vec::new();
            for id in EngineId::ALL.into_iter().filter(|e| e.takes_egress()) {
                rt.engine.submit(id, EngineCommand::WritePoint { name: n.clone(), value: value.clone() });
                sent.push(id);
            }
            Ok(json!({"point": n, "value": value, "sent_to": sent}))
        }
        "list_hubs" => {
            let hubs = rt.project_json.pointer("/hubs/hubs").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            let summary = |ports: Option<&Value>| -> Vec<Value> {
                ports
                    .and_then(|v| v.as_array())
                    .into_iter()
                    .flatten()
                    .map(|p| {
                        let s = |k: &str| p.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let conn = ["ext_connection", "modbus_connection", "mqtt_connection", "opcua_connection", "http_connection", "stream_endpoint", "las_endpoint", "snmp_target", "file_source"]
                            .iter()
                            .map(|k| s(k))
                            .find(|v| !v.is_empty())
                            .unwrap_or_default();
                        let binds: usize = p
                            .as_object()
                            .map(|o| o.iter().filter(|(k, _)| k.ends_with("_inputs") || k.ends_with("_outputs")).filter_map(|(_, v)| v.as_array()).map(|a| a.len()).sum())
                            .unwrap_or(0);
                        json!({"id": p.get("id"), "protocol": s("protocol"), "label": s("label"), "connection": conn, "binds": binds})
                    })
                    .collect()
            };
            Ok(json!(hubs
                .iter()
                .map(|h| json!({"id": h.get("id"), "name": h.get("name"), "protocol": h.get("protocol"), "inputs": summary(h.get("sources")), "outputs": summary(h.get("outputs"))}))
                .collect::<Vec<_>>()))
        }
        "get_project" => match args.get("section").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
            Some(sec) => rt.project_json.get(sec).cloned().ok_or_else(|| format!("no section \"{sec}\"")),
            None => Ok(rt.project_json.clone()),
        },
        "set_project_section" => {
            let sec = str_arg(args, "section")?.to_string();
            if matches!(sec.as_str(), "version" | "layout") {
                return Err(format!("\"{sec}\" is managed by the UI"));
            }
            let value = args.get("value").cloned().ok_or("missing \"value\"")?;
            let mut project = rt.project_json.clone();
            if !project.is_object() {
                project = json!({});
            }
            project[sec.as_str()] = value;
            let auto = rt.auto_run_hubs;
            rt.put_project(project, auto, None)?;
            Ok(json!({"ok": true, "section": sec}))
        }
        "add_connection" => {
            let proto = str_arg(args, "protocol")?.to_ascii_lowercase();
            if !matches!(proto.as_str(), "modbus" | "wits" | "websocket" | "sparkplug" | "dnp3" | "can") {
                return Err(format!("add_connection supports modbus, wits, websocket, sparkplug, dnp3, can (got \"{proto}\"); use set_project_section for others"));
            }
            let conn = args.get("connection").filter(|c| c.is_object()).cloned().ok_or("\"connection\" must be an object")?;
            let cid = conn.get("id").and_then(|v| v.as_str()).filter(|s| !s.trim().is_empty()).ok_or("connection needs an \"id\"")?.to_string();
            let mut project = rt.project_json.clone();
            let section = project.get(&proto).cloned().unwrap_or_else(|| json!({}));
            let mut list = section.get("connections").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            let replaced = if let Some(slot) = list.iter_mut().find(|c| c.get("id").and_then(|v| v.as_str()) == Some(cid.as_str())) {
                *slot = conn;
                true
            } else {
                list.push(conn);
                false
            };
            let mut section = section;
            if !section.is_object() {
                section = json!({});
            }
            section["connections"] = json!(list);
            project[proto.as_str()] = section;
            let auto = rt.auto_run_hubs;
            rt.put_project(project, auto, None)?;
            Ok(json!({"ok": true, "protocol": proto, "id": cid, "replaced": replaced}))
        }
        "open_project" => {
            let path = PathBuf::from(str_arg(args, "path")?);
            let auto = args.get("auto_run_hubs").and_then(|v| v.as_bool()).unwrap_or(rt.auto_run_hubs);
            rt.stop_all_hubs();
            rt.load_project_file(&path, auto)?;
            Ok(json!({"ok": true, "path": path.display().to_string()}))
        }
        "stop_all_hubs" => {
            rt.stop_all_hubs();
            Ok(json!({"ok": true}))
        }
        other => Err(format!("unknown tool \"{other}\"")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime() -> (Arc<Mutex<Runtime>>, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("pb-mcp-test-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("project.json");
        std::fs::write(
            &path,
            json!({"version": 11, "settings": {"name": "Test"}, "modbus": {"connections": []},
                   "hubs": {"hubs": [{"id": 1, "name": "Plant", "protocol": "Modbus",
                     "sources": [{"id": 2, "protocol": "Dnp3", "label": "RTU", "ext_connection": "rtu", "ext_inputs": [{"kind":"analog","index":0,"point":""}]}],
                     "outputs": []}]}})
            .to_string(),
        )
        .unwrap();
        (Arc::new(Mutex::new(Runtime::start(None, path.clone()))), path)
    }

    fn call(rt: &Arc<Mutex<Runtime>>, id: i64, method: &str, params: Value) -> Value {
        handle_rpc(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}), rt).unwrap()
    }

    #[test]
    fn handshake_tools_and_calls() {
        let (rt, path) = runtime();
        let init = call(&rt, 1, "initialize", json!({"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"t","version":"1"}}));
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert!(handle_rpc(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}), &rt).is_none());
        let list = call(&rt, 2, "tools/list", json!({}));
        let names: Vec<&str> = list["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"list_points") && names.contains(&"add_connection"));
        // Hubs come from the project.
        let hubs = call(&rt, 3, "tools/call", json!({"name":"list_hubs","arguments":{}}));
        let hs = &hubs["result"]["structuredContent"];
        assert_eq!(hs[0]["inputs"][0]["protocol"], "Dnp3");
        assert_eq!(hs[0]["inputs"][0]["binds"], 1);
        // Point materialised from the hub bind is listed.
        let pts = call(&rt, 4, "tools/call", json!({"name":"list_points","arguments":{"filter":"dnp3"}}));
        assert_eq!(pts["result"]["structuredContent"]["points"][0]["source"], "dnp3/rtu::analog:0");
        // Add a connection: applied and saved to disk.
        let add = call(&rt, 5, "tools/call", json!({"name":"add_connection","arguments":{"protocol":"dnp3","connection":{"id":"rtu","host":"10.0.0.5","port":20000}}}));
        assert_eq!(add["result"]["isError"], false, "{add}");
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved["dnp3"]["connections"][0]["host"], "10.0.0.5");
        let again = call(&rt, 6, "tools/call", json!({"name":"add_connection","arguments":{"protocol":"dnp3","connection":{"id":"rtu","host":"10.0.0.6"}}}));
        assert_eq!(again["result"]["structuredContent"]["replaced"], true);
        // Section get / set.
        call(&rt, 7, "tools/call", json!({"name":"set_project_section","arguments":{"section":"settings","value":{"name":"Renamed"}}}));
        let st = call(&rt, 8, "tools/call", json!({"name":"get_project","arguments":{"section":"settings"}}));
        assert_eq!(st["result"]["structuredContent"]["name"], "Renamed");
        // Errors are tool errors, unknown methods are JSON-RPC errors.
        let bad = call(&rt, 9, "tools/call", json!({"name":"read_point","arguments":{"name":"nope"}}));
        assert_eq!(bad["result"]["isError"], true);
        let refused = call(&rt, 10, "tools/call", json!({"name":"set_project_section","arguments":{"section":"layout","value":{}}}));
        assert_eq!(refused["result"]["isError"], true);
        assert_eq!(call(&rt, 11, "nope", json!({}))["error"]["code"], -32601);
        let w = call(&rt, 12, "tools/call", json!({"name":"write_point","arguments":{"name":"sp","value":12.5}}));
        assert_eq!(w["result"]["structuredContent"]["value"], "12.5");
    }

    #[test]
    fn http_parsing_and_auth() {
        let raw = b"POST /mcp HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer abc\r\nContent-Length: 2\r\n\r\n{}";
        let req = read_http(&mut std::io::Cursor::new(&raw[..])).unwrap();
        assert_eq!((req.method.as_str(), req.path.as_str(), req.auth.as_str(), req.body.as_slice()), ("POST", "/mcp", "Bearer abc", &b"{}"[..]));
        let (rt, _) = runtime();
        assert_eq!(route(&req, "other", &rt).0, "401 Unauthorized");
        assert_eq!(route(&req, "", &rt).0, "401 Unauthorized");
        let ping = HttpReq { body: br#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#.to_vec(), ..req };
        assert_eq!(route(&ping, "abc", &rt).0, "200 OK");
        let note = HttpReq { body: br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#.to_vec(), method: "POST".into(), path: "/mcp".into(), auth: "Bearer abc".into() };
        assert_eq!(route(&note, "abc", &rt).0, "202 Accepted");
        let get = HttpReq { method: "GET".into(), path: "/mcp".into(), auth: "Bearer abc".into(), body: vec![] };
        assert_eq!(route(&get, "abc", &rt).0, "405 Method Not Allowed");
        let other = HttpReq { path: "/x".into(), ..get };
        assert_eq!(route(&other, "abc", &rt).0, "404 Not Found");
    }
}
