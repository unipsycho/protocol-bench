//! Protocol Bench project file (JSON) — MQTT connections, hub Input/Output binds,
//! point id bindings, and dock layout (PB-95).
//!
//! Default path: `directories` → `…/ProtocolBench/project.json`
//! Theme + auto-run hubs stay in `ui.toml`. Hub diagram is mirrored here when saved.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::dock::DockPane;
use crate::hubs::{HubModel, HubProto, MqttInputBind, MqttOutputBind};
use crate::point_rows;
use crate::layout::LayoutNode;
use crate::mqtt_panel::{
    EmbeddedBrokerConfig, MqttClient, MqttOutput, MqttPanelState, MqttParseRule,
};
use crate::opcua_panel::{EmbeddedOpcUaConfig, OpcUaClient, OpcUaPanelState};
use crate::persist::UiState;
use pb_points::{is_address_path, is_short_point_id, PointBinding, PointStore};

/// v8: dock layout (tree / hidden / maximized) lives in the project (PB-95).
pub const PROJECT_VERSION: u32 = 11;

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
    /// LAS well-log endpoints. Absent on older files → defaults.
    #[serde(default)]
    pub las: LasProject,
    /// SNMP agent targets. Absent on older files → defaults.
    #[serde(default)]
    pub snmp: SnmpProject,
    /// Modbus connections (TCP / RTU, client / server). Absent on older files → defaults.
    #[serde(default)]
    pub modbus: ModbusProject,
    /// File-follow sources (CSV / JSON lines). Absent on older files → defaults.
    #[serde(default)]
    pub files: FilesProject,
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
            las: LasProject::default(),
            snmp: SnmpProject::default(),
            modbus: ModbusProject::default(),
            files: FilesProject::default(),
            hubs: None,
            settings: ProjectSettings::default(),
            layout: None,
        }
    }
}

/// LAS well-log file endpoints.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LasProject {
    #[serde(default)]
    pub endpoints: Vec<LasEndpointSerde>,
    #[serde(default)]
    pub selected_id: String,
}

impl Default for LasProject {
    fn default() -> Self {
        let ep = LasEndpointSerde::default_incoming();
        Self {
            selected_id: ep.id.clone(),
            endpoints: vec![ep],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LasEndpointSerde {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default = "default_las_role")]
    pub role: String,
    #[serde(default)]
    pub path: String,
    #[serde(default = "default_las_version")]
    pub version: String,
    #[serde(default = "default_las_follow")]
    pub follow: bool,
}

fn default_las_role() -> String {
    "incoming".into()
}
fn default_las_version() -> String {
    "2.0".into()
}
fn default_las_follow() -> bool {
    true
}

impl LasEndpointSerde {
    fn default_incoming() -> Self {
        Self {
            id: "well-a".into(),
            label: "Well A".into(),
            role: default_las_role(),
            path: String::new(),
            version: default_las_version(),
            follow: true,
        }
    }

    fn from_endpoint(e: &crate::las_panel::LasEndpoint) -> Self {
        Self {
            id: e.id.clone(),
            label: e.label.clone(),
            role: e.role.as_str().into(),
            path: e.path.clone(),
            version: e.version.clone(),
            follow: e.follow,
        }
    }

    fn to_endpoint(&self) -> crate::las_panel::LasEndpoint {
        crate::las_panel::LasEndpoint {
            id: self.id.clone(),
            label: if self.label.is_empty() {
                self.id.clone()
            } else {
                self.label.clone()
            },
            role: crate::las_panel::LasRole::from_str(&self.role),
            path: self.path.clone(),
            version: if self.version.is_empty() {
                default_las_version()
            } else {
                self.version.clone()
            },
            follow: self.follow,
        }
    }
}

impl LasProject {
    pub fn from_panel(state: &crate::las_panel::LasPanelState) -> Self {
        let endpoints = state
            .endpoints
            .iter()
            .map(LasEndpointSerde::from_endpoint)
            .collect();
        let selected_id = state
            .selected()
            .map(|e| e.id.clone())
            .unwrap_or_default();
        Self {
            endpoints,
            selected_id,
        }
    }

    pub fn apply_to_panel(&self, state: &mut crate::las_panel::LasPanelState) {
        state.endpoints = self.endpoints.iter().map(|e| e.to_endpoint()).collect();
        if state.endpoints.is_empty() {
            state
                .endpoints
                .push(crate::las_panel::LasEndpoint::default_incoming());
        }
        state.selected = if self.selected_id.is_empty() {
            None
        } else {
            state
                .endpoints
                .iter()
                .position(|e| e.id == self.selected_id)
        };
        state.pending_delete = None;
    }
}

/// Modbus connections.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModbusProject {
    #[serde(default)]
    pub connections: Vec<crate::modbus_panel::ModbusConn>,
    #[serde(default)]
    pub selected_id: String,
}

impl Default for ModbusProject {
    /// Plant sample: a tank PLC (TCP), a power meter (RTU) and the hub's own Modbus server.
    /// Shipped **disabled** so a fresh project never dials out; the demo project enables them.
    fn default() -> Self {
        use crate::hubs::ModbusBind;
        use crate::modbus_panel::{ModbusConn, ModbusParity, ModbusTransport};
        let b = |area: &str, address: u16, dtype: &str, point: &str| ModbusBind {
            area: area.into(),
            address,
            dtype: dtype.into(),
            order: "abcd".into(),
            unit: 0,
            point: point.into(),
        };
        let plc = ModbusConn {
            id: "plc".into(),
            label: "Tank PLC".into(),
            port: 5020,
            enabled: false,
            ..ModbusConn::default_client()
        };
        let meter = ModbusConn {
            id: "meter".into(),
            label: "Power meter (RTU)".into(),
            transport: ModbusTransport::Rtu,
            serial_port: "/dev/ttyUSB0".into(),
            parity: ModbusParity::Even,
            unit_id: 3,
            enabled: false,
            ..ModbusConn::default_client()
        };
        let hub = ModbusConn {
            id: "hub".into(),
            label: "Plant Modbus server".into(),
            port: 1502,
            enabled: false,
            map: vec![
                b("holding", 0, "f32", "modbus_1"),
                b("holding", 2, "f32", "modbus_2"),
                b("holding", 4, "u32", "modbus_3"),
                b("holding", 6, "bit3", "modbus_6"),
                b("coil", 0, "bool", "modbus_4"),
                b("coil", 1, "bool", "mqtt_1"),
                b("discrete", 0, "bool", "modbus_5"),
                b("input", 0, "u16", "modbus_7"),
                b("input", 2, "f32", "modbus_8"),
            ],
            ..ModbusConn::default_server()
        };
        Self {
            selected_id: String::new(),
            connections: vec![plc, meter, hub],
        }
    }
}

impl ModbusProject {
    pub fn from_panel(state: &crate::modbus_panel::ModbusPanelState) -> Self {
        Self {
            connections: state.conns.clone(),
            selected_id: state
                .selected
                .and_then(|i| state.conns.get(i))
                .map(|c| c.id.clone())
                .unwrap_or_default(),
        }
    }

    pub fn apply_to_panel(&self, state: &mut crate::modbus_panel::ModbusPanelState) {
        state.conns = self.connections.clone();
        if state.conns.is_empty() {
            state.conns.push(crate::modbus_panel::ModbusConn::default_client());
        }
        state.selected = if self.selected_id.is_empty() {
            None
        } else {
            state.conns.iter().position(|c| c.id == self.selected_id)
        };
        state.pending_delete = None;
    }
}

/// File-follow sources.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FilesProject {
    #[serde(default)]
    pub sources: Vec<crate::files_panel::FileSource>,
    #[serde(default)]
    pub selected_id: String,
}

impl Default for FilesProject {
    fn default() -> Self {
        let f = crate::files_panel::FileSource::default_csv();
        Self {
            selected_id: f.id.clone(),
            sources: vec![f],
        }
    }
}

impl FilesProject {
    pub fn from_panel(state: &crate::files_panel::FilesPanelState) -> Self {
        Self {
            sources: state.sources.clone(),
            selected_id: state
                .selected
                .and_then(|i| state.sources.get(i))
                .map(|c| c.id.clone())
                .unwrap_or_default(),
        }
    }

    pub fn apply_to_panel(&self, state: &mut crate::files_panel::FilesPanelState) {
        state.sources = self.sources.clone();
        if state.sources.is_empty() {
            state.sources.push(crate::files_panel::FileSource::default_csv());
        }
        state.selected = if self.selected_id.is_empty() {
            None
        } else {
            state.sources.iter().position(|c| c.id == self.selected_id)
        };
        state.pending_delete = None;
    }
}

/// SNMP agent targets.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SnmpProject {
    #[serde(default)]
    pub targets: Vec<SnmpTargetSerde>,
    #[serde(default)]
    pub selected_id: String,
}

impl Default for SnmpProject {
    fn default() -> Self {
        let t = SnmpTargetSerde::default_local();
        Self {
            selected_id: t.id.clone(),
            targets: vec![t],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SnmpTargetSerde {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default = "default_snmp_host")]
    pub host: String,
    #[serde(default = "default_snmp_port")]
    pub port: u16,
    #[serde(default = "default_snmp_version")]
    pub version: String,
    #[serde(default = "default_snmp_community")]
    pub community: String,
    #[serde(default = "default_snmp_timeout")]
    pub timeout_secs: u32,
}

fn default_snmp_host() -> String {
    "192.168.1.1".into()
}
fn default_snmp_port() -> u16 {
    161
}
fn default_snmp_version() -> String {
    "v2c".into()
}
fn default_snmp_community() -> String {
    "public".into()
}
fn default_snmp_timeout() -> u32 {
    3
}

impl SnmpTargetSerde {
    fn default_local() -> Self {
        Self {
            id: "switch".into(),
            label: "Core switch".into(),
            host: default_snmp_host(),
            port: default_snmp_port(),
            version: default_snmp_version(),
            community: default_snmp_community(),
            timeout_secs: default_snmp_timeout(),
        }
    }

    fn from_target(t: &crate::snmp_panel::SnmpTarget) -> Self {
        Self {
            id: t.id.clone(),
            label: t.label.clone(),
            host: t.host.clone(),
            port: t.port.parse().unwrap_or(161),
            version: t.version.as_str().into(),
            community: t.community.clone(),
            timeout_secs: t.timeout_secs.parse().unwrap_or(3),
        }
    }

    fn to_target(&self) -> crate::snmp_panel::SnmpTarget {
        crate::snmp_panel::SnmpTarget {
            id: self.id.clone(),
            label: if self.label.is_empty() {
                self.id.clone()
            } else {
                self.label.clone()
            },
            host: self.host.clone(),
            port: self.port.to_string(),
            version: crate::snmp_panel::SnmpVersion::from_str(&self.version),
            community: self.community.clone(),
            timeout_secs: self.timeout_secs.to_string(),
        }
    }
}

impl SnmpProject {
    pub fn from_panel(state: &crate::snmp_panel::SnmpPanelState) -> Self {
        let targets = state.targets.iter().map(SnmpTargetSerde::from_target).collect();
        let selected_id = state.selected().map(|t| t.id.clone()).unwrap_or_default();
        Self {
            targets,
            selected_id,
        }
    }

    pub fn apply_to_panel(&self, state: &mut crate::snmp_panel::SnmpPanelState) {
        state.targets = self.targets.iter().map(|t| t.to_target()).collect();
        if state.targets.is_empty() {
            state.targets.push(crate::snmp_panel::SnmpTarget::default_local());
        }
        state.selected = if self.selected_id.is_empty() {
            None
        } else {
            state.targets.iter().position(|t| t.id == self.selected_id)
        };
        state.pending_delete = None;
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

/// Promote Hub Input (and aggregator) binds into the point bus.
///
/// New Input binds leave `point` empty — this allocates `mqtt_N` / `opc_N` /
/// `http_N` / `stream_N` / `las_N` / `snmp_N` and writes the id back onto the bind. Existing short
/// ids are re-bound to their wire address so `project.points` and the live
/// store stay aligned after IPC.
/// Returns true when any hub bind field changed (caller should dirty-save).
pub fn materialize_hub_point_binds(hubs: &mut HubModel, store: &mut PointStore) -> bool {
    let mut dirty = false;
    for hub in &mut hubs.hubs {
        for port in hub.sources.iter_mut().chain(hub.outputs.iter_mut()) {
            match port.protocol {
                HubProto::Mqtt => {
                    let conn = port.mqtt_connection.clone();
                    for bind in &mut port.mqtt_inputs {
                        let topic = bind.topic.trim();
                        if topic.is_empty() {
                            continue;
                        }
                        let source =
                            point_rows::mqtt_bind_source(&conn, topic, bind.path.trim());
                        if bind.point.trim().is_empty() {
                            let id = store.ensure_id("mqtt", &source);
                            bind.point = id;
                            dirty = true;
                        } else {
                            store.bind(bind.point.trim(), &source);
                        }
                    }
                }
                HubProto::OpcUa => {
                    let conn = port.opcua_connection.clone();
                    for bind in &mut port.opcua_inputs {
                        let node = bind.node_id.trim();
                        if node.is_empty() {
                            continue;
                        }
                        let source = if conn.trim().is_empty() {
                            format!("opcua/{node}")
                        } else {
                            format!("opcua/{}/{node}", conn.trim())
                        };
                        if bind.point.trim().is_empty() {
                            // 3-char proto code (opcua is 5 — short ids are XXX_n).
                            let id = store.ensure_id("opc", &source);
                            bind.point = id;
                            dirty = true;
                        } else {
                            store.bind(bind.point.trim(), &source);
                        }
                    }
                }
                HubProto::Http => {
                    let conn = port.http_connection.clone();
                    for bind in &mut port.http_inputs {
                        let json_path = bind.json_path.trim();
                        if json_path.is_empty() {
                            continue;
                        }
                        let source = point_rows::http_bind_source(
                            &conn,
                            bind.path.trim(),
                            json_path,
                        );
                        if bind.point.trim().is_empty() {
                            let id = store.ensure_id("http", &source);
                            bind.point = id;
                            dirty = true;
                        } else {
                            store.bind(bind.point.trim(), &source);
                        }
                    }
                }
                HubProto::Streams => {
                    let endpoint = port.stream_endpoint.clone();
                    for bind in &mut port.stream_inputs {
                        let source =
                            point_rows::stream_bind_source(&endpoint, bind.field.trim());
                        if bind.point.trim().is_empty() {
                            let id = store.ensure_id("stream", &source);
                            bind.point = id;
                            dirty = true;
                        } else {
                            store.bind(bind.point.trim(), &source);
                        }
                    }
                }
                HubProto::Las => {
                    let endpoint = port.las_endpoint.clone();
                    for bind in &mut port.las_inputs {
                        let curve = bind.curve.trim();
                        if curve.is_empty() {
                            continue;
                        }
                        let source = point_rows::las_bind_source(&endpoint, curve);
                        if bind.point.trim().is_empty() {
                            let id = store.ensure_id("las", &source);
                            bind.point = id;
                            dirty = true;
                        } else {
                            store.bind(bind.point.trim(), &source);
                        }
                    }
                }
HubProto::Modbus => {
                    let conn = port.modbus_connection.clone();
                    for bind in &mut port.modbus_inputs {
                        let source = point_rows::modbus_bind_source(&conn, bind);
                        if bind.point.trim().is_empty() {
                            let id = store.ensure_id("modbus", &source);
                            bind.point = id;
                            dirty = true;
                        } else {
                            store.bind(bind.point.trim(), &source);
                        }
                    }
                }
                HubProto::Files => {
                    let src = port.file_source.clone();
                    for bind in &mut port.file_inputs {
                        let column = bind.column.trim();
                        if column.is_empty() {
                            continue;
                        }
                        let source = point_rows::file_bind_source(&src, column);
                        if bind.point.trim().is_empty() {
                            let id = store.ensure_id("file", &source);
                            bind.point = id;
                            dirty = true;
                        } else {
                            store.bind(bind.point.trim(), &source);
                        }
                    }
                }
                HubProto::Snmp => {
                    let target = port.snmp_target.clone();
                    for bind in &mut port.snmp_inputs {
                        let oid = bind.oid.trim();
                        if oid.is_empty() {
                            continue;
                        }
                        let source = point_rows::snmp_bind_source(&target, oid);
                        if bind.point.trim().is_empty() {
                            let id = store.ensure_id("snmp", &source);
                            bind.point = id;
                            dirty = true;
                        } else {
                            store.bind(bind.point.trim(), &source);
                        }
                    }
                }
            }
        }
        // Aggregator outputs are host points (`agg_N`) published by Outputs.
        for step in &hub.aggregators {
            let id = format!("agg_{}", step.id);
            let source = format!("agg/step/{}", step.id);
            store.bind(&id, &source);
        }
    }
    dirty
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hubs::HttpInputBind;

    #[test]
    fn materialize_hub_binds_allocates_and_lists() {
        let mut hubs = HubModel::default();
        // Start from an empty bind slate: the sample ships a pump-command bind on `mqtt_1`.
        for hub in &mut hubs.hubs {
            for port in hub.sources.iter_mut().chain(hub.outputs.iter_mut()) {
                port.mqtt_inputs.clear();
            }
        }
        hubs.add_hub();
        let hub = hubs.hubs[0].id;
        let src = hubs.hub(hub).unwrap().sources[0].id;
        if let Some(port) = hubs
            .hub_mut(hub)
            .and_then(|h| h.sources.iter_mut().find(|p| p.id == src))
        {
            port.protocol = HubProto::Mqtt;
            port.mqtt_connection = "Mosquitto".into();
            port.mqtt_inputs.clear(); // drop the sample's pump-command bind
            port.mqtt_inputs.push(MqttInputBind {
                topic: "plant/t".into(),
                path: String::new(),
                point: String::new(),
            });
            port.mqtt_inputs.push(MqttInputBind {
                topic: "plant/u".into(),
                path: "v".into(),
                point: "mqtt_9".into(),
            });
        }
        let mut store = PointStore::new();
        assert!(materialize_hub_point_binds(&mut hubs, &mut store));
        let port = hubs
            .hub(hub)
            .unwrap()
            .sources
            .iter()
            .find(|p| p.id == src)
            .unwrap();
        assert_eq!(port.mqtt_inputs[0].point, "mqtt_1");
        assert_eq!(port.mqtt_inputs[1].point, "mqtt_9");
        assert_eq!(
            store.source_for_id("mqtt_1"),
            Some("mqtt/Mosquitto/plant/t")
        );
        assert_eq!(
            store.source_for_id("mqtt_9"),
            Some("mqtt/Mosquitto/plant/u::v")
        );
        assert!(store.get("mqtt_1").is_some());
        assert!(store.get("mqtt_9").is_some());
        // Second pass is idempotent (no further bind edits).
        assert!(!materialize_hub_point_binds(&mut hubs, &mut store));
    }

    #[test]
    fn materialize_http_binds_allocates_http_n() {
        let mut hubs = HubModel::default();
        hubs.add_hub();
        let hub = hubs.hubs[0].id;
        let src = hubs.hub(hub).unwrap().sources[0].id;
        if let Some(port) = hubs
            .hub_mut(hub)
            .and_then(|h| h.sources.iter_mut().find(|p| p.id == src))
        {
            port.protocol = HubProto::Http;
            port.http_connection = "Api".into();
            port.http_inputs.push(HttpInputBind {
                path: "/v1/tank".into(),
                json_path: "data.amount".into(),
                point: String::new(),
            });
        }
        let mut store = PointStore::new();
        assert!(materialize_hub_point_binds(&mut hubs, &mut store));
        let port = hubs
            .hub(hub)
            .unwrap()
            .sources
            .iter()
            .find(|p| p.id == src)
            .unwrap();
        assert_eq!(port.http_inputs[0].point, "http_1");
        assert_eq!(
            store.source_for_id("http_1"),
            Some("http/Api/v1/tank::data.amount")
        );
        assert!(store.get("http_1").is_some());
        assert!(!materialize_hub_point_binds(&mut hubs, &mut store));
    }

    #[test]
    fn multi_client_roundtrip() {
        let mut panel = MqttPanelState::default();
        panel.clients[0].host = "broker.local".into();
        panel.embedded.id = "hub".into();
        panel.embedded.name = "Plant hub".into();
        panel.embedded.enabled = true;
        panel.embedded.bind = "0.0.0.0".into();
        panel.embedded.port = 1885;
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
            las: LasProject::default(),
            snmp: SnmpProject::default(),
            modbus: ModbusProject::default(),
            files: FilesProject::default(),
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
        assert_eq!(back.mqtt.embedded.port, 1885);
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
        assert_eq!(back.mqtt.embedded.port, 1885);
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

    #[test]
    fn modbus_and_files_roundtrip_and_materialize() {
        use crate::hubs::{FileInputBind, ModbusBind};
        let mut file = ProjectFile::default();
        assert_eq!(file.modbus.connections.len(), 3);
        file.modbus.connections[0].port = 1502;
        let json = serde_json::to_string(&file).unwrap();
        let back: ProjectFile = serde_json::from_str(&json).unwrap();
        assert_eq!(back.modbus, file.modbus);
        // Older files without the new sections still load.
        let old: ProjectFile = serde_json::from_str(r#"{"version":8}"#).unwrap();
        assert!(!old.files.sources.is_empty());

        let mut hubs = HubModel::default();
        let port = &mut hubs.hubs[0].sources[0];
        port.protocol = HubProto::Modbus;
        port.modbus_connection = "plc".into();
        port.modbus_inputs.clear();
        port.modbus_inputs.push(ModbusBind {
            area: "holding".into(),
            address: 100,
            dtype: "f32".into(),
            order: "cdab".into(),
            unit: 0,
            point: String::new(),
        });
        let port = &mut hubs.hubs[0].sources[1];
        port.protocol = HubProto::Files;
        port.file_source = "meter".into();
        port.file_inputs.push(FileInputBind { column: "temp".into(), point: String::new() });
        let mut store = PointStore::default();
        assert!(materialize_hub_point_binds(&mut hubs, &mut store));
        assert_eq!(hubs.hubs[0].sources[0].modbus_inputs[0].point, "modbus_1");
        assert!(hubs.hubs[0].sources[1].file_inputs[0].point.starts_with("file_"));
    }

    #[test]
    fn modbus_source_strings_match_the_service() {
        use crate::hubs::ModbusBind;
        use crate::point_rows::modbus_bind_source;
        let b = |area: &str, addr: u16, dt: &str, order: &str, unit: u8| ModbusBind {
            area: area.into(),
            address: addr,
            dtype: dt.into(),
            order: order.into(),
            unit,
            point: String::new(),
        };
        // Same literals asserted in pb-service `source_strings_match_the_ui_format`.
        assert_eq!(modbus_bind_source("plc", &b("holding", 100, "f32", "abcd", 0)), "modbus/plc::holding:100:f32");
        assert_eq!(modbus_bind_source("plc", &b("holding", 100, "f32", "cdab", 0)), "modbus/plc::holding:100:f32:cdab");
        assert_eq!(modbus_bind_source("plc", &b("coil", 3, "bool", "abcd", 7)), "modbus/plc/u7::coil:3:bool");
        assert_eq!(modbus_bind_source("", &b("input", 0, "u16", "abcd", 0)), "modbus::input:0:u16");
    }

    #[test]
    fn plant_sample_is_internally_consistent() {
        use crate::modbus_panel::{map_problems, ModbusRole};
        let hubs = HubModel::default();
        let modbus = ModbusProject::default();
        let conn = |id: &str| modbus.connections.iter().find(|c| c.id == id);
        // Every Modbus port names a real client connection; the Modbus hub names a real server.
        for hub in &hubs.hubs {
            for port in hub.sources.iter().chain(hub.outputs.iter()) {
                if port.protocol == HubProto::Modbus {
                    let c = conn(&port.modbus_connection).expect("modbus port connection exists");
                    assert_eq!(c.role, ModbusRole::Client, "{}", port.label);
                    for b in port.modbus_inputs.iter().chain(port.modbus_outputs.iter()) {
                        assert_eq!(b.problem(false), None, "{} {}", port.label, b.describe());
                    }
                }
            }
            if hub.protocol == HubProto::Modbus {
                let c = conn(&hub.modbus_server).expect("hub server exists");
                assert_eq!(c.role, ModbusRole::Server);
                assert!(map_problems(&c.map).is_empty(), "{:?}", map_problems(&c.map));
            }
        }
        // Every point the server map serves is produced by a bind somewhere in the sample.
        let mut hubs = hubs;
        let mut store = PointStore::new();
        materialize_hub_point_binds(&mut hubs, &mut store);
        let server = modbus.connections.iter().find(|c| c.role == ModbusRole::Server).unwrap();
        for entry in &server.map {
            assert!(store.source_for_id(&entry.point).is_some(), "served point {} has no source bind", entry.point);
        }
        // Sample connections ship disabled so a fresh project never dials out.
        assert!(modbus.connections.iter().all(|c| !c.enabled));
    }

    /// `PB_EXPORT_DEMO=/path/modbus-plant.json cargo test -p pb-ui export_demo -- --ignored`
    /// writes the Plant sample as a runnable project (connections enabled, TCP to the simulator).
    #[test]
    #[ignore]
    fn export_demo_project() {
        let Some(path) = std::env::var_os("PB_EXPORT_DEMO") else { return };
        let mut file = ProjectFile::default();
        file.hubs = Some(HubModel::default());
        for c in &mut file.modbus.connections {
            // The RTU meter needs a real serial line — leave it off in the demo.
            c.enabled = c.id != "meter";
        }
        file.settings.name = "Modbus plant demo".into();
        file.settings.description =
            "Tank PLC (Modbus TCP, run scripts/modbus-plant-sim.py) bridged to MQTT and a hub Modbus server on :1502".into();
        let text = serde_json::to_string_pretty(&file).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// Writes the two field-demo projects (`modbus-hub.json`, `edge-logger.json`) into the
    /// directory named by `PB_EXPORT_DEMOS`. Data files live in `<dir>/data` (see scripts/field-sim.py).
    #[test]
    #[ignore]
    fn export_field_demos() {
        use crate::files_panel::{FileParser, FileSource, FileStart};
        use crate::hubs::{FileInputBind, HubPort, LasInputBind, LasOutputBind, ModbusBind, MqttOutputBind, SnmpInputBind};
        use crate::modbus_panel::ModbusConn;

        let Some(dir) = std::env::var_os("PB_EXPORT_DEMOS") else { return };
        let dir = std::path::PathBuf::from(dir);
        let data = dir.join("data");
        let data_path = |f: &str| data.join(f).to_string_lossy().into_owned();
        let mb = |area: &str, address: u16, dtype: &str, order: &str, point: &str| {
            let mut b = ModbusBind {
                area: area.into(),
                address,
                dtype: dtype.into(),
                order: order.into(),
                unit: 0,
                point: point.into(),
            };
            if order.is_empty() {
                b.order = "abcd".into();
            }
            b
        };
        let meter_csv = FileSource {
            id: "meter".into(),
            label: "Meter log (CSV)".into(),
            path: data_path("meter.csv"),
            parser: FileParser::Csv,
            start_at: FileStart::End,
            ..FileSource::default_csv()
        };
        let switch = SnmpTargetSerde {
            id: "sw".into(),
            label: "Core switch (field-sim)".into(),
            host: "127.0.0.1".into(),
            port: 1161,
            version: "v2c".into(),
            community: "public".into(),
            timeout_secs: 2,
        };

        // ---------------- Modbus-hub ----------------
        let mut a = ProjectFile::default();
        a.settings.name = "Modbus-hub".into();
        a.settings.description = "Modbus hub: Tank PLC (Modbus TCP :5020) + meter CSV + SNMP switch in; \
            the hub serves every point as a Modbus server on :1502; a master write to coil 1 on :1502 \
            drives the PLC pump; the plant picture is logged to a time-indexed LAS file. \
            Run scripts/modbus-plant-sim.py and scripts/field-sim.py first."
            .into();
        a.next_point_seq = 30;
        a.modbus.connections = vec![
            ModbusConn { id: "plc".into(), label: "Tank PLC".into(), port: 5020, enabled: true, ..ModbusConn::default_client() },
            ModbusConn {
                id: "hub".into(),
                label: "Modbus-hub server".into(),
                port: 1502,
                enabled: true,
                map: vec![
                    mb("holding", 0, "f32", "cdab", "modbus_1"),
                    mb("holding", 2, "f32", "cdab", "modbus_2"),
                    mb("holding", 4, "u32", "abcd", "modbus_3"),
                    mb("holding", 6, "bit3", "abcd", "modbus_6"),
                    mb("holding", 8, "f32", "abcd", "file_1"),
                    mb("holding", 10, "f32", "abcd", "file_2"),
                    mb("holding", 12, "u32", "abcd", "snmp_1"),
                    mb("holding", 14, "u32", "abcd", "snmp_2"),
                    mb("coil", 0, "bool", "abcd", "modbus_4"),
                    mb("coil", 1, "bool", "abcd", "modbus_9"),
                    mb("discrete", 0, "bool", "abcd", "modbus_5"),
                ],
                ..ModbusConn::default_server()
            },
        ];
        a.modbus.selected_id = "plc".into();
        a.files.sources = vec![meter_csv.clone()];
        a.files.selected_id = "meter".into();
        a.snmp.targets = vec![switch.clone()];
        a.snmp.selected_id = "sw".into();
        a.las.endpoints = vec![LasEndpointSerde {
            id: "plant-log".into(),
            label: "Plant log (written)".into(),
            role: "outgoing".into(),
            path: data_path("plant-log.las"),
            version: "2.0".into(),
            follow: false,
        }];
        a.las.selected_id = "plant-log".into();
        let mut hubs = HubModel::sample();
        hubs.selected = Some(1);
        hubs.selection = vec![(1, None)];
        hubs.hubs = vec![crate::hubs::Hub {
            id: 1,
            name: "Modbus-hub".into(),
            protocol: HubProto::Modbus,
            sources: vec![
                {
                    let mut p = HubPort::new(2, HubProto::Modbus, "Tank PLC");
                    p.modbus_connection = "plc".into();
                    p.modbus_interval_secs = 1.0;
                    p.modbus_inputs = vec![
                        mb("holding", 100, "f32", "cdab", "modbus_1"),
                        mb("holding", 102, "f32", "cdab", "modbus_2"),
                        mb("holding", 110, "u32", "abcd", "modbus_3"),
                        mb("coil", 0, "bool", "abcd", "modbus_4"),
                        mb("discrete", 0, "bool", "abcd", "modbus_5"),
                        mb("holding", 120, "bit3", "abcd", "modbus_6"),
                    ];
                    p
                },
                {
                    let mut p = HubPort::new(3, HubProto::Files, "Meter CSV");
                    p.file_source = "meter".into();
                    p.file_inputs = vec![
                        FileInputBind { column: "volts".into(), point: "file_1".into() },
                        FileInputBind { column: "kw".into(), point: "file_2".into() },
                    ];
                    p
                },
                {
                    let mut p = HubPort::new(4, HubProto::Snmp, "Core switch");
                    p.snmp_target = "sw".into();
                    p.snmp_interval_secs = 2.0;
                    p.snmp_inputs = vec![
                        SnmpInputBind { oid: "1.3.6.1.2.1.1.3.0".into(), point: "snmp_1".into() },
                        SnmpInputBind { oid: "1.3.6.1.2.1.2.2.1.10.1".into(), point: "snmp_2".into() },
                    ];
                    p
                },
            ],
            outputs: vec![
                {
                    // Written by a master on :1502 (coil 1) → forwarded to the PLC pump coil.
                    let mut p = HubPort::new(5, HubProto::Modbus, "Pump command");
                    p.modbus_connection = "plc".into();
                    p.modbus_outputs = vec![mb("coil", 1, "bool", "abcd", "modbus_9")];
                    p
                },
                {
                    let mut p = HubPort::new(6, HubProto::Las, "Plant log");
                    p.las_endpoint = "plant-log".into();
                    p.las_index = "time".into();
                    p.las_outputs = vec![
                        LasOutputBind { point: "modbus_1".into(), curve: "LEVEL".into() },
                        LasOutputBind { point: "modbus_2".into(), curve: "TEMP".into() },
                        LasOutputBind { point: "file_2".into(), curve: "KW".into() },
                    ];
                    p
                },
            ],
            aggregators: vec![],
            calculations: vec![],
            at: None,
            modbus_server: "hub".into(),
            opcua_expose_unmapped: true,
            opcua_catalog: vec![],
        }];
        a.hubs = Some(hubs);
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(dir.join("modbus-hub.json"), serde_json::to_string_pretty(&a).unwrap()).unwrap();

        // ---------------- Edge-logger ----------------
        let mut b = ProjectFile::default();
        b.settings.name = "Edge-logger".into();
        b.settings.description = "MQTT hub (embedded broker :1885): a growing LAS well log + SNMP switch + meter CSV \
            in; MQTT topics out and a Modbus write to the Tank PLC. Run scripts/field-sim.py (and \
            scripts/modbus-plant-sim.py for the Modbus output)."
            .into();
        b.next_point_seq = 30;
        b.mqtt.embedded.enabled = true;
        b.mqtt.embedded.port = 1885;
        // Outputs publish through a client session; this one loops back into the hub's own broker.
        b.mqtt.clients = vec![MqttClientSerde {
            id: "local".into(),
            label: "Edge loopback".into(),
            port: 1885,
            client_id: "pb-edge-logger".into(),
            subscriptions: Vec::new(),
            auto_connect: true,
            ..MqttClientSerde::default_local()
        }];
        b.mqtt.selected_id = "local".into();
        b.modbus.connections = vec![ModbusConn {
            id: "plc".into(),
            label: "Tank PLC".into(),
            port: 5020,
            enabled: true,
            ..ModbusConn::default_client()
        }];
        b.modbus.selected_id = "plc".into();
        b.files.sources = vec![meter_csv];
        b.files.selected_id = "meter".into();
        b.snmp.targets = vec![switch];
        b.snmp.selected_id = "sw".into();
        b.las.endpoints = vec![
            LasEndpointSerde {
                id: "well-a".into(),
                label: "Well A (growing)".into(),
                role: "incoming".into(),
                path: data_path("well-a.las"),
                version: "2.0".into(),
                follow: true,
            },
            LasEndpointSerde {
                id: "well-copy".into(),
                label: "Well A copy (written)".into(),
                role: "outgoing".into(),
                path: data_path("well-a-copy.las"),
                version: "2.0".into(),
                follow: false,
            },
        ];
        b.las.selected_id = "well-a".into();
        let broker = "local".to_string();
        let mut hubs = HubModel::sample();
        hubs.selected = Some(1);
        hubs.selection = vec![(1, None)];
        let out = |point: &str, topic: &str| MqttOutputBind {
            point: point.into(),
            topic: topic.into(),
            qos: 0,
            retain: true,
            auto: false,
        };
        hubs.hubs = vec![crate::hubs::Hub {
            id: 1,
            name: "Edge-logger".into(),
            protocol: HubProto::Mqtt,
            sources: vec![
                {
                    let mut p = HubPort::new(2, HubProto::Las, "Well A log");
                    p.las_endpoint = "well-a".into();
                    p.las_inputs = vec![
                        LasInputBind { curve: "DEPT".into(), point: "las_1".into() },
                        LasInputBind { curve: "GR".into(), point: "las_2".into() },
                        LasInputBind { curve: "DT".into(), point: "las_3".into() },
                    ];
                    p
                },
                {
                    let mut p = HubPort::new(3, HubProto::Snmp, "Core switch");
                    p.snmp_target = "sw".into();
                    p.snmp_interval_secs = 2.0;
                    p.snmp_inputs = vec![SnmpInputBind { oid: "1.3.6.1.2.1.2.2.1.10.1".into(), point: "snmp_1".into() }];
                    p
                },
                {
                    let mut p = HubPort::new(4, HubProto::Files, "Meter CSV");
                    p.file_source = "meter".into();
                    p.file_inputs = vec![
                        FileInputBind { column: "volts".into(), point: "file_1".into() },
                        FileInputBind { column: "temp".into(), point: "file_2".into() },
                    ];
                    p
                },
            ],
            outputs: vec![
                {
                    let mut p = HubPort::new(5, HubProto::Mqtt, "Edge topics");
                    p.mqtt_connection = broker;
                    p.mqtt_outputs = vec![
                        out("las_1", "edge/well-a/depth"),
                        out("las_2", "edge/well-a/gr"),
                        out("las_3", "edge/well-a/dt"),
                        out("snmp_1", "edge/switch/in_octets"),
                        out("file_1", "edge/meter/volts"),
                    ];
                    p
                },
                {
                    let mut p = HubPort::new(6, HubProto::Las, "Log copy");
                    p.las_endpoint = "well-copy".into();
                    p.las_index = "depth".into();
                    p.las_outputs = vec![
                        LasOutputBind { point: "las_1".into(), curve: "DEPT".into() },
                        LasOutputBind { point: "las_2".into(), curve: "GR".into() },
                    ];
                    p
                },
                {
                    // Meter temperature → PLC holding register (e.g. an external setpoint).
                    let mut p = HubPort::new(7, HubProto::Modbus, "PLC setpoint");
                    p.modbus_connection = "plc".into();
                    p.modbus_outputs = vec![mb("holding", 130, "f32", "cdab", "file_2")];
                    p
                },
            ],
            aggregators: vec![],
            calculations: vec![],
            at: None,
            modbus_server: String::new(),
            opcua_expose_unmapped: true,
            opcua_catalog: vec![],
        }];
        b.hubs = Some(hubs);
        std::fs::write(dir.join("edge-logger.json"), serde_json::to_string_pretty(&b).unwrap()).unwrap();
    }

}

#[cfg(test)]
mod sample_files_tests {
    use super::*;

    /// Every project in the sibling samples folder must parse as a `ProjectFile`.
    #[test]
    fn sample_projects_parse() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../protocol-bench-projects");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return; // samples live outside this repo; skip when absent
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let value: serde_json::Value = serde_json::from_str(&text).unwrap();
            let parsed: Result<ProjectFile, _> = serde_json::from_value(value);
            assert!(parsed.is_ok(), "{}: {:?}", path.display(), parsed.err());
        }
    }
}
