//! Protocol Bench project file (JSON) — multi-client MQTT + point id bindings.
//!
//! Default path: `directories` → `…/ProtocolBench/project.json`
//! Layout/theme stay in `ui.toml`.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::mqtt_panel::{MqttClient, MqttOutput, MqttPanelState, MqttParseRule};
use pb_points::{is_address_path, is_short_point_id, PointBinding, PointStore};

pub const PROJECT_VERSION: u32 = 3;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProjectFile {
    #[serde(default = "default_version")]
    pub version: u32,
    /// Short id ↔ source address bindings (`mqtt_1` → `mqtt/local/topic`).
    #[serde(default)]
    pub points: Vec<PointBindingSerde>,
    /// Next `mqtt_N` number to issue (never reuse within a project).
    #[serde(default = "default_next_point_seq")]
    pub next_point_seq: u64,
    #[serde(default)]
    pub mqtt: MqttProject,
}

fn default_next_point_seq() -> u64 {
    1
}

fn default_version() -> u32 {
    PROJECT_VERSION
}

impl Default for ProjectFile {
    fn default() -> Self {
        Self {
            version: PROJECT_VERSION,
            points: Vec::new(),
            next_point_seq: 1,
            mqtt: MqttProject::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PointBindingSerde {
    pub id: String,
    pub source: String,
}

impl From<&PointBinding> for PointBindingSerde {
    fn from(b: &PointBinding) -> Self {
        Self {
            id: b.id.clone(),
            source: b.source.clone(),
        }
    }
}

impl PointBindingSerde {
    pub fn to_binding(&self) -> PointBinding {
        PointBinding {
            id: self.id.clone(),
            source: self.source.clone(),
        }
    }
}

/// Multi-client MQTT section (v2+). v1 single-client files migrate on load.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MqttProject {
    #[serde(default)]
    pub clients: Vec<MqttClientSerde>,
    /// Selected connection id in the panel (optional).
    #[serde(default)]
    pub selected_id: String,
}

impl Default for MqttProject {
    fn default() -> Self {
        let client = MqttClientSerde::default_local();
        Self {
            selected_id: client.id.clone(),
            clients: vec![client],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MqttClientSerde {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_mqtt_client_id")]
    pub client_id: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default = "default_transport")]
    pub transport: String,
    #[serde(default)]
    pub subscriptions: Vec<String>,
    #[serde(default)]
    pub parse_rules: Vec<MqttParseRuleSerde>,
    #[serde(default)]
    pub outputs: Vec<MqttOutputSerde>,
    #[serde(default)]
    pub auto_connect: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MqttParseRuleSerde {
    #[serde(default)]
    pub topic: String,
    pub path: String,
    #[serde(default)]
    pub point: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MqttOutputSerde {
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
fn default_mqtt_client_id() -> String {
    "protocol-bench".into()
}
fn default_transport() -> String {
    "tcp".into()
}

impl MqttClientSerde {
    fn default_local() -> Self {
        Self {
            id: "local".into(),
            label: "Local broker".into(),
            host: default_host(),
            port: default_port(),
            client_id: default_mqtt_client_id(),
            username: String::new(),
            password: String::new(),
            transport: default_transport(),
            subscriptions: vec!["pb/#".into()],
            parse_rules: Vec::new(),
            outputs: Vec::new(),
            auto_connect: false,
        }
    }

    fn from_client(c: &MqttClient) -> Self {
        Self {
            id: c.id.clone(),
            label: c.label.clone(),
            host: c.host.clone(),
            port: c.port.parse().unwrap_or(1883),
            client_id: c.client_id.clone(),
            username: c.username.clone(),
            password: c.password.clone(),
            transport: normalize_transport(&c.transport),
            subscriptions: c.subscriptions.clone(),
            parse_rules: c
                .parse_rules
                .iter()
                .map(|r| MqttParseRuleSerde {
                    topic: r.topic.clone(),
                    path: r.path.clone(),
                    point: r.point.clone(),
                })
                .collect(),
            outputs: c
                .outputs
                .iter()
                .map(|o| MqttOutputSerde {
                    point: o.point.clone(),
                    topic: o.topic.clone(),
                    qos: o.qos,
                    retain: o.retain,
                })
                .collect(),
            auto_connect: c.auto_connect,
        }
    }

    fn to_client(&self) -> MqttClient {
        MqttClient {
            id: self.id.clone(),
            label: if self.label.is_empty() {
                self.id.clone()
            } else {
                self.label.clone()
            },
            host: self.host.clone(),
            port: self.port.to_string(),
            client_id: self.client_id.clone(),
            username: self.username.clone(),
            password: self.password.clone(),
            transport: normalize_transport(&self.transport),
            subscriptions: self.subscriptions.clone(),
            parse_rules: self
                .parse_rules
                .iter()
                .map(|r| MqttParseRule {
                    topic: r.topic.clone(),
                    path: r.path.clone(),
                    point: r.point.clone(),
                })
                .collect(),
            outputs: self
                .outputs
                .iter()
                .map(|o| MqttOutput {
                    point: o.point.clone(),
                    topic: o.topic.clone(),
                    qos: o.qos,
                    retain: o.retain,
                })
                .collect(),
            auto_connect: self.auto_connect,
            draft_sub: String::new(),
            draft_path_topic: String::new(),
            draft_path: String::new(),
            draft_path_point: String::new(),
            draft_out_point: String::new(),
            draft_out_topic: String::new(),
        }
    }
}

impl MqttProject {
    pub fn from_panel(state: &MqttPanelState) -> Self {
        let clients = state
            .clients
            .iter()
            .map(MqttClientSerde::from_client)
            .collect();
        let selected_id = state
            .selected()
            .map(|c| c.id.clone())
            .unwrap_or_default();
        Self {
            clients,
            selected_id,
        }
    }

    pub fn apply_to_panel(&self, state: &mut MqttPanelState) {
        state.clients = self.clients.iter().map(|c| c.to_client()).collect();
        if state.clients.is_empty() {
            state.clients.push(MqttClient::default_local());
        }
        state.selected = state
            .clients
            .iter()
            .position(|c| c.id == self.selected_id)
            .unwrap_or(0);
    }
}

/// Rewrite legacy long-path point refs to short ids via the host store.
pub fn migrate_panel_point_refs(state: &mut MqttPanelState, store: &mut PointStore) {
    let known: Vec<String> = state.clients.iter().map(|c| c.id.clone()).collect();
    for client in &mut state.clients {
        for rule in &mut client.parse_rules {
            if !rule.point.is_empty() {
                rule.point = resolve_point_ref(&rule.point, &known, &client.id, store);
            }
        }
        for out in &mut client.outputs {
            out.point = resolve_point_ref(&out.point, &known, &client.id, store);
        }
    }
}

fn resolve_point_ref(
    raw: &str,
    known_clients: &[String],
    default_client: &str,
    store: &mut PointStore,
) -> String {
    let raw = raw.trim();
    if raw.is_empty() || is_short_point_id(raw) {
        return raw.to_string();
    }
    let address = if is_address_path(raw) {
        migrate_address(raw, known_clients, default_client)
    } else {
        return raw.to_string();
    };
    store.ensure_id("mqtt", &address)
}

/// Rewrite legacy `mqtt/<topic>` → `mqtt/<client>/<topic>`.
fn migrate_address(name: &str, known_clients: &[String], default_client: &str) -> String {
    let name = name.trim();
    if !name.starts_with("mqtt/") {
        return name.to_string();
    }
    let rest = &name["mqtt/".len()..];
    let (body, path) = match rest.split_once("::") {
        Some((b, p)) if !p.is_empty() => (b, Some(p)),
        _ => (rest, None),
    };
    if let Some((first, topic)) = body.split_once('/') {
        if !topic.is_empty() && known_clients.iter().any(|c| c == first) {
            return name.to_string();
        }
    }
    let client = if default_client.is_empty() {
        "default"
    } else {
        default_client
    };
    match path {
        Some(p) => format!("mqtt/{client}/{body}::{p}"),
        None => format!("mqtt/{client}/{body}"),
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

pub fn default_project_path() -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("ca", "AgileAutomation", "ProtocolBench")?;
    Some(dirs.config_dir().join("project.json"))
}

pub fn load(path: &Path) -> Result<ProjectFile, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    // v1 single-client → wrap as one client.
    if value.get("mqtt").and_then(|m| m.get("clients")).is_none()
        && value
            .get("mqtt")
            .and_then(|m| m.get("host"))
            .is_some()
    {
        let legacy = value.get("mqtt").cloned().unwrap_or(serde_json::json!({}));
        let mut client = MqttClientSerde::default_local();
        if let Some(h) = legacy.get("host").and_then(|v| v.as_str()) {
            client.host = h.into();
        }
        if let Some(p) = legacy.get("port").and_then(|v| v.as_u64()) {
            client.port = p as u16;
        }
        if let Some(c) = legacy.get("client_id").and_then(|v| v.as_str()) {
            client.client_id = c.into();
        }
        if let Some(u) = legacy.get("username").and_then(|v| v.as_str()) {
            client.username = u.into();
        }
        if let Some(p) = legacy.get("password").and_then(|v| v.as_str()) {
            client.password = p.into();
        }
        if let Some(t) = legacy.get("transport").and_then(|v| v.as_str()) {
            client.transport = normalize_transport(t);
        }
        if let Some(subs) = legacy.get("subscriptions").and_then(|v| v.as_array()) {
            client.subscriptions = subs
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();
        } else if let Some(s) = legacy.get("subscribe_topic").and_then(|v| v.as_str()) {
            client.subscriptions = vec![s.into()];
        }
        if let Some(rules) = legacy.get("parse_rules").cloned() {
            client.parse_rules = serde_json::from_value(rules).unwrap_or_default();
        }
        client.auto_connect = legacy
            .get("auto_connect")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        return Ok(ProjectFile {
            version: PROJECT_VERSION,
            points: Vec::new(),
            next_point_seq: 1,
            mqtt: MqttProject {
                selected_id: client.id.clone(),
                clients: vec![client],
            },
        });
    }
    let mut project: ProjectFile = serde_json::from_value(value).map_err(|e| e.to_string())?;
    for c in &mut project.mqtt.clients {
        c.transport = normalize_transport(&c.transport);
    }
    if project.mqtt.clients.is_empty() {
        project.mqtt = MqttProject::default();
    }
    if project.next_point_seq == 0 {
        project.next_point_seq = 1;
    }
    // High-water mark from any persisted ids.
    for b in &project.points {
        if let Some(n) = b.id.rsplit_once('_').and_then(|(_, s)| s.parse::<u64>().ok()) {
            project.next_point_seq = project.next_point_seq.max(n.saturating_add(1));
        }
    }
    project.version = PROJECT_VERSION;
    Ok(project)
}

pub fn save(path: &Path, project: &ProjectFile) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(project).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| e.to_string())
}

/// Snapshot point bindings from the live host store for project.json.
pub fn bindings_from_store(store: &PointStore) -> Vec<PointBindingSerde> {
    store
        .export_bindings()
        .iter()
        .map(PointBindingSerde::from)
        .collect()
}

pub fn import_bindings_to_store(
    store: &mut PointStore,
    bindings: &[PointBindingSerde],
    next_point_seq: u64,
) {
    let list: Vec<PointBinding> = bindings.iter().map(|b| b.to_binding()).collect();
    store.import_bindings(&list);
    store.set_next_seq_at_least(next_point_seq.max(1));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_client_roundtrip() {
        let mut panel = MqttPanelState::default();
        panel.clients[0].host = "broker.local".into();
        let project = ProjectFile {
            version: PROJECT_VERSION,
            points: vec![PointBindingSerde {
                id: "mqtt_1".into(),
                source: "mqtt/local/pb/in".into(),
            }],
            next_point_seq: 2,
            mqtt: MqttProject::from_panel(&panel),
        };
        let text = serde_json::to_string_pretty(&project).unwrap();
        let back: ProjectFile = serde_json::from_str(&text).unwrap();
        assert_eq!(back.mqtt.clients[0].host, "broker.local");
        assert_eq!(back.points[0].id, "mqtt_1");
    }

    #[test]
    fn migrates_long_path_output_to_short_id() {
        let mut panel = MqttPanelState::default();
        panel.clients[0].outputs.push(MqttOutput {
            point: "mqtt/local/pb/out".into(),
            topic: "pb/out".into(),
            qos: 0,
            retain: false,
        });
        let mut store = PointStore::new();
        migrate_panel_point_refs(&mut panel, &mut store);
        let id = &panel.clients[0].outputs[0].point;
        assert!(is_short_point_id(id), "{id}");
        assert_eq!(store.source_for_id(id), Some("mqtt/local/pb/out"));
    }
}
