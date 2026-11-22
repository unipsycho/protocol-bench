//! MQTT protocol pack (cdylib) for Protocol Bench.
//!
//! Multi-client rumqttc sessions + optional embedded rumqttd LAN broker.
//! Worker + EventLoop pattern from TopicBench `mqtt/live.rs` (no UI/tree).

mod broker;
mod json_path;
mod session;

use broker::EmbeddedBroker;

pub use json_path::{
    migrate_point_id, point_id_for_client_field, point_id_for_client_topic,
    split_point_id, topic_matches,
};

use pb_abi::{HostRef, Pack, PB_LOG_ERROR, PB_LOG_INFO, PB_QUALITY_BAD, PB_QUALITY_GOOD, PB_VALUE_BYTES, PB_VALUE_STRING};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use session::{MqttSession, SessionEvent, TransportKind};
use std::collections::BTreeMap;
use std::os::raw::c_int;
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
    host: HostRef,
    clients: BTreeMap<String, ClientRuntime>,
    /// Embedded hub broker (at most one).
    embedded: Option<EmbeddedBroker>,
    embedded_error: String,
}

impl PackState {
    fn new(host: HostRef) -> Self {
        Self {
            host,
            clients: BTreeMap::new(),
            embedded: None,
            embedded_error: String::new(),
        }
    }

    fn log(&self, level: i32, msg: &str) {
        self.host.log(level, msg);
    }

    /// Upsert a host point. `name` is a short id or long address; `source` is
    /// protocol (`mqtt`) when `name` is an address, or the address when `name` is short.
    fn upsert_string(&self, name: &str, source: &str, value: &str, good: bool) {
        self.host.upsert_text(name, source, value, quality(good));
    }

    fn upsert_bytes(&self, name: &str, source: &str, value: &[u8], good: bool) {
        self.host.upsert(name, source, PB_VALUE_BYTES, value, quality(good));
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
                    if let Some(value) = pb_json::extract_text(text, path) {
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
        // Peel `role: "broker"` entries — they are not rumqttc clients. Auto-start
        // when requested; never stop here (manual Start / broker_stop own that).
        let raw_clients = value
            .get("clients")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let mut client_vals = Vec::new();
        let mut broker: Option<Value> = None;
        for c in raw_clients {
            if c.get("role").and_then(|v| v.as_str()) == Some("broker") {
                broker = Some(c);
            } else {
                client_vals.push(c);
            }
        }
        let clients: Vec<ClientConfig> = serde_json::from_value(Value::Array(client_vals))
            .map_err(|e| format!("configure clients: {e}"))?;
        self.reconcile_clients(clients)?;
        if let Some(b) = broker {
            let auto = b
                .get("auto_start")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if auto {
                let start = broker_start_payload(&b);
                self.start_embedded(&start)?;
            }
        }
        Ok(())
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
            "broker_start" => self.start_embedded(value),
            "broker_stop" => {
                self.stop_embedded();
                Ok(())
            }
            other => Err(format!("unknown op '{other}'")),
        }
    }

    fn start_embedded(&mut self, value: &Value) -> Result<(), String> {
        let bind = value
            .get("bind")
            .and_then(|v| v.as_str())
            .unwrap_or("0.0.0.0");
        let port = value
            .get("port")
            .and_then(|v| v.as_u64())
            .unwrap_or(1885) as u16;
        // Already listening on this address — keep the accept loop.
        if let Some(broker) = self.embedded.as_ref() {
            let want: std::net::SocketAddr = format!("{bind}:{port}")
                .parse()
                .unwrap_or_else(|_| broker.listen_addr());
            if broker.listen_addr() == want
                || (bind == "0.0.0.0"
                    && broker.listen_addr().port() == port
                    && broker.listen_addr().ip().is_unspecified())
            {
                self.embedded_error.clear();
                return Ok(());
            }
        }
        self.stop_embedded();
        let users: Vec<(String, String)> = value
            .get("users")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|u| {
                        let username = u.get("username")?.as_str()?.to_string();
                        if username.is_empty() {
                            return None;
                        }
                        let password = u
                            .get("password")
                            .and_then(|p| p.as_str())
                            .unwrap_or("")
                            .to_string();
                        Some((username, password))
                    })
                    .collect()
            })
            .unwrap_or_default();
        match EmbeddedBroker::start(bind, port, &users) {
            Ok(broker) => {
                let addr = broker.listen_addr();
                self.embedded = Some(broker);
                self.embedded_error.clear();
                self.log(PB_LOG_INFO, &format!("embedded broker listening on {addr}"));
                Ok(())
            }
            Err(e) => {
                self.embedded_error = e.clone();
                self.log(PB_LOG_ERROR, &format!("embedded broker: {e}"));
                Err(e)
            }
        }
    }

    fn stop_embedded(&mut self) {
        if let Some(broker) = self.embedded.take() {
            let addr = broker.listen_addr();
            broker.stop();
            self.log(PB_LOG_INFO, &format!("embedded broker stopped ({addr})"));
        }
        self.embedded_error.clear();
    }

    fn any_connected(&self) -> bool {
        // Hub broker count as "up" so egress / status treat the pack as live.
        self.embedded.is_some() || self.clients.values().any(|c| c.connected)
    }

    fn aggregate_state(&self) -> String {
        if self.embedded.is_some() {
            return "broker".into();
        }
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
        let mut parts = Vec::new();
        if !self.embedded_error.is_empty() {
            parts.push(format!("broker: {}", self.embedded_error));
        }
        for c in self.clients.values() {
            if !c.last_error.is_empty() {
                parts.push(format!("{}: {}", c.config.id, c.last_error));
            }
        }
        parts.join("; ")
    }

    fn clients_snapshot_json(&self) -> String {
        let mut rows: Vec<Value> = Vec::new();
        if let Some(broker) = self.embedded.as_ref() {
            rows.push(serde_json::json!({
                "id": "hub",
                "label": "Hub broker",
                "role": "broker",
                "state": "listening",
                "connected": true,
                "listen": broker.listen_addr().to_string(),
                "last_error": "",
                "topics": [],
                "traffic": [],
            }));
        } else if !self.embedded_error.is_empty() {
            rows.push(serde_json::json!({
                "id": "hub",
                "label": "Hub broker",
                "role": "broker",
                "state": "error",
                "connected": false,
                "last_error": self.embedded_error,
                "topics": [],
                "traffic": [],
            }));
        }
        for rt in self.clients.values() {
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
            rows.push(serde_json::json!({
                "id": rt.config.id,
                "label": rt.config.label,
                "state": rt.state,
                "connected": rt.connected,
                "last_error": rt.last_error,
                "topics": rt.config.subscriptions,
                "host": rt.config.host,
                "port": rt.config.port,
                "traffic": traffic,
            }));
        }
        serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into())
    }

    fn known_client_ids(&self) -> Vec<String> {
        self.clients.keys().cloned().collect()
    }

    /// Publish via output binding if `point` matches.
    /// No matching bind → Ok (Input-only points). Bound but disconnected → Err (retry).
    fn publish_for_point(&mut self, point: &str, payload: &[u8]) -> Result<(), String> {
        let known = self.known_client_ids();
        let default = known.first().cloned().unwrap_or_else(|| "default".into());
        let point = migrate_point_id(point, &known, &default);
        let mut last_err: Option<String> = None;
        let mut matched = false;
        for rt in self.clients.values() {
            let Some(out) = rt.config.outputs.iter().find(|o| {
                migrate_point_id(&o.point, &known, &default) == point
            }) else {
                continue;
            };
            matched = true;
            let Some(session) = rt.session.as_ref() else {
                last_err = Some(format!("output: '{}' not connected", rt.config.id));
                continue;
            };
            return session.publish(&out.topic, payload, out.qos, out.retain);
        }
        if matched {
            return Err(last_err.unwrap_or_else(|| "output: not connected".into()));
        }
        // No Output bind for this point — quiet success (host egress walks all points).
        Ok(())
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

/// Shape a `role: broker` project entry into `broker_start` fields.
fn broker_start_payload(raw: &Value) -> Value {
    let bind = raw
        .get("bind")
        .and_then(|v| v.as_str())
        .or_else(|| raw.get("host").and_then(|v| v.as_str()))
        .unwrap_or("0.0.0.0");
    let port = raw
        .get("port")
        .and_then(|v| v.as_u64())
        .unwrap_or(1885) as u16;
    let anonymous = raw
        .get("anonymous")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let users = if anonymous {
        Value::Array(Vec::new())
    } else if let Some(arr) = raw.get("users").cloned() {
        arr
    } else {
        let username = raw
            .get("username")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let password = raw
            .get("password")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if username.trim().is_empty() {
            Value::Array(Vec::new())
        } else {
            serde_json::json!([{ "username": username, "password": password }])
        }
    };
    serde_json::json!({
        "bind": bind,
        "port": port,
        "users": users,
    })
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

fn quality(good: bool) -> i32 {
    if good { PB_QUALITY_GOOD } else { PB_QUALITY_BAD }
}

impl PackState {
    /// Host topics may be `"<client id>\t<topic>"`; a bare topic goes to the first client.
    fn split_client_topic(&self, raw: &str) -> (String, String) {
        match raw.split_once('\t') {
            Some((id, t)) => (id.to_string(), t.to_string()),
            None => (self.clients.keys().next().cloned().unwrap_or_else(|| "default".into()), raw.to_string()),
        }
    }
}

impl Pack for PackState {
    const ID: &'static [u8] = PACK_ID;
    const TITLE: &'static [u8] = PACK_TITLE;
    const SUBSCRIBE: bool = true;
    const PUBLISH: bool = true;

    fn new(host: HostRef) -> Self {
        PackState::new(host)
    }

    fn configure(&mut self, json: &str) -> Result<(), String> {
        self.configure_json(json)
    }

    fn config_failed(&mut self, err: String) {
        self.log(PB_LOG_ERROR, &err);
    }

    /// Connect all clients marked auto_connect (or every client if none flagged).
    fn connect(&mut self) -> c_int {
        let flagged = self.clients.values().any(|c| c.config.auto_connect);
        let ids: Vec<String> = self
            .clients
            .iter()
            .filter(|(_, c)| !flagged || c.config.auto_connect)
            .map(|(id, _)| id.clone())
            .collect();
        let mut ok = true;
        for id in ids {
            if let Some(rt) = self.clients.get_mut(&id) {
                rt.config.auto_connect = true;
                if let Err(e) = PackState::start_client(rt) {
                    rt.last_error = e;
                    ok = false;
                }
            }
        }
        if ok { 0 } else { -1 }
    }

    fn disconnect(&mut self) -> c_int {
        for rt in self.clients.values_mut() {
            rt.config.auto_connect = false;
            PackState::stop_client(rt);
        }
        0
    }

    fn shutdown(&mut self) {
        self.stop_embedded();
        for rt in self.clients.values_mut() {
            PackState::stop_client(rt);
        }
    }

    fn subscribe(&mut self, topic: &str, qos: u8) -> c_int {
        let (id, topic) = self.split_client_topic(topic);
        let value = serde_json::json!({ "op": "subscribe", "id": id, "topic": topic, "qos": qos });
        match self.configure_op("subscribe", &value) {
            Ok(()) => 0,
            Err(e) => {
                self.log(PB_LOG_ERROR, &e);
                -1
            }
        }
    }

    fn publish(&mut self, topic: &str, payload: &[u8], qos: u8, retain: bool) -> c_int {
        let (id, topic) = self.split_client_topic(topic);
        let value = serde_json::json!({
            "op": "publish", "id": id, "topic": topic,
            "payload": String::from_utf8_lossy(payload), "qos": qos, "retain": retain,
        });
        match self.configure_op("publish", &value) {
            Ok(()) => 0,
            Err(e) => {
                self.log(PB_LOG_ERROR, &e);
                -1
            }
        }
    }

    /// Host egress: publish `payload` to the Output bind for point `name` (string / bytes only).
    fn write_point(&mut self, name: &str, kind: i32, bytes: &[u8]) -> c_int {
        if !matches!(kind, PB_VALUE_STRING | PB_VALUE_BYTES) {
            return -1;
        }
        match self.publish_for_point(name, bytes) {
            Ok(()) => 0,
            Err(e) => {
                // Host egress retries until the Output client connects — avoid log spam.
                if !e.contains("not connected") {
                    self.log(PB_LOG_ERROR, &e);
                }
                -1
            }
        }
    }

    fn poll(&mut self) {
        self.drain();
    }

    fn state_label(&self) -> String {
        self.aggregate_state()
    }

    fn last_error(&self) -> String {
        self.aggregate_error()
    }

    fn is_connected(&self) -> bool {
        self.any_connected()
    }

    /// Multi-client snapshot (state, observed topics, traffic) the UI parses.
    fn detail_json(&self) -> String {
        self.clients_snapshot_json()
    }
}

pb_abi::export_pack!(PackState);

#[cfg(test)]
mod tests {
    use super::*;
    use pb_abi::{PbHostApi, PbPackApi, PB_ABI_VERSION};

    /// Host with no callbacks: upserts and logs are dropped.
    fn quiet_host() -> HostRef {
        HostRef::new(PbHostApi { abi_version: PB_ABI_VERSION, ctx: std::ptr::null_mut(), upsert_point: None, log_message: None })
    }

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
    fn traffic_snapshot_includes_observed_topics() {
        let mut state = PackState::new(quiet_host());
        let mut cfg = ClientConfig::default();
        cfg.id = "local".into();
        cfg.subscriptions = vec!["pb/#".into(), "sensor/+/temp".into()];
        state.reconcile_clients(vec![cfg]).unwrap();
        let rt = state.clients.get_mut("local").unwrap();
        rt.observe_traffic("pb/a/b", "{\"n\":1}");
        rt.observe_traffic("sensor/1/temp", "21.5");
        let snap = state.clients_snapshot_json();
        let v: Value = serde_json::from_str(&snap).unwrap();
        let traffic = v[0]["traffic"].as_array().unwrap();
        assert!(
            traffic.iter().any(|t| t["topic"] == "pb/a/b"),
            "missing pb/a/b in {snap}"
        );
        assert!(
            traffic.iter().any(|t| t["topic"] == "sensor/1/temp"),
            "missing sensor topic in {snap}"
        );
        let topics = v[0]["topics"].as_array().unwrap();
        assert!(topics.iter().any(|t| t == "pb/#"));
        assert!(topics.iter().any(|t| t == "sensor/+/temp"));
    }

    #[test]
    fn configure_broker_role_auto_starts() {
        let mut state = PackState::new(quiet_host());
        let port = 1895u16;
        let json = format!(
            r#"{{"clients":[{{"id":"hub","role":"broker","auto_start":true,"anonymous":true,"bind":"127.0.0.1","port":{port}}}]}}"#
        );
        state.configure_json(&json).expect("configure");
        assert!(state.embedded.is_some(), "hub broker should auto-start");
        assert_eq!(state.aggregate_state(), "broker");
        assert!(state.any_connected());
        state.stop_embedded();
    }

    #[test]
    fn reconcile_empty_ok() {
        let mut state = PackState::new(quiet_host());
        assert!(state.reconcile_clients(Vec::new()).is_ok());
    }
}
