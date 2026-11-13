//! MQTT client protocol pack (cdylib) for Protocol Bench.
//!
//! Multi-client: one pack handle owns many broker connections. Worker + EventLoop
//! pattern from TopicBench `mqtt/live.rs` (no UI/tree). Broker hub role deferred.

mod json_path;
mod session;

pub use json_path::{
    extract_path_value, migrate_point_id, point_id_for_client_field, point_id_for_client_topic,
    split_point_id, topic_matches,
};

use pb_abi::{
    write_cstr, PbHostApi, PbPackApi, PB_ABI_VERSION, PB_LOG_ERROR, PB_LOG_INFO, PB_QUALITY_BAD,
    PB_QUALITY_GOOD, PB_VALUE_BYTES, PB_VALUE_STRING,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use session::{MqttSession, SessionEvent, TransportKind};
use std::collections::BTreeMap;
use std::ffi::CStr;
use std::os::raw::{c_char, c_int, c_void};
use std::sync::mpsc::TryRecvError;

const PACK_ID: &[u8] = b"mqtt\0";
const PACK_TITLE: &[u8] = b"MQTT\0";

/// One JSON field → point rule (topic filter + dotted path).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ParseRule {
    #[serde(default)]
    pub topic: String,
    pub path: String,
    #[serde(default)]
    pub point: String,
}

/// One broker connection inside the MQTT pack.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClientConfig {
    /// Stable connection id (UI / project / point namespace).
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_client_id")]
    pub client_id: String,
    /// Fresh random MQTT client id on each connect (avoids broker collisions).
    #[serde(default)]
    pub unique_client_id: bool,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default = "default_transport")]
    pub transport: String,
    #[serde(default = "default_keep_alive")]
    pub keep_alive_secs: u64,
    #[serde(default)]
    pub subscriptions: Vec<String>,
    #[serde(default)]
    pub parse_rules: Vec<ParseRule>,
    /// Point → publish topic bindings for this client (MQTT outputs only).
    #[serde(default)]
    pub outputs: Vec<OutputBinding>,
    /// Desired session state after reconcile.
    #[serde(default)]
    pub auto_connect: bool,
}

/// Bind a host point to an MQTT publish topic on this client.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputBinding {
    pub point: String,
    pub topic: String,
    #[serde(default)]
    pub qos: u8,
    #[serde(default)]
    pub retain: bool,
}

fn default_host() -> String {
    "127.0.0.1".into()
}
fn default_port() -> u16 {
    1883
}
fn default_client_id() -> String {
    "protocol-bench".into()
}
fn default_transport() -> String {
    "tcp".into()
}
fn default_keep_alive() -> u64 {
    30
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            id: "default".into(),
            label: "MQTT".into(),
            host: default_host(),
            port: default_port(),
            client_id: default_client_id(),
            unique_client_id: false,
            username: String::new(),
            password: String::new(),
            transport: default_transport(),
            keep_alive_secs: default_keep_alive(),
            subscriptions: vec!["pb/#".into()],
            parse_rules: Vec::new(),
            outputs: Vec::new(),
            auto_connect: false,
        }
    }
}

fn connect_client_id(cfg: &ClientConfig) -> String {
    if cfg.unique_client_id {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::time::{SystemTime, UNIX_EPOCH};
        static SEQ: AtomicU64 = AtomicU64::new(1);
        let t = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let mix = t ^ n.wrapping_mul(0x9e37_79b9_7f4a_7c15);
        format!("pb-{}-{:x}", cfg.id, mix & 0xffff_ffff)
    } else if cfg.client_id.trim().is_empty() {
        format!("pb-{}", cfg.id)
    } else {
        cfg.client_id.clone()
    }
}

impl ClientConfig {
    fn transport_kind(&self) -> Result<TransportKind, String> {
        match self.transport.to_ascii_lowercase().as_str() {
            "tcp" | "mqtt" => Ok(TransportKind::Tcp),
            "tls" | "mqtts" => Ok(TransportKind::Tls),
            "ws" | "websocket" => Ok(TransportKind::Ws),
            "wss" => Ok(TransportKind::Wss),
            other => Err(format!("unknown transport '{other}'")),
        }
    }
}

/// Last payload + count for one observed topic (topic-tree preview; not a point).
#[derive(Debug, Clone)]
struct TrafficLeaf {
    payload: String,
    count: u64,
}

const TRAFFIC_TOPIC_CAP: usize = 2048;

struct ClientRuntime {
    config: ClientConfig,
    session: Option<MqttSession>,
    connected: bool,
    state: String,
    last_error: String,
    quality_bad: bool,
    /// Subscribed traffic for UI topic tree (independent of point promotion).
    traffic: BTreeMap<String, TrafficLeaf>,
}

impl ClientRuntime {
    fn new(config: ClientConfig) -> Self {
        Self {
            config,
            session: None,
            connected: false,
            state: "disconnected".into(),
            last_error: String::new(),
            quality_bad: false,
            traffic: BTreeMap::new(),
        }
    }

    fn observe_traffic(&mut self, topic: &str, payload: &str) {
        if topic.is_empty() {
            return;
        }
        if let Some(leaf) = self.traffic.get_mut(topic) {
            leaf.payload = payload.to_string();
            leaf.count = leaf.count.saturating_add(1);
            return;
        }
        if self.traffic.len() >= TRAFFIC_TOPIC_CAP {
            // Drop an arbitrary oldest-key entry to stay bounded.
            if let Some(key) = self.traffic.keys().next().cloned() {
                self.traffic.remove(&key);
            }
        }
        self.traffic.insert(
            topic.to_string(),
            TrafficLeaf {
                payload: payload.to_string(),
                count: 1,
            },
        );
    }
}

struct PackState {
    host: PbHostApi,
    clients: BTreeMap<String, ClientRuntime>,
}

impl PackState {
    fn new(host: PbHostApi) -> Self {
        Self {
            host,
            clients: BTreeMap::new(),
        }
    }

    fn log(&self, level: i32, msg: &str) {
        if let Some(log) = self.host.log_message {
            if let Ok(c) = std::ffi::CString::new(msg) {
                unsafe { log(self.host.ctx, level, c.as_ptr()) };
            }
        }
    }

    /// Upsert a host point. `name` is a short id or long address; `source` is
    /// protocol (`mqtt`) when `name` is an address, or the address when `name` is short.
    fn upsert_string(&self, name: &str, source: &str, value: &str, good: bool) {
        let Some(upsert) = self.host.upsert_point else {
            return;
        };
        let Ok(c_name) = std::ffi::CString::new(name) else {
            return;
        };
        let Ok(c_src) = std::ffi::CString::new(source) else {
            return;
        };
        let bytes = value.as_bytes();
        unsafe {
            upsert(
                self.host.ctx,
                c_name.as_ptr(),
                PB_VALUE_STRING,
                bytes.as_ptr() as *const c_void,
                bytes.len(),
                if good {
                    PB_QUALITY_GOOD
                } else {
                    PB_QUALITY_BAD
                },
                c_src.as_ptr(),
            );
        }
    }

    fn upsert_bytes(&self, name: &str, source: &str, value: &[u8], good: bool) {
        let Some(upsert) = self.host.upsert_point else {
            return;
        };
        let Ok(c_name) = std::ffi::CString::new(name) else {
            return;
        };
        let Ok(c_src) = std::ffi::CString::new(source) else {
            return;
        };
        unsafe {
            upsert(
                self.host.ctx,
                c_name.as_ptr(),
                PB_VALUE_BYTES,
                value.as_ptr() as *const c_void,
                value.len(),
                if good {
                    PB_QUALITY_GOOD
                } else {
                    PB_QUALITY_BAD
                },
                c_src.as_ptr(),
            );
        }
    }

    fn stop_client(rt: &mut ClientRuntime) {
        if let Some(session) = rt.session.take() {
            session.stop();
        }
        rt.connected = false;
        rt.state = "disconnected".into();
    }

    fn start_client(rt: &mut ClientRuntime) -> Result<(), String> {
        Self::stop_client(rt);
        let transport = rt.config.transport_kind()?;
        let client_id = connect_client_id(&rt.config);
        let session = MqttSession::spawn(
            rt.config.host.clone(),
            rt.config.port,
            client_id,
            rt.config.username.clone(),
            rt.config.password.clone(),
            transport,
            rt.config.keep_alive_secs,
        )?;
        rt.session = Some(session);
        rt.state = "connecting".into();
        rt.last_error.clear();
        rt.connected = false;
        Ok(())
    }

    fn sync_subscriptions(rt: &mut ClientRuntime) -> Result<(), String> {
        let Some(session) = rt.session.as_ref() else {
            return Ok(());
        };
        // MQTT filters may include `+` / `#` (single- / multi-level wildcards).
        for topic in &rt.config.subscriptions {
            let t = topic.trim();
            if t.is_empty() {
                continue;
            }
            session
                .subscribe(t, 0)
                .map_err(|e| format!("subscribe '{t}': {e}"))?;
        }
        Ok(())
    }

    fn handle_client_event(&mut self, id: &str, ev: SessionEvent) {
        let Some(rt) = self.clients.get_mut(id) else {
            return;
        };
        match ev {
            SessionEvent::State { connected, detail } => {
                rt.connected = connected;
                rt.state = if connected {
                    "connected".into()
                } else if detail.to_ascii_lowercase().contains("connecting") {
                    "connecting".into()
                } else {
                    "disconnected".into()
                };
                if connected {
                    rt.quality_bad = false;
                    rt.last_error.clear();
                    if let Err(e) = Self::sync_subscriptions(rt) {
                        rt.last_error = e.clone();
                        rt.quality_bad = true;
                        self.log(PB_LOG_ERROR, &format!("[{id}] {e}"));
                    }
                } else if !detail.is_empty()
                    && !detail.to_ascii_lowercase().contains("connecting")
                    && !detail.to_ascii_lowercase().contains("disconnected by user")
                {
                    rt.last_error = detail.clone();
                    rt.quality_bad = true;
                }
                self.log(PB_LOG_INFO, &format!("[{id}] {detail}"));
            }
            SessionEvent::Message { topic, payload } => {
                // Always record traffic for the UI topic tree; only hub Input
                // parse_rules promote payloads to host points.
                let preview = match std::str::from_utf8(&payload) {
                    Ok(t) => t.to_string(),
                    Err(_) => format!("<{} bytes>", payload.len()),
                };
                rt.observe_traffic(&topic, &preview);
                let rules = rt.config.parse_rules.clone();
                let conn = rt.config.id.clone();
                let bad = rt.quality_bad;
                let text = std::str::from_utf8(&payload).ok().map(|s| s.to_string());
                for rule in rules {
                    if !topic_matches(&rule.topic, &topic) {
                        continue;
                    }
                    let path = rule.path.trim();
                    if path.is_empty() {
                        // Whole-payload bind.
                        let address = point_id_for_client_topic(&conn, &topic);
                        if let Some(text) = text.as_deref() {
                            if rule.point.trim().is_empty() {
                                self.upsert_string(&address, "mqtt", text, !bad);
                            } else {
                                self.upsert_string(rule.point.trim(), &address, text, !bad);
                            }
                        } else if rule.point.trim().is_empty() {
                            self.upsert_bytes(&address, "mqtt", &payload, !bad);
                        } else {
                            self.upsert_bytes(rule.point.trim(), &address, &payload, !bad);
                        }
                        continue;
                    }
                    let Some(text) = text.as_deref() else {
                        continue;
                    };
                    if let Some(value) = extract_path_value(text, path) {
                        let field_addr = point_id_for_client_field(&conn, &topic, path);
                        if rule.point.trim().is_empty() {
                            self.upsert_string(&field_addr, "mqtt", &value, !bad);
                        } else {
                            self.upsert_string(rule.point.trim(), &field_addr, &value, !bad);
                        }
                    }
                }
            }
            SessionEvent::Error { detail } => {
                rt.last_error = detail.clone();
                rt.quality_bad = true;
                rt.connected = false;
                rt.state = "disconnected".into();
                self.log(PB_LOG_ERROR, &format!("[{id}] {detail}"));
            }
        }
    }

    fn drain(&mut self) {
        let ids: Vec<String> = self.clients.keys().cloned().collect();
        for id in ids {
            loop {
                let ev = {
                    let Some(rt) = self.clients.get(&id) else {
                        break;
                    };
                    match rt.session.as_ref() {
                        Some(s) => match s.try_recv() {
                            Ok(ev) => ev,
                            Err(TryRecvError::Empty) => break,
                            Err(TryRecvError::Disconnected) => {
                                if let Some(rt) = self.clients.get_mut(&id) {
                                    rt.session = None;
                                    rt.connected = false;
                                    rt.state = "disconnected".into();
                                }
                                break;
                            }
                        },
                        None => break,
                    }
                };
                self.handle_client_event(&id, ev);
            }
        }
    }

    /// Full multi-client document or a single op.
    fn configure_json(&mut self, text: &str) -> Result<(), String> {
        let value: Value = serde_json::from_str(text).map_err(|e| format!("configure: {e}"))?;
        if let Some(op) = value.get("op").and_then(|v| v.as_str()) {
            return self.configure_op(op, &value);
        }
        // Legacy single-client document → one "default" client.
        if value.get("clients").is_none() && value.get("host").is_some() {
            let mut legacy: ClientConfig = serde_json::from_value(value)
                .map_err(|e| format!("configure legacy: {e}"))?;
            if legacy.id.trim().is_empty() {
                legacy.id = "default".into();
            }
            return self.reconcile_clients(vec![legacy]);
        }
        let clients: Vec<ClientConfig> = value
            .get("clients")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|e| format!("configure clients: {e}"))?
            .unwrap_or_default();
        self.reconcile_clients(clients)
    }

    fn reconcile_clients(&mut self, clients: Vec<ClientConfig>) -> Result<(), String> {
        let mut wanted = BTreeMap::new();
            let known: Vec<String> = clients.iter().map(|c| c.id.clone()).collect();
        let default_client = known
            .first()
            .cloned()
            .unwrap_or_else(|| "default".into());
        for mut cfg in clients {
            if cfg.id.trim().is_empty() {
                return Err("client id required".into());
            }
            cfg.transport = normalize_transport(&cfg.transport);
            if cfg.label.trim().is_empty() {
                cfg.label = cfg.id.clone();
            }
            // Migrate legacy mqtt/<topic> point overrides / output binds.
            for rule in &mut cfg.parse_rules {
                if !rule.point.trim().is_empty() {
                    rule.point = migrate_point_id(&rule.point, &known, &default_client);
                }
            }
            for out in &mut cfg.outputs {
                out.point = migrate_point_id(&out.point, &known, &cfg.id);
            }
            wanted.insert(cfg.id.clone(), cfg);
        }
        let existing: Vec<String> = self.clients.keys().cloned().collect();
        for id in existing {
            if !wanted.contains_key(&id) {
                if let Some(mut rt) = self.clients.remove(&id) {
                    Self::stop_client(&mut rt);
                }
            }
        }
        for (id, cfg) in wanted {
            let auto = cfg.auto_connect;
            let endpoint_changed = self
                .clients
                .get(&id)
                .map(|rt| {
                    rt.config.host != cfg.host
                        || rt.config.port != cfg.port
                        || rt.config.client_id != cfg.client_id
                        || rt.config.unique_client_id != cfg.unique_client_id
                        || rt.config.transport != cfg.transport
                        || rt.config.username != cfg.username
                        || rt.config.password != cfg.password
                })
                .unwrap_or(true);
            if let Some(rt) = self.clients.get_mut(&id) {
                rt.config = cfg;
            } else {
                self.clients.insert(id.clone(), ClientRuntime::new(cfg));
            }
            let rt = self.clients.get_mut(&id).unwrap();
            if auto {
                if endpoint_changed || rt.session.is_none() {
                    if let Err(e) = Self::start_client(rt) {
                        rt.last_error = e.clone();
                        rt.quality_bad = true;
                        self.log(PB_LOG_ERROR, &format!("[{id}] {e}"));
                    }
                } else if let Err(e) = Self::sync_subscriptions(rt) {
                    rt.last_error = e.clone();
                    rt.quality_bad = true;
                    self.log(PB_LOG_ERROR, &format!("[{id}] {e}"));
                }
            } else if rt.session.is_some() {
                Self::stop_client(rt);
            }
        }
        Ok(())
    }

    fn configure_op(&mut self, op: &str, value: &Value) -> Result<(), String> {
        let id = value
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        match op {
            "connect" => {
                let rt = self
                    .clients
                    .get_mut(&id)
                    .ok_or_else(|| format!("unknown client '{id}'"))?;
                rt.config.auto_connect = true;
                Self::start_client(rt)
            }
            "disconnect" => {
                let rt = self
                    .clients
                    .get_mut(&id)
                    .ok_or_else(|| format!("unknown client '{id}'"))?;
                rt.config.auto_connect = false;
                Self::stop_client(rt);
                Ok(())
            }
            "subscribe" => {
                let topic = value
                    .get("topic")
                    .and_then(|v| v.as_str())
                    .ok_or("subscribe requires topic")?
                    .to_string();
                let qos = value.get("qos").and_then(|v| v.as_u64()).unwrap_or(0) as u8;
                let rt = self
                    .clients
                    .get_mut(&id)
                    .ok_or_else(|| format!("unknown client '{id}'"))?;
                if !rt.config.subscriptions.iter().any(|t| t == &topic) {
                    rt.config.subscriptions.push(topic.clone());
                }
                if let Some(session) = rt.session.as_ref() {
                    session.subscribe(&topic, qos)?;
                }
                Ok(())
            }
            "publish" => {
                let topic = value
                    .get("topic")
                    .and_then(|v| v.as_str())
                    .ok_or("publish requires topic")?;
                let payload = value
                    .get("payload")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .as_bytes();
                let qos = value.get("qos").and_then(|v| v.as_u64()).unwrap_or(0) as u8;
                let retain = value.get("retain").and_then(|v| v.as_bool()).unwrap_or(false);
                let rt = self
                    .clients
                    .get(&id)
                    .ok_or_else(|| format!("unknown client '{id}'"))?;
                let session = rt
                    .session
                    .as_ref()
                    .ok_or_else(|| format!("publish: '{id}' not connected"))?;
                session.publish(topic, payload, qos, retain)
            }
            "remove" => {
                if let Some(mut rt) = self.clients.remove(&id) {
                    Self::stop_client(&mut rt);
                }
                Ok(())
            }
            other => Err(format!("unknown op '{other}'")),
        }
    }

    fn any_connected(&self) -> bool {
        self.clients.values().any(|c| c.connected)
    }

    fn aggregate_state(&self) -> String {
        if self.clients.is_empty() {
            return "idle".into();
        }
        if self.clients.values().any(|c| c.connected) {
            return "connected".into();
        }
        if self.clients.values().any(|c| c.state == "connecting") {
            return "connecting".into();
        }
        "disconnected".into()
    }

    fn aggregate_error(&self) -> String {
        self.clients
            .values()
            .filter(|c| !c.last_error.is_empty())
            .map(|c| format!("{}: {}", c.config.id, c.last_error))
            .collect::<Vec<_>>()
            .join("; ")
    }

    fn clients_snapshot_json(&self) -> String {
        let rows: Vec<Value> = self
            .clients
            .values()
            .map(|rt| {
                // Cap snapshot traffic so host buffer growth stays bounded under `#`.
                let traffic: Vec<Value> = rt
                    .traffic
                    .iter()
                    .take(400)
                    .map(|(topic, leaf)| {
                        serde_json::json!({
                            "topic": topic,
                            "payload": truncate_preview(&leaf.payload, 96),
                            "count": leaf.count,
                        })
                    })
                    .collect();
                serde_json::json!({
                    "id": rt.config.id,
                    "label": rt.config.label,
                    "state": rt.state,
                    "connected": rt.connected,
                    "last_error": rt.last_error,
                    "topics": rt.config.subscriptions,
                    "host": rt.config.host,
                    "port": rt.config.port,
                    "traffic": traffic,
                })
            })
            .collect();
        serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into())
    }

    fn known_client_ids(&self) -> Vec<String> {
        self.clients.keys().cloned().collect()
    }

    /// Publish via output binding if `point` matches; else derive topic from point id.
    fn publish_for_point(&mut self, point: &str, payload: &[u8]) -> Result<(), String> {
        let known = self.known_client_ids();
        let default = known.first().cloned().unwrap_or_else(|| "default".into());
        let point = migrate_point_id(point, &known, &default);
        for rt in self.clients.values() {
            if let Some(out) = rt.config.outputs.iter().find(|o| {
                migrate_point_id(&o.point, &known, &default) == point
            }) {
                let session = rt
                    .session
                    .as_ref()
                    .ok_or_else(|| format!("output: '{}' not connected", rt.config.id))?;
                return session.publish(&out.topic, payload, out.qos, out.retain);
            }
        }
        // Fallback: mqtt/<client>/<topic>[::path]
        let (client, topic, _) = split_point_id(&point, &known);
        let conn = client
            .map(|s| s.to_string())
            .ok_or_else(|| "write_point: no output binding and cannot derive client/topic".to_string())?;
        let value = serde_json::json!({
            "op": "publish",
            "id": conn,
            "topic": topic,
            "payload": String::from_utf8_lossy(payload),
            "qos": 0,
            "retain": false,
        });
        self.configure_op("publish", &value)
    }
}

fn normalize_transport(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "tls" | "mqtts" => "tls".into(),
        "ws" | "websocket" => "ws".into(),
        "wss" => "wss".into(),
        _ => "tcp".into(),
    }
}

fn truncate_preview(value: &str, max: usize) -> String {
    let flat = value.replace('\n', " ");
    if flat.chars().count() <= max {
        flat
    } else {
        let mut out: String = flat.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

#[no_mangle]
pub unsafe extern "C" fn pb_pack_abi_version() -> u32 {
    PB_ABI_VERSION
}

#[no_mangle]
pub unsafe extern "C" fn pb_pack_register(out: *mut PbPackApi) -> c_int {
    if out.is_null() {
        return -1;
    }
    *out = PbPackApi {
        abi_version: PB_ABI_VERSION,
        id: PACK_ID.as_ptr() as *const c_char,
        title: PACK_TITLE.as_ptr() as *const c_char,
        create: Some(create),
        destroy: Some(destroy),
        configure: Some(configure),
        connect: Some(connect),
        disconnect: Some(disconnect),
        subscribe: Some(subscribe),
        publish: Some(publish),
        write_point: Some(write_point),
        poll: Some(poll),
        state_label: Some(state_label),
        last_error: Some(last_error),
        is_connected: Some(is_connected),
        topics_json: Some(topics_json),
    };
    0
}

unsafe extern "C" fn create(host: *const PbHostApi) -> *mut c_void {
    if host.is_null() {
        return std::ptr::null_mut();
    }
    let host = *host;
    if host.abi_version != PB_ABI_VERSION {
        return std::ptr::null_mut();
    }
    Box::into_raw(Box::new(PackState::new(host))) as *mut c_void
}

unsafe extern "C" fn destroy(handle: *mut c_void) {
    if handle.is_null() {
        return;
    }
    let mut state = Box::from_raw(handle as *mut PackState);
    for rt in state.clients.values_mut() {
        PackState::stop_client(rt);
    }
}

unsafe extern "C" fn configure(handle: *mut c_void, json: *const c_char, len: usize) -> c_int {
    if handle.is_null() || json.is_null() {
        return -1;
    }
    let state = &mut *(handle as *mut PackState);
    let slice = std::slice::from_raw_parts(json as *const u8, len);
    let text = match std::str::from_utf8(slice) {
        Ok(t) => t,
        Err(_) => return -1,
    };
    match state.configure_json(text) {
        Ok(()) => 0,
        Err(e) => {
            state.log(PB_LOG_ERROR, &e);
            -1
        }
    }
}

/// Connect all clients marked auto_connect (or every client if none flagged).
unsafe extern "C" fn connect(handle: *mut c_void) -> c_int {
    if handle.is_null() {
        return -1;
    }
    let state = &mut *(handle as *mut PackState);
    let ids: Vec<String> = if state.clients.values().any(|c| c.config.auto_connect) {
        state
            .clients
            .iter()
            .filter(|(_, c)| c.config.auto_connect)
            .map(|(id, _)| id.clone())
            .collect()
    } else {
        state.clients.keys().cloned().collect()
    };
    let mut ok = true;
    for id in ids {
        if let Some(rt) = state.clients.get_mut(&id) {
            rt.config.auto_connect = true;
            if let Err(e) = PackState::start_client(rt) {
                rt.last_error = e;
                ok = false;
            }
        }
    }
    if ok {
        0
    } else {
        -1
    }
}

unsafe extern "C" fn disconnect(handle: *mut c_void) -> c_int {
    if handle.is_null() {
        return -1;
    }
    let state = &mut *(handle as *mut PackState);
    for rt in state.clients.values_mut() {
        rt.config.auto_connect = false;
        PackState::stop_client(rt);
    }
    0
}

unsafe extern "C" fn subscribe(handle: *mut c_void, topic: *const c_char, qos: u8) -> c_int {
    if handle.is_null() || topic.is_null() {
        return -1;
    }
    let state = &mut *(handle as *mut PackState);
    let topic = match CStr::from_ptr(topic).to_str() {
        Ok(t) => t.to_string(),
        Err(_) => return -1,
    };
    // Optional "client_id\ttopic" encoding from the host.
    let (client_id, topic) = match topic.split_once('\t') {
        Some((id, t)) => (id.to_string(), t.to_string()),
        None => {
            let id = state
                .clients
                .keys()
                .next()
                .cloned()
                .unwrap_or_else(|| "default".into());
            (id, topic)
        }
    };
    let value = serde_json::json!({
        "op": "subscribe",
        "id": client_id,
        "topic": topic,
        "qos": qos,
    });
    match state.configure_op("subscribe", &value) {
        Ok(()) => 0,
        Err(e) => {
            state.log(PB_LOG_ERROR, &e);
            -1
        }
    }
}

unsafe extern "C" fn publish(
    handle: *mut c_void,
    topic: *const c_char,
    payload: *const u8,
    len: usize,
    qos: u8,
    retain: c_int,
) -> c_int {
    if handle.is_null() || topic.is_null() {
        return -1;
    }
    let state = &mut *(handle as *mut PackState);
    let topic = match CStr::from_ptr(topic).to_str() {
        Ok(t) => t.to_string(),
        Err(_) => return -1,
    };
    let (client_id, topic) = match topic.split_once('\t') {
        Some((id, t)) => (id.to_string(), t.to_string()),
        None => {
            let id = state
                .clients
                .keys()
                .next()
                .cloned()
                .unwrap_or_else(|| "default".into());
            (id, topic)
        }
    };
    let bytes = if payload.is_null() || len == 0 {
        Vec::new()
    } else {
        std::slice::from_raw_parts(payload, len).to_vec()
    };
    let payload_str = String::from_utf8_lossy(&bytes).into_owned();
    let value = serde_json::json!({
        "op": "publish",
        "id": client_id,
        "topic": topic,
        "payload": payload_str,
        "qos": qos,
        "retain": retain != 0,
    });
    match state.configure_op("publish", &value) {
        Ok(()) => 0,
        Err(e) => {
            state.log(PB_LOG_ERROR, &e);
            -1
        }
    }
}

unsafe extern "C" fn write_point(
    handle: *mut c_void,
    name: *const c_char,
    value_kind: i32,
    value: *const c_void,
    value_len: usize,
) -> c_int {
    if handle.is_null() || name.is_null() {
        return -1;
    }
    let state = &mut *(handle as *mut PackState);
    let name = match CStr::from_ptr(name).to_str() {
        Ok(t) => t,
        Err(_) => return -1,
    };
    let payload: Vec<u8> = match value_kind {
        PB_VALUE_STRING | PB_VALUE_BYTES => {
            if value.is_null() {
                Vec::new()
            } else {
                std::slice::from_raw_parts(value as *const u8, value_len).to_vec()
            }
        }
        _ => return -1,
    };
    match state.publish_for_point(name, &payload) {
        Ok(()) => 0,
        Err(e) => {
            state.log(PB_LOG_ERROR, &e);
            -1
        }
    }
}

unsafe extern "C" fn poll(handle: *mut c_void) {
    if handle.is_null() {
        return;
    }
    let state = &mut *(handle as *mut PackState);
    state.drain();
}

unsafe extern "C" fn state_label(handle: *mut c_void, out: *mut c_char, out_len: usize) -> c_int {
    if handle.is_null() {
        return -1;
    }
    let state = &*(handle as *mut PackState);
    write_cstr(out, out_len, &state.aggregate_state())
}

unsafe extern "C" fn last_error(handle: *mut c_void, out: *mut c_char, out_len: usize) -> c_int {
    if handle.is_null() {
        return -1;
    }
    let state = &*(handle as *mut PackState);
    write_cstr(out, out_len, &state.aggregate_error())
}

unsafe extern "C" fn is_connected(handle: *mut c_void) -> c_int {
    if handle.is_null() {
        return 0;
    }
    let state = &*(handle as *mut PackState);
    if state.any_connected() {
        1
    } else {
        0
    }
}

unsafe extern "C" fn topics_json(handle: *mut c_void, out: *mut c_char, out_len: usize) -> c_int {
    if handle.is_null() {
        return -1;
    }
    // Host/UI reads the multi-client snapshot from this buffer.
    let state = &*(handle as *mut PackState);
    write_cstr(out, out_len, &state.clients_snapshot_json())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_fills_api() {
        let mut api = unsafe { std::mem::zeroed::<PbPackApi>() };
        let rc = unsafe { pb_pack_register(&mut api) };
        assert_eq!(rc, 0);
        assert_eq!(api.abi_version, PB_ABI_VERSION);
    }

    #[test]
    fn client_point_ids() {
        assert_eq!(point_id_for_client_topic("local", "a/b"), "mqtt/local/a/b");
        assert_eq!(
            point_id_for_client_field("local", "a/b", "temp"),
            "mqtt/local/a/b::temp"
        );
        let clients = vec!["local".into()];
        assert_eq!(
            migrate_point_id("mqtt/a/b", &clients, "local"),
            "mqtt/local/a/b"
        );
    }

    #[test]
    fn reconcile_empty_ok() {
        let host = PbHostApi {
            abi_version: PB_ABI_VERSION,
            ctx: std::ptr::null_mut(),
            upsert_point: None,
            log_message: None,
        };
        let mut state = PackState::new(host);
        assert!(state.reconcile_clients(Vec::new()).is_ok());
    }
}
