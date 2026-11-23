//! OPC UA protocol pack (cdylib) for Protocol Bench.
//!
//! External UA clients (async-opcua) + optional embedded UA server hub.
//! Mirrors `pb-mqtt` ABI / configure-op patterns.

mod server;
mod session;

use pb_abi::{HostRef, Pack, PB_LOG_ERROR, PB_LOG_INFO, PB_QUALITY_BAD, PB_QUALITY_GOOD};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use server::{EmbeddedServer, EmbeddedServerConfig, PointSnap};
use session::{BrowseNode, OpcUaSession, SessionEvent};
use std::collections::BTreeMap;
use std::os::raw::c_int;
use std::sync::mpsc::TryRecvError;

const PACK_ID: &[u8] = b"opcua\0";
const PACK_TITLE: &[u8] = b"OPC UA\0";

/// One outbound UA client connection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClientConfig {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default = "default_endpoint")]
    pub endpoint_url: String,
    #[serde(default = "default_policy")]
    pub security_policy: String,
    #[serde(default = "default_mode")]
    pub security_mode: String,
    #[serde(default = "default_auth")]
    pub auth: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    /// `strict` | `trust_once` (maps to trust_server_certs).
    #[serde(default = "default_trust")]
    pub trust_server: String,
    #[serde(default)]
    pub pki_dir: String,
    #[serde(default = "default_sub_interval")]
    pub subscription_interval_ms: u64,
    /// NodeIds to monitor (string form).
    #[serde(default)]
    pub monitored_nodes: Vec<MonitoredBind>,
    #[serde(default)]
    pub auto_connect: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MonitoredBind {
    pub node_id: String,
    #[serde(default)]
    pub point: String,
    /// `subscribe` (preferred) or `read_poll`.
    #[serde(default = "default_mode_sub")]
    pub mode: String,
}

fn default_endpoint() -> String {
    "opc.tcp://127.0.0.1:4840".into()
}
fn default_policy() -> String {
    "None".into()
}
fn default_mode() -> String {
    "None".into()
}
fn default_auth() -> String {
    "anonymous".into()
}
fn default_trust() -> String {
    "strict".into()
}
fn default_sub_interval() -> u64 {
    500
}
fn default_mode_sub() -> String {
    "subscribe".into()
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            id: "plant".into(),
            label: "Plant UA".into(),
            endpoint_url: default_endpoint(),
            security_policy: default_policy(),
            security_mode: default_mode(),
            auth: default_auth(),
            username: String::new(),
            password: String::new(),
            trust_server: default_trust(),
            pki_dir: String::new(),
            subscription_interval_ms: default_sub_interval(),
            monitored_nodes: Vec::new(),
            auto_connect: false,
        }
    }
}

struct ClientRuntime {
    config: ClientConfig,
    session: Option<OpcUaSession>,
    connected: bool,
    state: String,
    last_error: String,
    /// Parent NodeId → children (PB-89 hierarchical browse).
    browse_by_parent: BTreeMap<String, Vec<BrowseNode>>,
    checklist: String,
}

impl ClientRuntime {
    fn new(config: ClientConfig) -> Self {
        Self {
            config,
            session: None,
            connected: false,
            state: "disconnected".into(),
            last_error: String::new(),
            browse_by_parent: BTreeMap::new(),
            checklist: String::new(),
        }
    }
}

struct PackState {
    host: HostRef,
    clients: BTreeMap<String, ClientRuntime>,
    embedded: Option<EmbeddedServer>,
    embedded_error: String,
    embedded_cfg: Option<EmbeddedServerConfig>,
}

impl PackState {
    fn new(host: HostRef) -> Self {
        Self {
            host,
            clients: BTreeMap::new(),
            embedded: None,
            embedded_error: String::new(),
            embedded_cfg: None,
        }
    }

    fn log(&self, level: i32, msg: &str) {
        self.host.log(level, msg);
    }

    fn upsert_string(&self, name: &str, source: &str, value: &str, good: bool) {
        let quality = if good { PB_QUALITY_GOOD } else { PB_QUALITY_BAD };
        self.host.upsert_text(name, source, value, quality);
    }

    fn stop_client(rt: &mut ClientRuntime) {
        if let Some(session) = rt.session.take() {
            session.stop();
        }
        rt.connected = false;
        rt.state = "disconnected".into();
        rt.browse_by_parent.clear();
    }

    fn start_client(rt: &mut ClientRuntime) -> Result<(), String> {
        Self::stop_client(rt);
        let trust = rt.config.trust_server.eq_ignore_ascii_case("trust_once")
            || rt.config.trust_server.eq_ignore_ascii_case("true");
        let user = if rt.config.auth.eq_ignore_ascii_case("username") {
            rt.config.username.clone()
        } else {
            String::new()
        };
        let session = OpcUaSession::spawn(
            rt.config.endpoint_url.clone(),
            rt.config.security_policy.clone(),
            rt.config.security_mode.clone(),
            trust,
            rt.config.pki_dir.clone(),
            user,
            rt.config.password.clone(),
        )?;
        rt.session = Some(session);
        rt.state = "connecting".into();
        rt.last_error.clear();
        rt.checklist.clear();
        rt.connected = false;
        Ok(())
    }

    fn sync_monitors(rt: &mut ClientRuntime) -> Result<(), String> {
        let Some(session) = rt.session.as_ref() else {
            return Ok(());
        };
        let ids: Vec<String> = rt
            .config
            .monitored_nodes
            .iter()
            .map(|b| b.node_id.clone())
            .filter(|s| !s.trim().is_empty())
            .collect();
        if ids.is_empty() {
            return Ok(());
        }
        let prefer_poll = rt
            .config
            .monitored_nodes
            .iter()
            .any(|b| b.mode.eq_ignore_ascii_case("read_poll"));
        if prefer_poll {
            session.read_poll(ids)
        } else {
            session.subscribe(ids, rt.config.subscription_interval_ms)
        }
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
                } else if detail.contains("connecting") {
                    "connecting".into()
                } else {
                    "disconnected".into()
                };
                if connected {
                    rt.last_error.clear();
                    rt.checklist.clear();
                    if let Err(e) = Self::sync_monitors(rt) {
                        rt.last_error = e.clone();
                        self.log(PB_LOG_ERROR, &format!("[{id}] {e}"));
                    }
                }
                self.log(PB_LOG_INFO, &format!("[{id}] {detail}"));
            }
            SessionEvent::Browse { parent_id, nodes } => {
                rt.browse_by_parent.insert(parent_id, nodes);
            }
            SessionEvent::Data { node_id, value } => {
                let conn = rt.config.id.clone();
                let bind = rt
                    .config
                    .monitored_nodes
                    .iter()
                    .find(|b| b.node_id == node_id)
                    .cloned();
                let source = format!("opcua/{conn}/{node_id}");
                if let Some(b) = bind {
                    if b.point.trim().is_empty() {
                        self.upsert_string(&source, "opcua", &value, true);
                    } else {
                        self.upsert_string(b.point.trim(), &source, &value, true);
                    }
                } else {
                    self.upsert_string(&source, "opcua", &value, true);
                }
            }
            SessionEvent::Error { detail } => {
                rt.last_error = detail.clone();
                rt.checklist = connectivity_checklist(&detail);
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

    fn configure_json(&mut self, text: &str) -> Result<(), String> {
        let value: Value = serde_json::from_str(text).map_err(|e| format!("configure: {e}"))?;
        if let Some(op) = value.get("op").and_then(|v| v.as_str()) {
            return self.configure_op(op, &value);
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
        for mut cfg in clients {
            if cfg.id.trim().is_empty() {
                return Err("client id required".into());
            }
            if cfg.label.trim().is_empty() {
                cfg.label = cfg.id.clone();
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
                    rt.config.endpoint_url != cfg.endpoint_url
                        || rt.config.security_policy != cfg.security_policy
                        || rt.config.security_mode != cfg.security_mode
                        || rt.config.auth != cfg.auth
                        || rt.config.username != cfg.username
                        || rt.config.password != cfg.password
                        || rt.config.trust_server != cfg.trust_server
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
                        rt.checklist = connectivity_checklist(&e);
                        self.log(PB_LOG_ERROR, &format!("[{id}] {e}"));
                    }
                } else if let Err(e) = Self::sync_monitors(rt) {
                    rt.last_error = e.clone();
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
            "browse" => {
                let node_id = value
                    .get("node_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Objects");
                let rt = self
                    .clients
                    .get_mut(&id)
                    .ok_or_else(|| format!("unknown client '{id}'"))?;
                let session = rt
                    .session
                    .as_ref()
                    .ok_or_else(|| format!("browse: '{id}' not connected"))?;
                session.browse(node_id)
            }
            "monitor" => {
                let node_id = value
                    .get("node_id")
                    .and_then(|v| v.as_str())
                    .ok_or("monitor requires node_id")?
                    .to_string();
                let point = value
                    .get("point")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let mode = value
                    .get("mode")
                    .and_then(|v| v.as_str())
                    .unwrap_or("subscribe")
                    .to_string();
                let rt = self
                    .clients
                    .get_mut(&id)
                    .ok_or_else(|| format!("unknown client '{id}'"))?;
                if !rt.config.monitored_nodes.iter().any(|b| b.node_id == node_id) {
                    rt.config.monitored_nodes.push(MonitoredBind {
                        node_id,
                        point,
                        mode,
                    });
                }
                Self::sync_monitors(rt)
            }
            "server_start" => self.start_embedded(value),
            "server_stop" => {
                self.stop_embedded();
                Ok(())
            }
            "sync_points" => {
                let points: Vec<PointSnap> = value
                    .get("points")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|p| {
                                let text = |k: &str| {
                                    p.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string()
                                };
                                Some(PointSnap {
                                    browse_name: text("browse_name"),
                                    display_name: text("display_name"),
                                    folder: text("folder"),
                                    id: p.get("id")?.as_str()?.to_string(),
                                    value: p
                                        .get("value")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string(),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if let Some(server) = self.embedded.as_ref() {
                    server.sync_points(&points);
                }
                Ok(())
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

    fn start_embedded(&mut self, value: &Value) -> Result<(), String> {
        self.stop_embedded();
        let bind = value
            .get("bind")
            .and_then(|v| v.as_str())
            .unwrap_or("0.0.0.0")
            .to_string();
        let port = value.get("port").and_then(|v| v.as_u64()).unwrap_or(4840) as u16;
        let application_uri = value
            .get("application_uri")
            .and_then(|v| v.as_str())
            .unwrap_or("urn:protocol-bench:opcua:hub")
            .to_string();
        let application_name = value
            .get("application_name")
            .and_then(|v| v.as_str())
            .unwrap_or("Protocol Bench")
            .to_string();
        let allow_anonymous = value
            .get("allow_anonymous")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let enable_sign_and_encrypt = value
            .get("enable_sign_and_encrypt")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let auto_generate_cert = value
            .get("auto_generate_cert")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let auto_trust_clients = value
            .get("auto_trust_clients")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let pki_dir = value
            .get("pki_dir")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if allow_anonymous && (bind == "0.0.0.0" || bind == "::") {
            self.log(
                PB_LOG_INFO,
                "warning: anonymous OPC UA on non-loopback bind — LAN clients can connect",
            );
        }

        let cfg = EmbeddedServerConfig {
            bind,
            port,
            application_uri,
            application_name,
            allow_anonymous,
            enable_sign_and_encrypt,
            auto_generate_cert,
            auto_trust_clients,
            pki_dir,
        };
        match EmbeddedServer::start(cfg.clone()) {
            Ok(server) => {
                let url = server.listen_url().to_string();
                self.embedded = Some(server);
                self.embedded_cfg = Some(cfg);
                self.embedded_error.clear();
                self.log(PB_LOG_INFO, &format!("embedded OPC UA listening on {url}"));
                Ok(())
            }
            Err(e) => {
                self.embedded_error = e.clone();
                self.log(PB_LOG_ERROR, &format!("embedded OPC UA: {e}"));
                Err(format!(
                    "{e}. Checklist: port {port} free; PKI dir writable; firewall allows opc.tcp."
                ))
            }
        }
    }

    fn stop_embedded(&mut self) {
        if let Some(server) = self.embedded.take() {
            let url = server.listen_url().to_string();
            server.stop();
            self.log(PB_LOG_INFO, &format!("embedded OPC UA stopped ({url})"));
        }
        self.embedded_cfg = None;
        self.embedded_error.clear();
    }

    fn aggregate_state(&self) -> String {
        if self.embedded.is_some() {
            return "server".into();
        }
        if self.clients.values().any(|c| c.connected) {
            return "connected".into();
        }
        if self.clients.values().any(|c| c.state == "connecting") {
            return "connecting".into();
        }
        if self.clients.is_empty() {
            return "idle".into();
        }
        "disconnected".into()
    }

    fn aggregate_error(&self) -> String {
        let mut parts = Vec::new();
        if !self.embedded_error.is_empty() {
            parts.push(format!("server: {}", self.embedded_error));
        }
        for c in self.clients.values() {
            if !c.last_error.is_empty() {
                parts.push(format!("{}: {}", c.config.id, c.last_error));
            }
        }
        parts.join("; ")
    }

    fn clients_snapshot_json(&self) -> String {
        let rows: Vec<Value> = self
            .clients
            .values()
            .map(|rt| {
                // Parent → children map for hierarchical UI merge (PB-89).
                let mut browse = serde_json::Map::new();
                let mut remaining = 800usize;
                for (parent, kids) in &rt.browse_by_parent {
                    if remaining == 0 {
                        break;
                    }
                    let take = kids.len().min(remaining);
                    remaining -= take;
                    let rows: Vec<Value> = kids
                        .iter()
                        .take(take)
                        .map(|n| {
                            serde_json::json!({
                                "node_id": n.node_id,
                                "browse_name": n.browse_name,
                                "display_name": n.display_name,
                                "is_variable": n.is_variable,
                            })
                        })
                        .collect();
                    browse.insert(parent.clone(), Value::Array(rows));
                }
                serde_json::json!({
                    "id": rt.config.id,
                    "label": rt.config.label,
                    "state": rt.state,
                    "connected": rt.connected,
                    "last_error": rt.last_error,
                    "checklist": rt.checklist,
                    "endpoint_url": rt.config.endpoint_url,
                    "browse": browse,
                })
            })
            .collect();
        let mut root = serde_json::json!({
            "clients": rows,
            "server": {
                "running": self.embedded.is_some(),
                "error": self.embedded_error,
                "url": self.embedded.as_ref().map(|s| s.listen_url().to_string()).unwrap_or_default(),
            }
        });
        if let Some(cfg) = &self.embedded_cfg {
            root["server"]["bind"] = Value::String(cfg.bind.clone());
            root["server"]["port"] = Value::Number(cfg.port.into());
        }
        serde_json::to_string(&root).unwrap_or_else(|_| "{}".into())
    }
}

fn connectivity_checklist(detail: &str) -> String {
    let d = detail.to_ascii_lowercase();
    let mut steps = Vec::new();
    steps.push("1. Endpoint reachable? Check host:port and opc.tcp URL.".to_string());
    steps.push("2. Security mode/policy match the server endpoint.".to_string());
    steps.push("3. Certificate trusted? Use trust_once for lab, or import into PKI trusted/.".to_string());
    if d.contains("certificate") || d.contains("trust") || d.contains("security") {
        steps.push("Hint: failure looks cert/security related.".into());
    }
    if d.contains("connect") || d.contains("timeout") || d.contains("refused") {
        steps.push("Hint: failure looks network/reachability related.".into());
    }
    steps.join("\n")
}

impl Pack for PackState {
    const ID: &'static [u8] = PACK_ID;
    const TITLE: &'static [u8] = PACK_TITLE;
    const SUBSCRIBE: bool = true;

    fn new(host: HostRef) -> Self {
        PackState::new(host)
    }

    fn configure(&mut self, json: &str) -> Result<(), String> {
        self.configure_json(json)
    }

    fn config_failed(&mut self, err: String) {
        self.log(PB_LOG_ERROR, &err);
    }

    /// Connect the first client that has no session yet (the host calls this once per Start).
    fn connect(&mut self) -> c_int {
        for rt in self.clients.values_mut() {
            if rt.session.is_none() {
                return match PackState::start_client(rt) {
                    Ok(()) => 0,
                    Err(e) => {
                        rt.last_error = e;
                        -1
                    }
                };
            }
        }
        0
    }

    fn disconnect(&mut self) -> c_int {
        for rt in self.clients.values_mut() {
            PackState::stop_client(rt);
        }
        0
    }

    fn shutdown(&mut self) {
        self.stop_embedded();
        for rt in self.clients.values_mut() {
            PackState::stop_client(rt);
        }
        self.clients.clear();
    }

    /// The ABI's `subscribe` is reused: `topic` is a NodeId to monitor on the first connected client.
    fn subscribe(&mut self, node_id: &str, _qos: u8) -> c_int {
        let Some(rt) = self.clients.values_mut().find(|c| c.connected) else {
            return -1;
        };
        if !rt.config.monitored_nodes.iter().any(|b| b.node_id == node_id) {
            rt.config.monitored_nodes.push(MonitoredBind {
                node_id: node_id.to_string(),
                point: String::new(),
                mode: "subscribe".into(),
            });
        }
        match PackState::sync_monitors(rt) {
            Ok(()) => 0,
            Err(_) => -1,
        }
    }

    // `write_point` keeps the default (accept and ignore): v1 exposes points through the embedded
    // server (`sync_points`); writes to external UA servers are not implemented yet.

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
        self.embedded.is_some() || self.clients.values().any(|c| c.connected)
    }

    /// Snapshot JSON (clients + server + browse) the UI parses.
    fn detail_json(&self) -> String {
        self.clients_snapshot_json()
    }
}

pb_abi::export_pack!(PackState);

#[cfg(test)]
mod tests {
    use super::*;
    use pb_abi::{PbPackApi, PB_ABI_VERSION};

    #[test]
    fn register_fills_api() {
        let mut api = unsafe { std::mem::zeroed::<PbPackApi>() };
        assert_eq!(unsafe { pb_pack_register(&mut api) }, 0);
        assert_eq!(api.abi_version, PB_ABI_VERSION);
        assert!(!api.id.is_null());
    }
}
