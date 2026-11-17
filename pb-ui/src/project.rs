//! Protocol Bench project file (JSON) — MQTT connections, hub Input/Output binds,
//! point id bindings, and dock layout (PB-95).
//!
//! Default path: `directories` → `…/ProtocolBench/project.json`
//! Theme + auto-run hubs stay in `ui.toml`. Hub diagram is mirrored here when saved.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::dock::DockPane;
use crate::hubs::{HubModel, MqttInputBind, MqttOutputBind};
use crate::layout::LayoutNode;
use crate::mqtt_panel::{
    EmbeddedBrokerConfig, MqttClient, MqttOutput, MqttPanelState, MqttParseRule,
};
use crate::opcua_panel::{EmbeddedOpcUaConfig, OpcUaClient, OpcUaPanelState};
use crate::persist::UiState;
use pb_points::{is_address_path, is_short_point_id, PointBinding, PointStore};

/// v8: dock layout (tree / hidden / maximized) lives in the project (PB-95).
pub const PROJECT_VERSION: u32 = 8;

/// Project identity + options (Project panel / project.json).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ProjectSettings {
    /// Display name (falls back to file stem when empty).
    #[serde(default)]
    pub name: String,
    /// Free-form project notes.
    #[serde(default)]
    pub description: String,
    /// Legacy PB-57 field — migrated into `ui.toml` `auto_run_embedded_hubs` (PB-58).
    #[serde(default, skip_serializing)]
    pub auto_run_embedded_broker: bool,
}

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
    /// OPC UA clients + embedded UA server. Absent on older files → defaults.
    #[serde(default)]
    pub opcua: OpcUaProject,
    /// HTTP / REST outbound clients. Absent on older files → defaults.
    #[serde(default)]
    pub http: HttpProject,
    /// Streams / TCP endpoints (serial + TCP client/listener). Absent → defaults.
    #[serde(default)]
    pub streams: StreamsProject,
    /// Hub diagram + Input/Output MQTT binds. Absent on older files → keep ui.toml hubs.
    #[serde(default)]
    pub hubs: Option<HubModel>,
    #[serde(default)]
    pub settings: ProjectSettings,
    /// Dock visibility, tab order, splitters. Absent → seed once from current ui.toml (PB-95).
    #[serde(default)]
    pub layout: Option<DockLayout>,
}

/// Project-scoped dock layout (not theme / auto-run — those stay in `ui.toml`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DockLayout {
    #[serde(default)]
    pub tree: LayoutNode,
    #[serde(default)]
    pub hidden_panes: Vec<DockPane>,
    #[serde(default)]
    pub maximized: Option<DockPane>,
}

impl DockLayout {
    pub fn from_ui(state: &UiState) -> Self {
        Self {
            tree: state.tree.clone(),
            hidden_panes: state.hidden_panes.clone(),
            maximized: state.maximized,
        }
    }

    /// Apply tree / visibility / maximize; leaves theme, auto-run, and hubs alone.
    pub fn apply_to_ui(&self, state: &mut UiState) {
        state.tree = self.tree.clone();
        state.hidden_panes = self.hidden_panes.clone();
        state.maximized = self.maximized;
        state.sanitize();
    }
}

/// Restore project layout, or keep ui.toml and report that a seed write is needed.
/// Returns `true` when the project had no layout (caller should save to seed).
pub fn apply_or_seed_layout(file: &ProjectFile, ui: &mut UiState) -> bool {
    match &file.layout {
        Some(layout) => {
            layout.apply_to_ui(ui);
            false
        }
        None => true,
    }
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
            opcua: OpcUaProject::default(),
            http: HttpProject::default(),
            streams: StreamsProject::default(),
            hubs: None,
            settings: ProjectSettings::default(),
            layout: None,
        }
    }
}

/// HTTP / REST clients (outbound JSON).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HttpProject {
    #[serde(default)]
    pub clients: Vec<HttpClientSerde>,
    #[serde(default)]
    pub selected_id: String,
}

impl Default for HttpProject {
    fn default() -> Self {
        let client = HttpClientSerde::default_local();
        Self {
            selected_id: client.id.clone(),
            clients: vec![client],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HttpClientSerde {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default = "default_http_base")]
    pub base_url: String,
    #[serde(default = "default_http_auth")]
    pub auth: String,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default = "default_http_timeout")]
    pub timeout_secs: u32,
    #[serde(default)]
    pub headers: Vec<HttpHeaderSerde>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HttpHeaderSerde {
    pub name: String,
    pub value: String,
}

fn default_http_base() -> String {
    "http://127.0.0.1:8080".into()
}
fn default_http_auth() -> String {
    "none".into()
}
fn default_http_timeout() -> u32 {
    10
}

impl HttpClientSerde {
    fn default_local() -> Self {
        Self {
            id: "api".into(),
            label: "Local API".into(),
            base_url: default_http_base(),
            auth: default_http_auth(),
            token: String::new(),
            username: String::new(),
            password: String::new(),
            timeout_secs: default_http_timeout(),
            headers: vec![HttpHeaderSerde {
                name: "Content-Type".into(),
                value: "application/json".into(),
            }],
        }
    }

    fn from_client(c: &crate::http_panel::HttpClient) -> Self {
        Self {
            id: c.id.clone(),
            label: c.label.clone(),
            base_url: c.base_url.clone(),
            auth: c.auth.as_str().into(),
            token: c.token.clone(),
            username: c.username.clone(),
            password: c.password.clone(),
            timeout_secs: c.timeout_secs.parse().unwrap_or(10),
            headers: c
                .headers
                .iter()
                .map(|h| HttpHeaderSerde {
                    name: h.name.clone(),
                    value: h.value.clone(),
                })
                .collect(),
        }
    }

    fn to_client(&self) -> crate::http_panel::HttpClient {
        crate::http_panel::HttpClient {
            id: self.id.clone(),
            label: if self.label.is_empty() {
                self.id.clone()
            } else {
                self.label.clone()
            },
            base_url: self.base_url.clone(),
            auth: crate::http_panel::HttpAuthKind::from_str(&self.auth),
            token: self.token.clone(),
            username: self.username.clone(),
            password: self.password.clone(),
            timeout_secs: self.timeout_secs.to_string(),
            headers: self
                .headers
                .iter()
                .map(|h| crate::http_panel::HttpHeader {
                    name: h.name.clone(),
                    value: h.value.clone(),
                })
                .collect(),
            draft_header_name: String::new(),
            draft_header_value: String::new(),
        }
    }
}

impl HttpProject {
    pub fn from_panel(state: &crate::http_panel::HttpPanelState) -> Self {
        let clients = state
            .clients
            .iter()
            .map(HttpClientSerde::from_client)
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

    pub fn apply_to_panel(&self, state: &mut crate::http_panel::HttpPanelState) {
        state.clients = self.clients.iter().map(|c| c.to_client()).collect();
        if state.clients.is_empty() {
            state.clients.push(crate::http_panel::HttpClient::default_local());
        }
        state.selected = if self.selected_id.is_empty() {
            None
        } else {
            state
                .clients
                .iter()
                .position(|c| c.id == self.selected_id)
        };
    }
}


/// Streams / TCP endpoints (text serial + TCP).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StreamsProject {
    #[serde(default)]
    pub endpoints: Vec<StreamEndpointSerde>,
    #[serde(default)]
    pub selected_id: String,
}

impl Default for StreamsProject {
    fn default() -> Self {
        let ep = StreamEndpointSerde::default_serial();
        Self {
            selected_id: ep.id.clone(),
            endpoints: vec![ep],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StreamEndpointSerde {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default = "default_stream_kind")]
    pub kind: String,
    #[serde(default)]
    pub serial_port: String,
    #[serde(default = "default_baud")]
    pub baud: u32,
    #[serde(default = "default_data_bits")]
    pub data_bits: u8,
    #[serde(default = "default_parity")]
    pub parity: String,
    #[serde(default = "default_stop_bits")]
    pub stop_bits: u8,
    #[serde(default = "default_flow")]
    pub flow: String,
    #[serde(default = "default_stream_host")]
    pub host: String,
    #[serde(default = "default_tcp_port")]
    pub tcp_port: u16,
    #[serde(default = "default_true_stream")]
    pub reconnect: bool,
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default = "default_listen_port")]
    pub listen_port: u16,
}

fn default_stream_kind() -> String { "serial".into() }
fn default_baud() -> u32 { 9600 }
fn default_data_bits() -> u8 { 8 }
fn default_parity() -> String { "none".into() }
fn default_stop_bits() -> u8 { 1 }
fn default_flow() -> String { "none".into() }
fn default_stream_host() -> String { "127.0.0.1".into() }
fn default_tcp_port() -> u16 { 4001 }
fn default_true_stream() -> bool { true }
fn default_bind() -> String { "0.0.0.0".into() }
fn default_listen_port() -> u16 { 5001 }

impl StreamEndpointSerde {
    fn default_serial() -> Self {
        Self {
            id: "com1".into(),
            label: "Serial".into(),
            kind: default_stream_kind(),
            serial_port: if cfg!(windows) { "COM1".into() } else { "/dev/ttyUSB0".into() },
            baud: default_baud(),
            data_bits: default_data_bits(),
            parity: default_parity(),
            stop_bits: default_stop_bits(),
            flow: default_flow(),
            host: default_stream_host(),
            tcp_port: default_tcp_port(),
            reconnect: true,
            bind: default_bind(),
            listen_port: default_listen_port(),
        }
    }

    fn from_endpoint(e: &crate::streams_panel::StreamEndpoint) -> Self {
        Self {
            id: e.id.clone(),
            label: e.label.clone(),
            kind: e.kind.as_str().into(),
            serial_port: e.serial_port.clone(),
            baud: e.baud.parse().unwrap_or(9600),
            data_bits: e.data_bits.parse().unwrap_or(8),
            parity: e.parity.clone(),
            stop_bits: e.stop_bits.parse().unwrap_or(1),
            flow: e.flow.clone(),
            host: e.host.clone(),
            tcp_port: e.tcp_port.parse().unwrap_or(4001),
            reconnect: e.reconnect,
            bind: e.bind.clone(),
            listen_port: e.listen_port.parse().unwrap_or(5001),
        }
    }

    fn to_endpoint(&self) -> crate::streams_panel::StreamEndpoint {
        crate::streams_panel::StreamEndpoint {
            id: self.id.clone(),
            label: if self.label.is_empty() { self.id.clone() } else { self.label.clone() },
            kind: crate::streams_panel::StreamKind::from_str(&self.kind),
            serial_port: self.serial_port.clone(),
            baud: self.baud.to_string(),
            data_bits: self.data_bits.to_string(),
            parity: self.parity.clone(),
            stop_bits: self.stop_bits.to_string(),
            flow: self.flow.clone(),
            host: self.host.clone(),
            tcp_port: self.tcp_port.to_string(),
            reconnect: self.reconnect,
            bind: self.bind.clone(),
            listen_port: self.listen_port.to_string(),
        }
    }
}

impl StreamsProject {
    pub fn from_panel(state: &crate::streams_panel::StreamsPanelState) -> Self {
        let endpoints = state.endpoints.iter().map(StreamEndpointSerde::from_endpoint).collect();
        let selected_id = state.selected().map(|e| e.id.clone()).unwrap_or_default();
        Self { endpoints, selected_id }
    }

    pub fn apply_to_panel(&self, state: &mut crate::streams_panel::StreamsPanelState) {
        state.endpoints = self.endpoints.iter().map(|e| e.to_endpoint()).collect();
        if state.endpoints.is_empty() {
            state.endpoints.push(crate::streams_panel::StreamEndpoint::default_serial());
        }
        state.selected = if self.selected_id.is_empty() {
            None
        } else {
            state.endpoints.iter().position(|e| e.id == self.selected_id)
        };
        state.refresh_serial_ports();
    }
}

/// OPC UA section (v7+).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OpcUaProject {
    #[serde(default)]
    pub selected_client_id: String,
    #[serde(default)]
    pub embedded: EmbeddedOpcUaConfig,
    #[serde(default)]
    pub clients: Vec<OpcUaClientSerde>,
}

impl Default for OpcUaProject {
    fn default() -> Self {
        let client = OpcUaClientSerde::default_plant();
        Self {
            selected_client_id: client.id.clone(),
            embedded: EmbeddedOpcUaConfig::default(),
            clients: vec![client],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OpcUaClientSerde {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default = "default_opcua_endpoint")]
    pub endpoint_url: String,
    #[serde(default = "default_opcua_policy")]
    pub security_policy: String,
    #[serde(default = "default_opcua_mode")]
    pub security_mode: String,
    #[serde(default = "default_opcua_auth")]
    pub auth: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default = "default_opcua_trust")]
    pub trust_server: String,
    #[serde(default)]
    pub pki_dir: String,
    #[serde(default = "default_opcua_interval")]
    pub subscription_interval_ms: u64,
    #[serde(default)]
    pub auto_connect: bool,
}

fn default_opcua_endpoint() -> String {
    "opc.tcp://127.0.0.1:4840".into()
}
fn default_opcua_policy() -> String {
    "None".into()
}
fn default_opcua_mode() -> String {
    "None".into()
}
fn default_opcua_auth() -> String {
    "anonymous".into()
}
fn default_opcua_trust() -> String {
    "strict".into()
}
fn default_opcua_interval() -> u64 {
    500
}

impl OpcUaClientSerde {
    pub fn default_plant() -> Self {
        Self {
            id: "plant".into(),
            label: "Plant UA".into(),
            endpoint_url: default_opcua_endpoint(),
            security_policy: default_opcua_policy(),
            security_mode: default_opcua_mode(),
            auth: default_opcua_auth(),
            username: String::new(),
            password: String::new(),
            trust_server: default_opcua_trust(),
            pki_dir: String::new(),
            subscription_interval_ms: default_opcua_interval(),
            auto_connect: false,
        }
    }

    fn to_panel(&self) -> OpcUaClient {
        OpcUaClient {
            id: self.id.clone(),
            label: self.label.clone(),
            endpoint_url: self.endpoint_url.clone(),
            security_policy: self.security_policy.clone(),
            security_mode: self.security_mode.clone(),
            auth: self.auth.clone(),
            username: self.username.clone(),
            password: self.password.clone(),
            trust_server: self.trust_server.clone(),
            pki_dir: self.pki_dir.clone(),
            subscription_interval_ms: self.subscription_interval_ms,
            auto_connect: self.auto_connect,
            draft_node_id: String::new(),
            draft_point: String::new(),
        }
    }

    fn from_panel(c: &OpcUaClient) -> Self {
        Self {
            id: c.id.clone(),
            label: c.label.clone(),
            endpoint_url: c.endpoint_url.clone(),
            security_policy: c.security_policy.clone(),
            security_mode: c.security_mode.clone(),
            auth: c.auth.clone(),
            username: c.username.clone(),
            password: c.password.clone(),
            trust_server: c.trust_server.clone(),
            pki_dir: c.pki_dir.clone(),
            subscription_interval_ms: c.subscription_interval_ms,
            auto_connect: c.auto_connect,
        }
    }
}

impl OpcUaProject {
    pub fn from_panel(state: &OpcUaPanelState) -> Self {
        let selected_client_id = state
            .selected()
            .map(|c| c.id.clone())
            .unwrap_or_default();
        Self {
            selected_client_id,
            embedded: state.embedded.clone(),
            clients: state.clients.iter().map(OpcUaClientSerde::from_panel).collect(),
        }
    }

    pub fn apply_to_panel(&self, state: &mut OpcUaPanelState) {
        state.embedded = self.embedded.clone();
        state.clients = if self.clients.is_empty() {
            vec![OpcUaClient::default_plant()]
        } else {
            self.clients.iter().map(OpcUaClientSerde::to_panel).collect()
        };
        state.selected = state
            .clients
            .iter()
            .position(|c| c.id == self.selected_client_id);
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
    /// Embedded hub broker (single per project). Absent on older files → defaults.
    #[serde(default)]
    pub embedded: EmbeddedBrokerConfig,
}

impl Default for MqttProject {
    fn default() -> Self {
        let client = MqttClientSerde::default_local();
        Self {
            selected_id: client.id.clone(),
            clients: vec![client],
            embedded: EmbeddedBrokerConfig::default(),
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
    /// Fresh random MQTT client id on each connect (TopicBench-style).
    #[serde(default)]
    pub unique_client_id: bool,
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
    "pb-local".into()
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
            unique_client_id: false,
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
            unique_client_id: c.unique_client_id,
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
            unique_client_id: self.unique_client_id,
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
            embedded: state.embedded.clone(),
        }
    }

    pub fn apply_to_panel(&self, state: &mut MqttPanelState) {
        state.clients = self.clients.iter().map(|c| c.to_client()).collect();
        if state.clients.is_empty() {
            state.clients.push(MqttClient::default_local());
        }
        state.ensure_unique_mqtt_client_ids();
        // Empty selected_id → list-only (no detail form).
        state.selected = if self.selected_id.is_empty() {
            None
        } else {
            state
                .clients
                .iter()
                .position(|c| c.id == self.selected_id)
        };
        state.embedded = self.embedded.clone();
        state.embedded.migrate_auth();
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

/// Move client-level Inputs/Outputs onto hub nodes, then clear the client copies.
pub fn migrate_client_binds_to_hubs(state: &mut MqttPanelState, hubs: &mut HubModel) {
    for client in &mut state.clients {
        let inputs: Vec<MqttInputBind> = client
            .parse_rules
            .iter()
            .map(|r| MqttInputBind {
                topic: r.topic.clone(),
                path: r.path.clone(),
                point: r.point.clone(),
            })
            .collect();
        let outputs: Vec<MqttOutputBind> = client
            .outputs
            .iter()
            .map(|o| MqttOutputBind {
                point: o.point.clone(),
                topic: o.topic.clone(),
                qos: o.qos,
                retain: o.retain,
                auto: false,
            })
            .collect();
        if inputs.is_empty() && outputs.is_empty() {
            continue;
        }
        hubs.adopt_legacy_mqtt_client_binds(&client.id, &inputs, &outputs);
        client.parse_rules.clear();
        client.outputs.clear();
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
                embedded: EmbeddedBrokerConfig::default(),
            },
            opcua: OpcUaProject::default(),
            http: HttpProject::default(),
            streams: StreamsProject::default(),
            hubs: None,
            settings: ProjectSettings::default(),
            layout: None,
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
    // PB-78: keep last non-empty hubs file as project.json.bak before overwrite.
    if path.is_file() {
        if let Ok(prev) = load(path) {
            if prev.hubs.as_ref().is_some_and(|h| !h.hubs.is_empty()) {
                let bak = path.with_extension("json.bak");
                let _ = std::fs::copy(path, &bak);
            }
        }
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
        panel.embedded.id = "hub".into();
        panel.embedded.name = "Plant hub".into();
        panel.embedded.enabled = true;
        panel.embedded.bind = "0.0.0.0".into();
        panel.embedded.port = 1884;
        panel.embedded.anonymous = false;
        panel.embedded.username = "ops".into();
        panel.embedded.password = "secret".into();
        let project = ProjectFile {
            version: PROJECT_VERSION,
            points: vec![PointBindingSerde {
                id: "mqtt_1".into(),
                source: "mqtt/local/pb/in".into(),
            }],
            next_point_seq: 2,
            mqtt: MqttProject::from_panel(&panel),
            opcua: OpcUaProject::default(),
            http: HttpProject::default(),
            streams: StreamsProject::default(),
            hubs: None,
            settings: ProjectSettings {
                name: "Plant".into(),
                description: "Roundtrip fixture".into(),
                auto_run_embedded_broker: false,
            },
            layout: None,
        };
        let text = serde_json::to_string_pretty(&project).unwrap();
        let back: ProjectFile = serde_json::from_str(&text).unwrap();
        assert_eq!(back.mqtt.clients[0].host, "broker.local");
        assert_eq!(back.points[0].id, "mqtt_1");
        assert!(back.mqtt.embedded.enabled);
        assert_eq!(back.mqtt.embedded.port, 1884);
        assert_eq!(back.mqtt.embedded.name, "Plant hub");
        assert_eq!(back.mqtt.embedded.id, "hub");
        assert!(!back.mqtt.embedded.anonymous);
        assert_eq!(back.mqtt.embedded.username, "ops");
    }

    #[test]
    fn dock_layout_roundtrip_and_seed() {
        let mut ui = UiState::default();
        ui.hidden_panes.push(DockPane::Traffic);
        let layout = DockLayout::from_ui(&ui);
        let project = ProjectFile {
            layout: Some(layout.clone()),
            ..ProjectFile::default()
        };
        let text = serde_json::to_string_pretty(&project).unwrap();
        let back: ProjectFile = serde_json::from_str(&text).unwrap();
        assert_eq!(back.layout.as_ref(), Some(&layout));

        // Old project without layout → seed flag, keep current ui tree.
        let old: ProjectFile = serde_json::from_str(
            r#"{"version":7,"mqtt":{"clients":[],"selected_id":""}}"#,
        )
        .unwrap();
        assert!(old.layout.is_none());
        let mut seeded = UiState::default();
        seeded.hidden_panes.push(DockPane::Files);
        assert!(apply_or_seed_layout(&old, &mut seeded));
        assert_eq!(seeded.hidden_panes, vec![DockPane::Files]);

        let mut restored = UiState::default();
        assert!(!apply_or_seed_layout(&back, &mut restored));
        assert_eq!(restored.hidden_panes, vec![DockPane::Traffic]);
    }

    #[test]
    fn embedded_defaults_on_old_mqtt_json() {
        let raw = r#"{"version":4,"mqtt":{"clients":[{"id":"local","label":"Local","host":"127.0.0.1","port":1883,"client_id":"pb","transport":"tcp"}],"selected_id":"local"}}"#;
        let back: ProjectFile = serde_json::from_str(raw).unwrap();
        assert!(!back.mqtt.embedded.enabled);
        assert_eq!(back.mqtt.embedded.bind, "0.0.0.0");
        assert_eq!(back.mqtt.embedded.port, 1883);
        assert_eq!(back.mqtt.embedded.id, "hub");
        assert_eq!(back.mqtt.embedded.name, "Hub broker");
        assert!(back.mqtt.embedded.anonymous);
        assert!(back.mqtt.embedded.username.is_empty());
    }

    #[test]
    fn embedded_migrates_legacy_users() {
        let raw = r#"{"version":5,"mqtt":{"clients":[],"selected_id":"","embedded":{"id":"hub","name":"Hub broker","enabled":true,"bind":"0.0.0.0","port":1883,"users":[{"username":"ops","password":"x"}]}}}"#;
        let file: ProjectFile = serde_json::from_str(raw).unwrap();
        let mut panel = MqttPanelState::default();
        file.mqtt.apply_to_panel(&mut panel);
        assert!(!panel.embedded.anonymous);
        assert_eq!(panel.embedded.username, "ops");
        assert_eq!(panel.embedded.password, "x");
        assert!(panel.embedded.users.is_empty());
    }

    #[test]
    fn connection_choices_list_embedded_first() {
        let panel = MqttPanelState::default();
        let choices = panel.connection_choices();
        assert_eq!(choices[0].0, "hub");
        assert!(choices[0].1.contains("Hub broker"));
        assert!(choices.iter().any(|(id, _)| id == "local"));
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
