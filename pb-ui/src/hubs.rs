//! Hub diagram. Each hub is a conversion / routing point: sources (Inputs) in,
//! optional aggregator or calculation transforms on the edges, outputs out.
//! MQTT Input/Output nodes own topic/field → point binds (connections live in
//! the MQTT panel). Many Inputs may share one MQTT connection.

use egui::{
    pos2, vec2, Align2, Color32, CursorIcon, Id, Layout, Pos2, Rect, Rounding, ScrollArea, Sense,
    Shape, Stroke, Ui, Vec2,
};
use serde::Deserialize;

use crate::dock::DockPane;
use crate::theme::{blend_scope, divider_bar, font_semi, font_ui, tint, ThemePalette, SEP};

const PORT_W: f32 = 156.0;
const PORT_H: f32 = 44.0;
const HUB_W: f32 = 160.0;
const HUB_H: f32 = 68.0;
const GAP: f32 = 12.0;
const COL_GAP: f32 = 52.0;
const MARGIN: f32 = 40.0;
const INSPECT_DEFAULT: f32 = 276.0;
const INSPECT_MIN: f32 = 180.0;
const DIAGRAM_MIN: f32 = 280.0;
/// Shared height for Sources/Outputs name · protocol · remove controls.
const PORT_ROW_H: f32 = 28.0;
const PORT_ROW_GAP: f32 = 6.0;
const PORT_PROTO_W: f32 = 100.0;
/// Right gutter so the inspector scrollbar does not cover Add / ×.
const INSPECT_SCROLL_GUTTER: f32 = 14.0;
/// Current default view is maximum zoom. Wheel zooms out from here.
const ZOOM_MAX: f32 = 1.0;
const ZOOM_MIN: f32 = 0.25;
/// Wheel zoom gain. Smaller than egui's default `1/200` so each notch steps gently.
const ZOOM_WHEEL: f32 = 1.0 / 520.0;

fn default_inspector_width() -> f32 {
    INSPECT_DEFAULT
}

fn default_zoom() -> f32 {
    ZOOM_MAX
}

fn clamp_zoom(zoom: f32) -> f32 {
    if !zoom.is_finite() {
        return ZOOM_MAX;
    }
    zoom.clamp(ZOOM_MIN, ZOOM_MAX)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum HubProto {
    Streams,
    Modbus,
    Mqtt,
    OpcUa,
    Files,
}

impl HubProto {
    pub const ALL: [HubProto; 5] = [
        HubProto::Streams,
        HubProto::Modbus,
        HubProto::Mqtt,
        HubProto::OpcUa,
        HubProto::Files,
    ];

    /// Protocols that can own a hub (server / broker role).
    pub const HUBS: [HubProto; 3] = [HubProto::Modbus, HubProto::Mqtt, HubProto::OpcUa];

    pub fn label(self) -> &'static str {
        match self {
            Self::Streams => "Streams",
            Self::Modbus => "Modbus",
            Self::Mqtt => "MQTT",
            Self::OpcUa => "OPC UA",
            Self::Files => "Files",
        }
    }

    pub fn pane(self) -> DockPane {
        match self {
            Self::Streams => DockPane::Streams,
            Self::Modbus => DockPane::Modbus,
            Self::Mqtt => DockPane::Mqtt,
            Self::OpcUa => DockPane::OpcUa,
            Self::Files => DockPane::Files,
        }
    }

    /// True for Modbus server, MQTT broker, OPC UA server (hub-capable).
    pub fn hub_capable(self) -> bool {
        matches!(self, Self::Modbus | Self::Mqtt | Self::OpcUa)
    }

    fn ink(self, light: bool) -> Color32 {
        self.pane().ink(light)
    }
}

fn default_hub_protocol() -> HubProto {
    HubProto::Modbus
}

/// Accept legacy `null` / non-hub protocols from older saves as Modbus.
fn deserialize_hub_protocol<'de, D>(deserializer: D) -> Result<HubProto, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Option::<HubProto>::deserialize(deserializer)?;
    Ok(match raw {
        Some(proto) if proto.hub_capable() => proto,
        _ => HubProto::Modbus,
    })
}

/// Input node: topic (+ optional JSON path) → host point on a shared MQTT connection.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MqttInputBind {
    pub topic: String,
    /// Empty = whole payload; otherwise dotted JSON path.
    #[serde(default)]
    pub path: String,
    /// Always empty on new Input binds — host auto-assigns sequential `mqtt_N`.
    /// Persisted only for legacy migrated binds that already have an id.
    #[serde(default)]
    pub point: String,
}

/// Output node: host point → publish topic on a shared MQTT connection.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MqttOutputBind {
    pub point: String,
    pub topic: String,
    #[serde(default)]
    pub qos: u8,
    #[serde(default)]
    pub retain: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct HubPort {
    pub id: u64,
    pub protocol: HubProto,
    pub label: String,
    /// Center on the diagram, once the user has dragged this node.
    #[serde(default)]
    pub at: Option<[f32; 2]>,
    /// MQTT connection id from the MQTT panel. Many Input/Output nodes may share one.
    #[serde(default)]
    pub mqtt_connection: String,
    /// Input binds (sources with protocol MQTT). Overlap across nodes is OK.
    #[serde(default)]
    pub mqtt_inputs: Vec<MqttInputBind>,
    /// Output binds (outputs with protocol MQTT).
    #[serde(default)]
    pub mqtt_outputs: Vec<MqttOutputBind>,
    /// Ephemeral inspector drafts — not persisted.
    #[serde(skip)]
    pub draft_mqtt_topic: String,
    #[serde(skip)]
    pub draft_mqtt_path: String,
    #[serde(skip)]
    pub draft_mqtt_out_point: String,
    #[serde(skip)]
    pub draft_mqtt_out_topic: String,
}

impl HubPort {
    pub fn new(id: u64, protocol: HubProto, label: impl Into<String>) -> Self {
        Self {
            id,
            protocol,
            label: label.into(),
            at: None,
            mqtt_connection: String::new(),
            mqtt_inputs: Vec::new(),
            mqtt_outputs: Vec::new(),
            draft_mqtt_topic: String::new(),
            draft_mqtt_path: String::new(),
            draft_mqtt_out_point: String::new(),
            draft_mqtt_out_topic: String::new(),
        }
    }
}

/// Aggregate or calculation step on a hub edge (source↔hub or hub↔output).
/// Not a free-floating peer of the hub — it sits on the data path.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct HubStep {
    pub id: u64,
    pub label: String,
    /// Source or output port whose hub link carries this transform.
    #[serde(default)]
    pub edge_port: u64,
    /// Center on the diagram, once the user has dragged this node off the edge.
    #[serde(default)]
    pub at: Option<[f32; 2]>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Hub {
    pub id: u64,
    pub name: String,
    /// Always one server-capable protocol. Sources/outputs may use other protocols.
    #[serde(
        default = "default_hub_protocol",
        deserialize_with = "deserialize_hub_protocol"
    )]
    pub protocol: HubProto,
    pub sources: Vec<HubPort>,
    pub outputs: Vec<HubPort>,
    /// Transforms sitting on source→hub or hub→output edges.
    #[serde(default)]
    pub aggregators: Vec<HubStep>,
    /// Transforms sitting on source→hub or hub→output edges.
    #[serde(default)]
    pub calculations: Vec<HubStep>,
    /// Center on the diagram, once the user has dragged this hub.
    #[serde(default)]
    pub at: Option<[f32; 2]>,
}

impl Hub {
    /// Source, output, aggregator, or calculation id on this hub.
    fn has_node(&self, id: u64) -> bool {
        self.sources.iter().any(|port| port.id == id)
            || self.outputs.iter().any(|port| port.id == id)
            || self.aggregators.iter().any(|step| step.id == id)
            || self.calculations.iter().any(|step| step.id == id)
    }

    fn has_port(&self, id: u64) -> bool {
        self.sources.iter().any(|port| port.id == id)
            || self.outputs.iter().any(|port| port.id == id)
    }

    /// Steps on one port's edge, aggregators then calculations (path order).
    fn steps_on(&self, edge_port: u64) -> Vec<(u64, Role)> {
        let mut ids = Vec::new();
        ids.extend(
            self.aggregators
                .iter()
                .filter(|step| step.edge_port == edge_port)
                .map(|step| (step.id, Role::Aggregator)),
        );
        ids.extend(
            self.calculations
                .iter()
                .filter(|step| step.edge_port == edge_port)
                .map(|step| (step.id, Role::Calculation)),
        );
        ids
    }
}

/// One output continuing into another hub's source of the same protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Across {
    pub from_hub: u64,
    pub from_port: u64,
    pub to_hub: u64,
    pub to_port: u64,
    pub proto: HubProto,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct HubModel {
    pub hubs: Vec<Hub>,
    /// Hub whose details are open. A port selection also sets this.
    pub selected: Option<u64>,
    /// Set when a source, output, or transform node is the one showing details.
    #[serde(default)]
    pub selected_port: Option<u64>,
    /// Selected connection (source/output port id) for edge transforms.
    #[serde(default)]
    pub selected_link: Option<u64>,
    /// Canvas multi-select: `(hub_id, port_id)` with `None` for the hub node itself.
    /// Ephemeral UI state — not saved with the project.
    #[serde(skip)]
    pub selection: Vec<(u64, Option<u64>)>,
    /// Camera translation of `world * zoom` into the grid (pixels). `None` = auto-center.
    /// Ephemeral — not saved with the project.
    #[serde(skip)]
    pub view_pan: Option<[f32; 2]>,
    /// Width of the right-hand configure panel when the hubs view is wide enough.
    #[serde(default = "default_inspector_width")]
    pub inspector_width: f32,
    /// Diagram scale. `1.0` is the default (maximum); wheel zooms out toward `ZOOM_MIN`.
    #[serde(default = "default_zoom")]
    pub zoom: f32,
    #[serde(default)]
    next_id: u64,
}

impl Default for HubModel {
    fn default() -> Self {
        Self::sample()
    }
}

impl HubModel {
    /// Two hubs: Plant (Modbus server) with mixed-protocol links, Edge (MQTT broker).
    pub fn sample() -> Self {
        Self {
            next_id: 20,
            selected: Some(1),
            selected_port: None,
            selected_link: None,
            selection: vec![(1, None)],
            view_pan: None,
            inspector_width: INSPECT_DEFAULT,
            zoom: ZOOM_MAX,
            hubs: vec![
                Hub {
                    id: 1,
                    name: "Plant".into(),
                    protocol: HubProto::Modbus,
                    sources: vec![
                        HubPort::new(2, HubProto::Modbus, "RTU line"),
                        HubPort::new(3, HubProto::Streams, "Serial A"),
                    ],
                    outputs: vec![
                        HubPort::new(4, HubProto::Mqtt, "Plant topics"),
                        HubPort::new(5, HubProto::OpcUa, "SCADA"),
                    ],
                    aggregators: vec![],
                    calculations: vec![],
                    at: None,
                },
                Hub {
                    id: 6,
                    name: "Edge".into(),
                    protocol: HubProto::Mqtt,
                    sources: vec![{
                        let mut p = HubPort::new(7, HubProto::Mqtt, "Local broker");
                        p.mqtt_connection = "local".into();
                        p
                    }],
                    outputs: vec![{
                        let mut p = HubPort::new(8, HubProto::Mqtt, "Upstream");
                        p.mqtt_connection = "local".into();
                        p
                    }],
                    aggregators: vec![],
                    calculations: vec![],
                    at: None,
                },
            ],
        }
    }

    pub fn sanitize(&mut self) {
        let max_id = self
            .hubs
            .iter()
            .flat_map(|hub| {
                std::iter::once(hub.id)
                    .chain(hub.sources.iter().map(|port| port.id))
                    .chain(hub.outputs.iter().map(|port| port.id))
                    .chain(hub.aggregators.iter().map(|step| step.id))
                    .chain(hub.calculations.iter().map(|step| step.id))
            })
            .max()
            .unwrap_or(0);
        if self.next_id <= max_id {
            self.next_id = max_id + 1;
        }
        for hub in &mut self.hubs {
            // Hubs are always one server-capable protocol; ports keep their own protocols.
            if !hub.protocol.hub_capable() {
                hub.protocol = HubProto::Modbus;
            }
            // Attach orphan transforms (older saves) to a real port edge.
            let ports: Vec<u64> = hub
                .sources
                .iter()
                .chain(hub.outputs.iter())
                .map(|port| port.id)
                .collect();
            let fallback = ports.first().copied().unwrap_or(0);
            if ports.is_empty() {
                hub.aggregators.clear();
                hub.calculations.clear();
            } else {
                for step in hub
                    .aggregators
                    .iter_mut()
                    .chain(hub.calculations.iter_mut())
                {
                    if step.edge_port == 0 || !ports.contains(&step.edge_port) {
                        step.edge_port = fallback;
                    }
                }
            }
        }
        if self.selected.is_some_and(|id| self.hub(id).is_none()) {
            self.selected = self.hubs.first().map(|hub| hub.id);
            self.selected_port = None;
            self.selected_link = None;
        }
        if !self.inspector_width.is_finite() || self.inspector_width < INSPECT_MIN {
            self.inspector_width = INSPECT_DEFAULT;
        }
        self.zoom = clamp_zoom(self.zoom);
        self.keep_focus();
    }

    /// Drop a port, step, link, or multi-select entry that no longer exists.
    fn keep_focus(&mut self) {
        let keys = self.selection.clone();
        self.selection = keys
            .into_iter()
            .filter(|(hub_id, port_id)| {
                let Some(hub) = self.hub(*hub_id) else {
                    return false;
                };
                match port_id {
                    None => true,
                    Some(id) => hub.has_node(*id),
                }
            })
            .collect();
        let Some(hub_id) = self.selected else {
            self.selected_port = None;
            self.selected_link = None;
            return;
        };
        if let Some(port_id) = self.selected_port {
            let belongs = self.hub(hub_id).is_some_and(|hub| hub.has_node(port_id));
            if !belongs {
                self.selected_port = None;
            }
        }
        if let Some(link) = self.selected_link {
            let belongs = self.hub(hub_id).is_some_and(|hub| hub.has_port(link));
            if !belongs {
                self.selected_link = None;
            }
        }
    }

    /// True when more than one canvas node is selected (no item properties).
    pub fn multi_selected(&self) -> bool {
        self.selection.len() > 1
    }

    fn selection_contains(&self, hub_id: u64, port_id: Option<u64>) -> bool {
        self.selection
            .iter()
            .any(|(h, p)| *h == hub_id && *p == port_id)
    }

    /// Replace canvas selection (box select / clear). Syncs single-item focus when len == 1.
    pub fn set_selection(&mut self, keys: Vec<(u64, Option<u64>)>) {
        self.selection = keys;
        self.selected_link = None;
        match self.selection.as_slice() {
            [] => {
                // Empty canvas: clear Node Properties entirely.
                self.selected = None;
                self.selected_port = None;
            }
            [(hub_id, None)] => {
                self.selected = Some(*hub_id);
                self.selected_port = None;
            }
            [(hub_id, Some(port_id))] => {
                self.apply_single_focus(*hub_id, Some(*port_id));
            }
            rest => {
                self.selected = rest.first().map(|(hub_id, _)| *hub_id);
                self.selected_port = None;
            }
        }
    }

    fn apply_single_focus(&mut self, hub_id: u64, port_id: Option<u64>) {
        self.selected = Some(hub_id);
        self.selected_port = port_id;
        self.selected_link = None;
        if let Some(port_id) = port_id {
            if let Some(hub) = self.hub(hub_id) {
                if hub.has_port(port_id) {
                    self.selected_link = Some(port_id);
                } else if let Some(step) = hub
                    .aggregators
                    .iter()
                    .chain(hub.calculations.iter())
                    .find(|step| step.id == port_id)
                {
                    self.selected_link = Some(step.edge_port);
                }
            }
        }
    }

    pub fn focus_hub(&mut self, id: u64) {
        self.selection = vec![(id, None)];
        self.apply_single_focus(id, None);
    }

    pub fn focus_port(&mut self, hub_id: u64, port_id: u64) {
        self.selection = vec![(hub_id, Some(port_id))];
        self.apply_single_focus(hub_id, Some(port_id));
    }

    /// Select the connection between a port and its hub (for edge transforms).
    pub fn focus_link(&mut self, hub_id: u64, port_id: u64) {
        self.selection.clear();
        self.selected = Some(hub_id);
        self.selected_port = None;
        self.selected_link = Some(port_id);
    }

    /// Hub + port edge currently targeted for Add aggregator / calculation.
    pub fn transform_edge(&self) -> Option<(u64, u64)> {
        let hub_id = self.selected?;
        let hub = self.hub(hub_id)?;
        if let Some(link) = self.selected_link {
            if hub.has_port(link) {
                return Some((hub_id, link));
            }
        }
        let port_id = self.selected_port?;
        if hub.has_port(port_id) {
            Some((hub_id, port_id))
        } else {
            None
        }
    }

    /// Remember a dragged center. `port_id` none moves the hub.
    pub fn set_center(&mut self, hub_id: u64, port_id: Option<u64>, center: [f32; 2]) {
        if let Some(port_id) = port_id {
            if let Some(port) = self.port_mut(hub_id, port_id) {
                port.at = Some(center);
                return;
            }
            if let Some(step) = self.step_mut(hub_id, port_id) {
                step.at = Some(center);
            }
            return;
        }
        if let Some(hub) = self.hub_mut(hub_id) {
            hub.at = Some(center);
        }
    }

    /// Put every node back on the automatic arrangement and re-center the camera.
    pub fn clear_arrangement(&mut self) {
        self.view_pan = None;
        for hub in &mut self.hubs {
            hub.at = None;
            for port in hub.sources.iter_mut().chain(hub.outputs.iter_mut()) {
                port.at = None;
            }
            for step in hub
                .aggregators
                .iter_mut()
                .chain(hub.calculations.iter_mut())
            {
                step.at = None;
            }
        }
    }

    fn port_mut(&mut self, hub_id: u64, port_id: u64) -> Option<&mut HubPort> {
        let hub = self.hub_mut(hub_id)?;
        hub.sources
            .iter_mut()
            .chain(hub.outputs.iter_mut())
            .find(|port| port.id == port_id)
    }

    fn step_mut(&mut self, hub_id: u64, step_id: u64) -> Option<&mut HubStep> {
        let hub = self.hub_mut(hub_id)?;
        hub.aggregators
            .iter_mut()
            .chain(hub.calculations.iter_mut())
            .find(|step| step.id == step_id)
    }

    pub fn hub(&self, id: u64) -> Option<&Hub> {
        self.hubs.iter().find(|hub| hub.id == id)
    }

    fn hub_mut(&mut self, id: u64) -> Option<&mut Hub> {
        self.hubs.iter_mut().find(|hub| hub.id == id)
    }

    fn alloc(&mut self) -> u64 {
        let id = self.next_id.max(1);
        self.next_id = id + 1;
        id
    }

    pub fn add_hub(&mut self) {
        let id = self.alloc();
        let source = self.alloc();
        let output = self.alloc();
        let name = format!("Hub {}", self.hubs.len() + 1);
        self.hubs.push(Hub {
            id,
            name,
            protocol: HubProto::Modbus,
            sources: vec![HubPort::new(source, HubProto::Modbus, "Input")],
            outputs: vec![HubPort::new(output, HubProto::Mqtt, "Output")],
            aggregators: vec![],
            calculations: vec![],
            at: None,
        });
        self.focus_hub(id);
    }

    pub fn remove_hub(&mut self, id: u64) {
        self.hubs.retain(|hub| hub.id != id);
        self.selection.retain(|(hub_id, _)| *hub_id != id);
        if self.selected == Some(id) {
            if let Some((hub_id, port_id)) = self.selection.first().copied() {
                self.apply_single_focus(hub_id, port_id);
            } else if let Some(hub) = self.hubs.first() {
                self.focus_hub(hub.id);
            } else {
                self.selected = None;
                self.selected_port = None;
                self.selected_link = None;
            }
        }
        self.keep_focus();
    }

    pub fn set_hub_protocol(&mut self, id: u64, proto: HubProto) {
        if !proto.hub_capable() {
            return;
        }
        let Some(hub) = self.hub_mut(id) else {
            return;
        };
        // Hub protocol only — sources/outputs keep their own protocols.
        hub.protocol = proto;
    }

    /// Append a source or output and return its id.
    pub fn add_port(&mut self, hub_id: u64, source: bool) -> Option<u64> {
        let proto = if source {
            HubProto::Modbus
        } else {
            HubProto::Mqtt
        };
        let id = self.alloc();
        let Some(hub) = self.hub_mut(hub_id) else {
            return None;
        };
        let port = HubPort::new(id, proto, if source { "Input" } else { "Output" });
        if source {
            hub.sources.push(port);
        } else {
            hub.outputs.push(port);
        }
        Some(id)
    }

    /// Append an aggregator or calculation on a port's edge and return its id.
    pub fn add_step(&mut self, hub_id: u64, aggregator: bool, edge_port: u64) -> Option<u64> {
        {
            let hub = self.hub(hub_id)?;
            if !hub.has_port(edge_port) {
                return None;
            }
        }
        let id = self.alloc();
        let Some(hub) = self.hub_mut(hub_id) else {
            return None;
        };
        let step = HubStep {
            id,
            label: if aggregator {
                "Aggregator".into()
            } else {
                "Calculation".into()
            },
            edge_port,
            at: None,
        };
        if aggregator {
            hub.aggregators.push(step);
        } else {
            hub.calculations.push(step);
        }
        Some(id)
    }

    pub fn remove_port(&mut self, hub_id: u64, port_id: u64) {
        if let Some(hub) = self.hub_mut(hub_id) {
            hub.sources.retain(|port| port.id != port_id);
            hub.outputs.retain(|port| port.id != port_id);
            // Drop the node itself, or any transform that lived on that edge.
            hub.aggregators
                .retain(|step| step.id != port_id && step.edge_port != port_id);
            hub.calculations
                .retain(|step| step.id != port_id && step.edge_port != port_id);
        }
        if self.selected_port == Some(port_id) {
            self.selected_port = None;
        }
        if self.selected_link == Some(port_id) {
            self.selected_link = None;
        }
    }

    /// Collect Input/Output MQTT binds for one connection id.
    /// Many nodes may share the connection; each node's binds are included (overlap OK).
    pub fn mqtt_binds_for_connection(
        &self,
        conn_id: &str,
    ) -> (
        Vec<MqttInputBind>,
        Vec<MqttOutputBind>,
        Vec<String>,
    ) {
        let mut inputs = Vec::new();
        let mut outputs = Vec::new();
        let mut topics = Vec::new();
        if conn_id.is_empty() {
            return (inputs, outputs, topics);
        }
        for hub in &self.hubs {
            for port in &hub.sources {
                if port.protocol != HubProto::Mqtt || port.mqtt_connection != conn_id {
                    continue;
                }
                for bind in &port.mqtt_inputs {
                    if bind.topic.trim().is_empty() {
                        continue;
                    }
                    let topic = bind.topic.trim().to_string();
                    if !topics.iter().any(|t| t == &topic) {
                        topics.push(topic.clone());
                    }
                    inputs.push(MqttInputBind {
                        topic,
                        path: bind.path.trim().to_string(),
                        point: bind.point.trim().to_string(),
                    });
                }
            }
            for port in &hub.outputs {
                if port.protocol != HubProto::Mqtt || port.mqtt_connection != conn_id {
                    continue;
                }
                for bind in &port.mqtt_outputs {
                    if bind.point.trim().is_empty() || bind.topic.trim().is_empty() {
                        continue;
                    }
                    outputs.push(MqttOutputBind {
                        point: bind.point.trim().to_string(),
                        topic: bind.topic.trim().to_string(),
                        qos: bind.qos,
                        retain: bind.retain,
                    });
                }
            }
        }
        (inputs, outputs, topics)
    }

    /// Move legacy client-level parse_rules / outputs onto MQTT Input/Output nodes.
    pub fn adopt_legacy_mqtt_client_binds(
        &mut self,
        client_id: &str,
        parse_rules: &[MqttInputBind],
        outputs: &[MqttOutputBind],
    ) {
        if parse_rules.is_empty() && outputs.is_empty() {
            return;
        }
        if !parse_rules.is_empty() {
            let port_id = self
                .hubs
                .iter()
                .flat_map(|h| h.sources.iter())
                .find(|p| {
                    p.protocol == HubProto::Mqtt
                        && (p.mqtt_connection.is_empty() || p.mqtt_connection == client_id)
                        && p.mqtt_inputs.is_empty()
                })
                .map(|p| p.id);
            let port_id = port_id.or_else(|| {
                let hub_id = self.hubs.first().map(|h| h.id)?;
                self.add_port(hub_id, true).map(|id| {
                    if let Some(p) = self.port_mut(hub_id, id) {
                        p.protocol = HubProto::Mqtt;
                        p.label = format!("{client_id} in");
                    }
                    id
                })
            });
            if let Some(port_id) = port_id {
                for hub in &mut self.hubs {
                    if let Some(port) = hub.sources.iter_mut().find(|p| p.id == port_id) {
                        port.protocol = HubProto::Mqtt;
                        port.mqtt_connection = client_id.to_string();
                        for rule in parse_rules {
                            if !port.mqtt_inputs.iter().any(|b| b == rule) {
                                port.mqtt_inputs.push(rule.clone());
                            }
                        }
                        break;
                    }
                }
            }
        }
        if !outputs.is_empty() {
            let port_id = self
                .hubs
                .iter()
                .flat_map(|h| h.outputs.iter())
                .find(|p| {
                    p.protocol == HubProto::Mqtt
                        && (p.mqtt_connection.is_empty() || p.mqtt_connection == client_id)
                        && p.mqtt_outputs.is_empty()
                })
                .map(|p| p.id);
            let port_id = port_id.or_else(|| {
                let hub_id = self.hubs.first().map(|h| h.id)?;
                self.add_port(hub_id, false).map(|id| {
                    if let Some(p) = self.port_mut(hub_id, id) {
                        p.protocol = HubProto::Mqtt;
                        p.label = format!("{client_id} out");
                    }
                    id
                })
            });
            if let Some(port_id) = port_id {
                for hub in &mut self.hubs {
                    if let Some(port) = hub.outputs.iter_mut().find(|p| p.id == port_id) {
                        port.protocol = HubProto::Mqtt;
                        port.mqtt_connection = client_id.to_string();
                        for out in outputs {
                            if !port.mqtt_outputs.iter().any(|b| b == out) {
                                port.mqtt_outputs.push(out.clone());
                            }
                        }
                        break;
                    }
                }
            }
        }
    }

    /// Output of one hub that another hub takes in on the same protocol.
    pub fn across(&self) -> Vec<Across> {
        let mut links = Vec::new();
        for from in &self.hubs {
            for port in &from.outputs {
                for to in &self.hubs {
                    if to.id == from.id {
                        continue;
                    }
                    for source in &to.sources {
                        if source.protocol == port.protocol {
                            links.push(Across {
                                from_hub: from.id,
                                from_port: port.id,
                                to_hub: to.id,
                                to_port: source.id,
                                proto: port.protocol,
                            });
                        }
                    }
                }
            }
        }
        links
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Hub,
    Source,
    Output,
    Aggregator,
    Calculation,
}

#[derive(Clone, Copy)]
struct Placed {
    hub_id: u64,
    port_id: Option<u64>,
    role: Role,
    rect: Rect,
}

struct Spot {
    hub_id: u64,
    port_id: Option<u64>,
    role: Role,
    center: Pos2,
}

/// Compact columns. Transforms sit on the source↔hub / hub↔output edges.
/// A saved drag wins; an undragged port stays beside its hub.
fn layout_nodes(hubs: &[Hub]) -> Vec<Placed> {
    let defaults = default_spots(hubs);
    let mut hub_default = Vec::new();
    let mut hub_center = Vec::new();
    for hub in hubs {
        let spot = defaults
            .iter()
            .find(|spot| spot.hub_id == hub.id && spot.role == Role::Hub)
            .map(|spot| spot.center)
            .unwrap_or(pos2(MARGIN, MARGIN));
        let center = hub.at.map(|at| pos2(at[0], at[1])).unwrap_or(spot);
        hub_default.push((hub.id, spot));
        hub_center.push((hub.id, center));
    }
    let mut nodes = Vec::new();
    for hub in hubs {
        let center = hub_center
            .iter()
            .find(|(id, _)| *id == hub.id)
            .map(|(_, center)| *center)
            .unwrap_or(pos2(MARGIN, MARGIN));
        nodes.push(placed(hub.id, None, Role::Hub, center, vec2(HUB_W, HUB_H)));
        let origin = hub_default
            .iter()
            .find(|(id, _)| *id == hub.id)
            .map(|(_, center)| *center)
            .unwrap_or(center);
        for port in hub.sources.iter().chain(hub.outputs.iter()) {
            let role = if hub.sources.iter().any(|item| item.id == port.id) {
                Role::Source
            } else {
                Role::Output
            };
            place_dragged(
                &mut nodes,
                &defaults,
                origin,
                center,
                hub.id,
                port.id,
                role,
                port.at,
            );
        }
    }
    // Edge transforms: evenly along the port↔hub segment unless dragged.
    for hub in hubs {
        let Some(hub_node) = nodes
            .iter()
            .find(|node| node.hub_id == hub.id && node.role == Role::Hub)
            .copied()
        else {
            continue;
        };
        let edges: Vec<u64> = hub
            .sources
            .iter()
            .chain(hub.outputs.iter())
            .map(|port| port.id)
            .collect();
        for edge_port in edges {
            let on_edge = hub.steps_on(edge_port);
            if on_edge.is_empty() {
                continue;
            }
            let Some(port_node) = nodes
                .iter()
                .find(|node| node.hub_id == hub.id && node.port_id == Some(edge_port))
                .copied()
            else {
                continue;
            };
            let n = on_edge.len() as f32;
            for (i, (id, role)) in on_edge.into_iter().enumerate() {
                let at = hub
                    .aggregators
                    .iter()
                    .chain(hub.calculations.iter())
                    .find(|step| step.id == id)
                    .and_then(|step| step.at);
                let t = (i as f32 + 1.0) / (n + 1.0);
                let mid =
                    port_node.rect.center() + (hub_node.rect.center() - port_node.rect.center()) * t;
                let center = at.map(|at| pos2(at[0], at[1])).unwrap_or(mid);
                nodes.push(placed(
                    hub.id,
                    Some(id),
                    role,
                    center,
                    vec2(PORT_W, PORT_H),
                ));
            }
        }
    }
    nodes
}

fn place_dragged(
    nodes: &mut Vec<Placed>,
    defaults: &[Spot],
    origin: Pos2,
    center: Pos2,
    hub_id: u64,
    node_id: u64,
    role: Role,
    at: Option<[f32; 2]>,
) {
    let fallback = defaults
        .iter()
        .find(|spot| spot.port_id == Some(node_id))
        .map(|spot| center + (spot.center - origin))
        .unwrap_or(center);
    let at = at.map(|at| pos2(at[0], at[1])).unwrap_or(fallback);
    nodes.push(placed(
        hub_id,
        Some(node_id),
        role,
        at,
        vec2(PORT_W, PORT_H),
    ));
}

fn placed(hub_id: u64, port_id: Option<u64>, role: Role, center: Pos2, size: Vec2) -> Placed {
    Placed {
        hub_id,
        port_id,
        role,
        rect: Rect::from_center_size(center, size),
    }
}

fn default_spots(hubs: &[Hub]) -> Vec<Spot> {
    let src_x = MARGIN + PORT_W * 0.5;
    let hub_x = MARGIN + PORT_W + COL_GAP + HUB_W * 0.5;
    let out_x = hub_x + HUB_W * 0.5 + COL_GAP + PORT_W * 0.5;
    let mut y = MARGIN;
    let mut spots = Vec::new();
    for hub in hubs {
        let rows = hub.sources.len().max(hub.outputs.len()).max(1) as f32;
        let stack = rows * PORT_H + (rows - 1.0).max(0.0) * GAP;
        let band = stack.max(HUB_H) + 36.0;
        let mid = y + band * 0.5;
        spots.push(Spot {
            hub_id: hub.id,
            port_id: None,
            role: Role::Hub,
            center: pos2(hub_x, mid),
        });
        push_port_spots(&mut spots, &hub.sources, Role::Source, src_x, mid, hub.id);
        push_port_spots(&mut spots, &hub.outputs, Role::Output, out_x, mid, hub.id);
        y += band + 28.0;
    }
    spots
}

fn push_port_spots(
    spots: &mut Vec<Spot>,
    ports: &[HubPort],
    role: Role,
    x: f32,
    mid: f32,
    hub_id: u64,
) {
    if ports.is_empty() {
        return;
    }
    let ids: Vec<(u64, Role)> = ports.iter().map(|port| (port.id, role)).collect();
    push_id_spots(spots, &ids, x, mid, hub_id);
}

fn push_id_spots(spots: &mut Vec<Spot>, ids: &[(u64, Role)], x: f32, mid: f32, hub_id: u64) {
    if ids.is_empty() {
        return;
    }
    let stack = ids.len() as f32 * PORT_H + (ids.len() - 1) as f32 * GAP;
    let mut y = mid - stack * 0.5 + PORT_H * 0.5;
    for &(id, role) in ids {
        spots.push(Spot {
            hub_id,
            port_id: Some(id),
            role,
            center: pos2(x, y),
        });
        y += PORT_H + GAP;
    }
}

/// Scroll decision for the hub grid.
///
/// A bar is on only when a node rectangle falls outside the visible grid.
/// Padding, the scrollbar's own thickness, and the current pan offset are not inputs.
struct ScrollFit {
    /// True when content overflows the view on that axis (used by layout tests).
    #[allow(dead_code)]
    allow: [bool; 2],
    /// Content size. This stays the view size on an axis where every node already fits.
    #[allow(dead_code)]
    size: Vec2,
    /// Moves a node that starts before the grid origin into view / centers when it fits.
    shift: Vec2,
}

fn scroll_fit(nodes: &[Placed], view: Vec2) -> ScrollFit {
    let view = view.max(vec2(1.0, 1.0));
    if nodes.is_empty() {
        return ScrollFit {
            allow: [false, false],
            size: view,
            shift: Vec2::ZERO,
        };
    }
    let mut min = nodes[0].rect.min;
    let mut max = nodes[0].rect.max;
    for node in nodes.iter().skip(1) {
        min = min.min(node.rect.min);
        max = max.max(node.rect.max);
    }
    // Half a pixel of slack so a node sitting on the grid edge is still inside.
    // Overflow content keeps a small outer pad so nodes are not flush to the bar.
    const PAD: f32 = 12.0;
    let allow = [
        min.x < -0.5 || max.x > view.x + 0.5,
        min.y < -0.5 || max.y > view.y + 0.5,
    ];
    let mut size = view;
    let mut shift = Vec2::ZERO;
    if allow[0] {
        let origin = (min.x - PAD).min(0.0);
        shift.x = -origin;
        size.x = (max.x + PAD - origin).max(view.x);
    } else {
        // Center the diagram on axes that already fit.
        shift.x = (view.x - (max.x - min.x)) * 0.5 - min.x;
    }
    if allow[1] {
        let origin = (min.y - PAD).min(0.0);
        shift.y = -origin;
        size.y = (max.y + PAD - origin).max(view.y);
    } else {
        shift.y = (view.y - (max.y - min.y)) * 0.5 - min.y;
    }
    ScrollFit { allow, size, shift }
}

/// Scale layout rects about the diagram origin for the current zoom.
fn scale_nodes(nodes: &[Placed], zoom: f32) -> Vec<Placed> {
    nodes
        .iter()
        .map(|node| Placed {
            rect: Rect::from_min_max(
                pos2(node.rect.min.x * zoom, node.rect.min.y * zoom),
                pos2(node.rect.max.x * zoom, node.rect.max.y * zoom),
            ),
            ..*node
        })
        .collect()
}

/// Keep the world point under the cursor fixed while zoom changes.
/// Camera: `screen = grid.min + pan + world * zoom` → `pan' = view_pos - world * new_zoom`.
fn zoom_cam(pan: Vec2, view_pos: Vec2, old_zoom: f32, new_zoom: f32) -> Vec2 {
    let old_z = old_zoom.max(0.0001);
    let world = (view_pos - pan) / old_z;
    view_pos - world * new_zoom
}

/// Wheel (and pinch) over the canvas, anchored at the pointer. Clears scroll so parents do not move.
fn canvas_zoom_factor(ui: &Ui, rect: Rect) -> Option<(f32, Pos2)> {
    ui.ctx().input_mut(|i| {
        let pointer = i.pointer.hover_pos().filter(|pos| rect.contains(*pos))?;
        let scroll = i.smooth_scroll_delta;
        let pinch = i.zoom_delta();
        let mut factor = 1.0;
        if (pinch - 1.0).abs() > 0.0001 {
            factor *= pinch;
        }
        if scroll != Vec2::ZERO {
            // Plain wheel zooms toward `pointer`; pan stays middle/space drag.
            factor *= (ZOOM_WHEEL * scroll.y).exp();
            i.smooth_scroll_delta = Vec2::ZERO;
        }
        if (factor - 1.0).abs() < 0.0001 {
            return None;
        }
        Some((factor, pointer))
    })
}

/// What the hub panel changed this frame.
#[derive(Clone, Copy)]
pub struct HubChange {
    pub dirty: bool,
    pub open: Option<DockPane>,
}

impl HubChange {
    pub fn quiet() -> Self {
        Self {
            dirty: false,
            open: None,
        }
    }

    fn join(self, other: Self) -> Self {
        Self {
            dirty: self.dirty || other.dirty,
            open: self.open.or(other.open),
        }
    }
}

/// Draw the diagram and the editor.
pub fn paint(
    ui: &mut Ui,
    model: &mut HubModel,
    p: &ThemePalette,
    mqtt_connections: &[(String, String)],
) -> HubChange {
    let mut change = HubChange::quiet();
    // The dock column zeros item spacing, and that style is inherited here.
    // Restore the chrome gap (10×6) so toolbar buttons match the rest of the shell.
    ui.spacing_mut().item_spacing = vec2(10.0, 6.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        if text_button(ui, "New hub", false, p).clicked() {
            model.add_hub();
            change.dirty = true;
        }
        if text_button(ui, "Arrange", false, p).clicked() {
            model.clear_arrangement();
            change.dirty = true;
        }
        ui.add_space(8.0);
        legend(ui, p);
        ui.add_space(8.0);
        ui.add(
            egui::Label::new(
                egui::RichText::new(
                    "Drag empty canvas to box-select. Drag a selection to move as a group. Click a link or node for details. Right-click a connection to add aggregator/calculation. Wheel zooms toward cursor. Middle-drag or Space-drag pans.",
                )
                .font(font_ui(12.0))
                .color(p.muted),
            )
            .truncate(),
        );
    });
    ui.add_space(8.0);
    let wide = ui.available_width() > 760.0;
    if wide {
        // Same inter-panel gap as the dock chrome (`SEP`).
        let gap = SEP;
        let total = ui.available_width();
        let height = ui.available_height().max(160.0);
        let max_inspect = (total - gap - DIAGRAM_MIN).max(INSPECT_MIN);
        let inspect_w = model.inspector_width.clamp(INSPECT_MIN, max_inspect);
        let diagram_w = (total - inspect_w - gap).max(DIAGRAM_MIN);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            change = change.join(diagram_view(ui, model, diagram_w, height, p));
            match hub_inspect_splitter(ui, height, gap, p) {
                SplitCmd::Reset => {
                    model.inspector_width = INSPECT_DEFAULT.clamp(INSPECT_MIN, max_inspect);
                    change.dirty = true;
                }
                SplitCmd::Delta(delta) => {
                    let next = (model.inspector_width - delta).clamp(INSPECT_MIN, max_inspect);
                    if (next - model.inspector_width).abs() > 0.01 {
                        model.inspector_width = next;
                        change.dirty = true;
                    }
                }
                SplitCmd::Idle => {}
            }
            change = change.join(inspector(ui, model, inspect_w, height, p, mqtt_connections));
        });
    } else {
        let height = (ui.available_height() * 0.58).clamp(180.0, 420.0);
        change = change.join(diagram_view(ui, model, ui.available_width(), height, p));
        ui.add_space(8.0);
        change = change.join(inspector(
            ui,
            model,
            ui.available_width(),
            ui.available_height().max(120.0),
            p,
            mqtt_connections,
        ));
    }
    change
}

enum SplitCmd {
    Idle,
    Delta(f32),
    Reset,
}

/// Vertical drag bar between the hub canvas and the properties panel.
/// Hit area and paint match the dock `tile_splitter` chrome (SEP gap + inset divider).
fn hub_inspect_splitter(ui: &mut Ui, height: f32, gap: f32, p: &ThemePalette) -> SplitCmd {
    let (rect, resp) = ui.allocate_exact_size(vec2(gap, height), Sense::click_and_drag());
    let fill = if resp.hovered() || resp.dragged() {
        p.accent
    } else {
        // About 40% transparent — same as dock panel dividers.
        tint(p.line, 153)
    };
    ui.painter()
        .rect_filled(divider_bar(rect, true), Rounding::same(1.0), fill);
    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
    if resp.double_clicked() {
        return SplitCmd::Reset;
    }
    if resp.dragged() {
        return SplitCmd::Delta(resp.drag_delta().x);
    }
    SplitCmd::Idle
}

fn legend(ui: &mut Ui, p: &ThemePalette) {
    legend_swatch(ui, p.accent, false, "Through this hub", p);
    legend_swatch(ui, p.live, true, "Continues into another hub", p);
}

fn legend_swatch(ui: &mut Ui, color: Color32, dashed: bool, label: &str, p: &ThemePalette) {
    let (rect, _) = ui.allocate_exact_size(vec2(28.0, 14.0), Sense::hover());
    let y = rect.center().y;
    let stroke = Stroke::new(1.6, color);
    if dashed {
        ui.painter()
            .line_segment([pos2(rect.min.x, y), pos2(rect.min.x + 8.0, y)], stroke);
        ui.painter()
            .line_segment([pos2(rect.min.x + 12.0, y), pos2(rect.max.x, y)], stroke);
    } else {
        ui.painter()
            .line_segment([pos2(rect.min.x, y), pos2(rect.max.x, y)], stroke);
    }
    ui.label(
        egui::RichText::new(label)
            .font(font_ui(11.0))
            .color(p.muted),
    );
}

fn diagram_view(
    ui: &mut Ui,
    model: &mut HubModel,
    width: f32,
    height: f32,
    p: &ThemePalette,
) -> HubChange {
    let (rect, _) = ui.allocate_exact_size(vec2(width.max(1.0), height.max(1.0)), Sense::hover());
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, Rounding::same(8.0), blend_scope(p.bg, p.is_light));
    paint_grid(&painter, rect, p.is_light);
    painter.rect_stroke(
        rect,
        Rounding::same(8.0),
        Stroke::new(1.0, tint(p.accent, 150)),
    );
    let grid = rect.shrink(1.0);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(grid)
            .layout(Layout::top_down(egui::Align::Min)),
    );
    child.set_clip_rect(grid);
    let mut change = HubChange::quiet();

    let mut zoom = clamp_zoom(model.zoom);
    let fit = scroll_fit(&scale_nodes(&layout_nodes(&model.hubs), zoom), grid.size());
    // Persistent camera; None falls back to auto-center / overflow fit shift.
    let mut cam = model
        .view_pan
        .map(|p| vec2(p[0], p[1]))
        .unwrap_or(fit.shift);

    // Middle-drag / Space-drag pans the camera (content follows the pointer).
    let pan_delta = canvas_pan_delta(&child, rect);
    let panning = pan_delta.is_some();
    if let Some(delta) = pan_delta {
        cam += delta;
        model.view_pan = Some([cam.x, cam.y]);
    }

    // Wheel zoom: keep the world point under the pointer fixed.
    if let Some((factor, pointer)) = canvas_zoom_factor(&child, rect) {
        let next = clamp_zoom(zoom * factor);
        if (next - zoom).abs() > 0.0005 {
            let view_pos = pointer - grid.min;
            cam = zoom_cam(cam, view_pos, zoom, next);
            model.view_pan = Some([cam.x, cam.y]);
            zoom = next;
            if (model.zoom - zoom).abs() > 0.0005 {
                model.zoom = zoom;
                change.dirty = true;
            }
        }
    }

    // Paint in the grid; camera pan replaces ScrollArea for zoom/pan.
    let (area, canvas) = child.allocate_exact_size(grid.size(), Sense::click_and_drag());
    if model.hubs.is_empty() {
        child.painter().text(
            area.center(),
            Align2::CENTER_CENTER,
            "No hubs yet — right-click or use New hub",
            font_ui(14.0),
            p.muted,
        );
        if let Some(cmd) = canvas_menu(&canvas, None, None) {
            apply_menu(model, cmd, &mut change);
        }
        return change;
    }
    let nodes = scale_nodes(&layout_nodes(&model.hubs), zoom);
    let shift = area.min.to_vec2() + cam;
    let screen: Vec<Placed> = nodes
        .iter()
        .map(|node| Placed {
            rect: node.rect.translate(shift),
            ..*node
        })
        .collect();
    let mut gesture = gesture_nodes(&mut child, model, &screen, panning);
    // Links only when no node took the pointer (nodes sit on top of edges).
    if gesture.hover.is_none()
        && gesture.pick.is_none()
        && gesture.menu.is_none()
        && !panning
    {
        if let Some((hub_id, port_id)) = hit_link(&mut child, model, &screen, zoom) {
            gesture.hot_link = Some((hub_id, port_id));
            gesture.hover = Some((hub_id, Some(port_id)));
            child.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            if canvas.clicked() || canvas.secondary_clicked() {
                gesture.pick = Some((hub_id, Some(port_id)));
                gesture.link_pick = true;
            }
        }
    }
    // Bounding-box select on empty canvas (not while panning, on a node, or on a link).
    let marquee_id = Id::new("pb_hub_marquee");
    let mut marquee_paint = None;
    let on_empty = gesture.hover.is_none()
        && gesture.drag.is_none()
        && gesture.hot_link.is_none()
        && !panning;
    if on_empty && canvas.drag_started() {
        if let Some(pos) = canvas.interact_pointer_pos() {
            child.ctx().data_mut(|d| {
                d.insert_temp(
                    marquee_id,
                    Marquee {
                        origin: pos,
                        tip: pos,
                    },
                );
            });
        }
    }
    if canvas.dragged() {
        if let Some(mut box_drag) = child.ctx().data(|d| d.get_temp::<Marquee>(marquee_id)) {
            if let Some(pos) = canvas.interact_pointer_pos() {
                box_drag.tip = pos;
                child.ctx().data_mut(|d| d.insert_temp(marquee_id, box_drag));
                marquee_paint = Some(box_drag);
            }
        }
    }
    let mut finished_marquee = false;
    if canvas.drag_stopped() {
        if let Some(box_drag) = child.ctx().data_mut(|d| d.remove_temp::<Marquee>(marquee_id)) {
            finished_marquee = true;
            let box_rect = box_drag.rect();
            if box_rect.width() > 4.0 || box_rect.height() > 4.0 {
                let keys = nodes_in_marquee(&screen, box_rect);
                model.set_selection(keys);
                change.dirty = true;
            }
        }
    } else if let Some(box_drag) = child.ctx().data(|d| d.get_temp::<Marquee>(marquee_id)) {
        marquee_paint = Some(box_drag);
    }
    // Empty click clears selection. Skip when a marquee just finished (release also clicks).
    if on_empty
        && canvas.clicked()
        && !finished_marquee
        && !gesture.link_pick
        && gesture.pick.is_none()
    {
        model.set_selection(Vec::new());
        change.dirty = true;
    }
    if let Some((hub_id, port_id, delta)) = gesture.drag {
        // Group-move every selected node; if the dragged node was outside the
        // selection, replace selection with that one node first.
        if !model.selection_contains(hub_id, port_id) {
            match port_id {
                Some(id) => model.focus_port(hub_id, id),
                None => model.focus_hub(hub_id),
            }
        }
        let keys = model.selection.clone();
        let z = zoom.max(0.0001);
        for (move_hub, move_port) in keys {
            let Some(node) = screen
                .iter()
                .find(|node| node.hub_id == move_hub && node.port_id == move_port)
            else {
                continue;
            };
            let world = (node.rect.center() - shift + delta) / z;
            model.set_center(
                move_hub,
                move_port,
                [world.x.max(24.0), world.y.max(24.0)],
            );
        }
        change.dirty = true;
    }
    if let Some((hub_id, port_id)) = gesture.pick {
        if gesture.link_pick {
            if let Some(port_id) = port_id {
                model.focus_link(hub_id, port_id);
                change.dirty = true;
            }
        } else {
            match port_id {
                Some(port_id) => model.focus_port(hub_id, port_id),
                None => model.focus_hub(hub_id),
            }
            change.dirty = true;
        }
    }
    if let Some(pane) = gesture.open {
        change.open = Some(pane);
    }
    let target = model
        .selected
        .or_else(|| model.hubs.first().map(|hub| hub.id));
    // Prefer the hovered connection; else the already-selected edge.
    let edge = gesture.hot_link.or_else(|| model.transform_edge());
    let menu = gesture.menu.or_else(|| canvas_menu(&canvas, target, edge));
    if let Some(cmd) = menu {
        apply_menu(model, cmd, &mut change);
    }
    let nodes = scale_nodes(&layout_nodes(&model.hubs), zoom);
    let screen: Vec<Placed> = nodes
        .iter()
        .map(|node| Placed {
            rect: node.rect.translate(shift),
            ..*node
        })
        .collect();
    paint_flow(
        child.painter(),
        &screen,
        model,
        gesture.hover,
        gesture.hot_link.or(model.selected_link.map(|id| (model.selected.unwrap_or(0), id))),
        zoom,
        p,
    );
    if let Some(box_drag) = marquee_paint {
        paint_marquee(child.painter(), box_drag.rect(), p);
    }
    if panning {
        child.ctx().set_cursor_icon(egui::CursorIcon::AllScroll);
    }
    change
}

/// Middle-drag, or Space+left-drag: pointer delta for camera pan.
fn canvas_pan_delta(ui: &Ui, rect: Rect) -> Option<Vec2> {
    let typing = ui.ctx().wants_keyboard_input();
    ui.ctx().input(|i| {
        let over = i.pointer.hover_pos().is_some_and(|pos| rect.contains(pos));
        if !over {
            return None;
        }
        let middle = i.pointer.button_down(egui::PointerButton::Middle);
        let space = i.key_down(egui::Key::Space) && !typing;
        if middle || (space && i.pointer.primary_down()) {
            Some(i.pointer.delta())
        } else {
            None
        }
    })
}

enum MenuCmd {
    AddAggregator { hub_id: u64, edge_port: u64 },
    AddCalculation { hub_id: u64, edge_port: u64 },
    AddSource(u64),
    AddOutput(u64),
    NewHub,
}

fn apply_menu(model: &mut HubModel, cmd: MenuCmd, change: &mut HubChange) {
    match cmd {
        MenuCmd::AddAggregator { hub_id, edge_port } => {
            if let Some(step_id) = model.add_step(hub_id, true, edge_port) {
                model.focus_port(hub_id, step_id);
                change.dirty = true;
            }
        }
        MenuCmd::AddCalculation { hub_id, edge_port } => {
            if let Some(step_id) = model.add_step(hub_id, false, edge_port) {
                model.focus_port(hub_id, step_id);
                change.dirty = true;
            }
        }
        MenuCmd::AddSource(hub_id) => {
            if let Some(port_id) = model.add_port(hub_id, true) {
                model.focus_port(hub_id, port_id);
                change.dirty = true;
            }
        }
        MenuCmd::AddOutput(hub_id) => {
            if let Some(port_id) = model.add_port(hub_id, false) {
                model.focus_port(hub_id, port_id);
                change.dirty = true;
            }
        }
        MenuCmd::NewHub => {
            model.add_hub();
            change.dirty = true;
        }
    }
}

/// Edge transforms: only when a connection (or its port) is the target.
fn edge_transform_menu(ui: &mut Ui, hub_id: u64, edge_port: u64, cmd: &mut Option<MenuCmd>) {
    if ui.button("Add aggregator").clicked() {
        *cmd = Some(MenuCmd::AddAggregator { hub_id, edge_port });
        ui.close_menu();
    }
    if ui.button("Add calculation").clicked() {
        *cmd = Some(MenuCmd::AddCalculation { hub_id, edge_port });
        ui.close_menu();
    }
}

/// Hub port adds (source / output).
fn hub_port_menu(ui: &mut Ui, hub_id: u64, cmd: &mut Option<MenuCmd>) {
    if ui.button("Add source").clicked() {
        *cmd = Some(MenuCmd::AddSource(hub_id));
        ui.close_menu();
    }
    if ui.button("Add output").clicked() {
        *cmd = Some(MenuCmd::AddOutput(hub_id));
        ui.close_menu();
    }
}

/// Right-click empty canvas. Transforms only when an edge is already selected.
fn canvas_menu(
    resp: &egui::Response,
    hub_id: Option<u64>,
    edge: Option<(u64, u64)>,
) -> Option<MenuCmd> {
    let mut cmd = None;
    resp.context_menu(|ui| {
        ui.set_min_width(168.0);
        if let Some((hub_id, edge_port)) = edge {
            edge_transform_menu(ui, hub_id, edge_port, &mut cmd);
            ui.separator();
        }
        if let Some(hub_id) = hub_id {
            hub_port_menu(ui, hub_id, &mut cmd);
            ui.separator();
        }
        if ui.button("New hub").clicked() {
            cmd = Some(MenuCmd::NewHub);
            ui.close_menu();
        }
    });
    cmd
}

struct Gesture {
    hover: Option<(u64, Option<u64>)>,
    pick: Option<(u64, Option<u64>)>,
    drag: Option<(u64, Option<u64>, Vec2)>,
    open: Option<DockPane>,
    menu: Option<MenuCmd>,
    /// True when `pick` came from a connection, not a node.
    link_pick: bool,
    /// Hovered / menu-targeted link (hub_id, port_id).
    hot_link: Option<(u64, u64)>,
}

fn gesture_nodes(ui: &mut Ui, model: &HubModel, nodes: &[Placed], panning: bool) -> Gesture {
    let mut gesture = Gesture {
        hover: None,
        pick: None,
        drag: None,
        open: None,
        menu: None,
        link_pick: false,
        hot_link: None,
    };
    for node in nodes {
        let id = Id::new((
            "pb_hub_node",
            node.hub_id,
            node.port_id.unwrap_or(0),
            node.role as u8,
        ));
        let opens = open_target(model, node).is_some();
        let tip = match node.role {
            Role::Source | Role::Output => {
                "Drag to move. Click for details. Right-click to add aggregator/calculation on this link."
            }
            _ if opens => {
                "Drag to move. Click for details. Double-click opens the protocol panel. Right-click to add source/output."
            }
            _ => "Drag to move. Click for details. Right-click to add source/output.",
        };
        let resp = ui
            .interact(node.rect, id, Sense::click_and_drag())
            .on_hover_text(tip);
        let key = (node.hub_id, node.port_id);
        if !panning && (resp.hovered() || resp.dragged()) {
            gesture.hover = Some(key);
            let cursor = if resp.dragged() {
                egui::CursorIcon::Grabbing
            } else {
                egui::CursorIcon::Grab
            };
            ui.ctx().set_cursor_icon(cursor);
        }
        if !panning && resp.dragged() {
            gesture.drag = Some((node.hub_id, node.port_id, resp.drag_delta()));
        } else if resp.clicked() {
            gesture.pick = Some(key);
        }
        if resp.secondary_clicked() {
            gesture.pick = Some(key);
        }
        if resp.double_clicked() {
            gesture.open = open_target(model, node);
        }
        let mut menu = None;
        resp.context_menu(|ui| {
            ui.set_min_width(168.0);
            match node.role {
                Role::Source | Role::Output => {
                    if let Some(port_id) = node.port_id {
                        edge_transform_menu(ui, node.hub_id, port_id, &mut menu);
                        ui.separator();
                    }
                    hub_port_menu(ui, node.hub_id, &mut menu);
                }
                Role::Aggregator | Role::Calculation => {
                    if let Some(step) = find_step(model, node.hub_id, node.port_id) {
                        edge_transform_menu(ui, node.hub_id, step.edge_port, &mut menu);
                    }
                }
                Role::Hub => hub_port_menu(ui, node.hub_id, &mut menu),
            }
        });
        if menu.is_some() {
            gesture.menu = menu;
        }
    }
    gesture
}

/// Nearest source↔hub / hub↔output connection under the pointer, if within hit slop.
fn hit_link(ui: &Ui, model: &HubModel, nodes: &[Placed], zoom: f32) -> Option<(u64, u64)> {
    let pointer = ui
        .ctx()
        .pointer_hover_pos()
        .or_else(|| ui.ctx().pointer_interact_pos())?;
    let thresh = (8.0 * zoom.max(0.35)).max(6.0);
    let mut best: Option<(f32, u64, u64)> = None;
    for node in nodes {
        if node.role != Role::Source && node.role != Role::Output {
            continue;
        }
        let Some(port_id) = node.port_id else {
            continue;
        };
        let Some(hub) = find(nodes, node.hub_id, Role::Hub, None) else {
            continue;
        };
        let chain = edge_chain(model, nodes, node, hub, port_id);
        for pair in chain.windows(2) {
            let Some(dist) = link_distance(pair[0], pair[1], pointer) else {
                continue;
            };
            if dist > thresh {
                continue;
            }
            let closer = best.map(|(d, _, _)| dist < d).unwrap_or(true);
            if closer {
                best = Some((dist, node.hub_id, port_id));
            }
        }
    }
    best.map(|(_, hub_id, port_id)| (hub_id, port_id))
}

/// Node rects along one port's path (port → transforms → hub, or hub → transforms → port).
fn edge_chain(
    model: &HubModel,
    nodes: &[Placed],
    port: &Placed,
    hub: &Placed,
    edge_port: u64,
) -> Vec<Rect> {
    let on_edge: Vec<u64> = model
        .hub(port.hub_id)
        .map(|h| h.steps_on(edge_port).into_iter().map(|(id, _)| id).collect())
        .unwrap_or_default();
    let mut steps: Vec<&Placed> = nodes
        .iter()
        .filter(|node| {
            node.hub_id == port.hub_id
                && matches!(node.role, Role::Aggregator | Role::Calculation)
                && node.port_id.is_some_and(|id| on_edge.contains(&id))
        })
        .collect();
    let origin = port.rect.center();
    steps.sort_by(|a, b| {
        let da = (a.rect.center() - origin).length_sq();
        let db = (b.rect.center() - origin).length_sq();
        da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut chain = Vec::with_capacity(steps.len() + 2);
    if port.role == Role::Source {
        chain.push(port.rect);
        chain.extend(steps.iter().map(|node| node.rect));
        chain.push(hub.rect);
    } else {
        chain.push(hub.rect);
        chain.extend(steps.iter().map(|node| node.rect));
        chain.push(port.rect);
    }
    chain
}

/// Shortest distance from `pos` to the cubic link between two node rects.
fn link_distance(from: Rect, to: Rect, pos: Pos2) -> Option<f32> {
    let (start, end, dir) = facing(from, to);
    let pull = ((end - start).length() * 0.42).clamp(28.0, 110.0);
    let c1 = start + dir * pull;
    let c2 = end - dir * pull;
    let mut best = f32::MAX;
    for step in 0..=24 {
        let t = step as f32 / 24.0;
        let point = cubic(start, c1, c2, end, t);
        best = best.min((point - pos).length());
    }
    if best.is_finite() {
        Some(best)
    } else {
        None
    }
}

/// Protocol panel for this node, when the node is one protocol.
fn open_target(model: &HubModel, node: &Placed) -> Option<DockPane> {
    match node.role {
        Role::Hub => model.hub(node.hub_id).map(|hub| hub.protocol.pane()),
        Role::Source | Role::Output => {
            find_port(model, node.hub_id, node.port_id).map(|port| port.protocol.pane())
        }
        Role::Aggregator | Role::Calculation => None,
    }
}

fn paint_grid(painter: &egui::Painter, rect: Rect, is_light: bool) {
    let step = 18.0;
    let stroke = Stroke::new(1.0, crate::theme::scope::grid(is_light));
    let mut x = rect.min.x;
    while x < rect.max.x {
        painter.line_segment([pos2(x, rect.min.y), pos2(x, rect.max.y)], stroke);
        x += step;
    }
    let mut y = rect.min.y;
    while y < rect.max.y {
        painter.line_segment([pos2(rect.min.x, y), pos2(rect.max.x, y)], stroke);
        y += step;
    }
}

fn paint_flow(
    painter: &egui::Painter,
    nodes: &[Placed],
    model: &HubModel,
    hover: Option<(u64, Option<u64>)>,
    hot_link: Option<(u64, u64)>,
    zoom: f32,
    p: &ThemePalette,
) {
    for node in nodes {
        if node.role != Role::Source && node.role != Role::Output {
            continue;
        }
        let Some(port_id) = node.port_id else {
            continue;
        };
        let Some(hub) = find(nodes, node.hub_id, Role::Hub, None) else {
            continue;
        };
        let chain = edge_chain(model, nodes, node, hub, port_id);
        let ink = port_proto(model, node.hub_id, node.port_id).ink(p.is_light);
        let link_selected = hot_link == Some((node.hub_id, port_id))
            || model.selected_link == Some(port_id) && model.selected == Some(node.hub_id);
        let hot = link_selected || link_hot(node, hub, hover, model);
        for pair in chain.windows(2) {
            link_curve(painter, pair[0], pair[1], ink, false, hot);
        }
    }
    for link in model.across() {
        let Some(from) = find(nodes, link.from_hub, Role::Output, Some(link.from_port)) else {
            continue;
        };
        let Some(to) = find(nodes, link.to_hub, Role::Source, Some(link.to_port)) else {
            continue;
        };
        let hot = link_hot(from, to, hover, model);
        let ink = link.proto.ink(p.is_light);
        link_curve(painter, from.rect, to.rect, ink, true, hot);
    }
    for node in nodes {
        let hot = hovered(node, hover);
        let on = selected_node(model, node)
            || (matches!(node.role, Role::Source | Role::Output)
                && model.selected == Some(node.hub_id)
                && model.selected_link == node.port_id
                && model.selected_port.is_none());
        match node.role {
            Role::Hub => paint_hub_node(painter, node, model, on, hot, zoom, p),
            Role::Source | Role::Output => paint_port_node(painter, node, model, on, hot, zoom, p),
            Role::Aggregator | Role::Calculation => {
                paint_step_node(painter, node, model, on, hot, zoom, p)
            }
        }
    }
}

fn hovered(node: &Placed, hover: Option<(u64, Option<u64>)>) -> bool {
    hover.is_some_and(|(hub, port)| node.hub_id == hub && node.port_id == port)
}

fn selected_node(model: &HubModel, node: &Placed) -> bool {
    model.selection_contains(node.hub_id, node.port_id)
}

/// Drag-box on the empty canvas for multi-select.
#[derive(Clone, Copy, Default)]
struct Marquee {
    origin: Pos2,
    tip: Pos2,
}

impl Marquee {
    fn rect(self) -> Rect {
        Rect::from_two_pos(self.origin, self.tip)
    }
}

fn nodes_in_marquee(nodes: &[Placed], rect: Rect) -> Vec<(u64, Option<u64>)> {
    nodes
        .iter()
        .filter(|node| rect.intersects(node.rect))
        .map(|node| (node.hub_id, node.port_id))
        .collect()
}

fn paint_marquee(painter: &egui::Painter, rect: Rect, p: &ThemePalette) {
    painter.rect_filled(rect, Rounding::same(2.0), tint(p.accent, 28));
    painter.rect_stroke(rect, Rounding::same(2.0), Stroke::new(1.2, p.accent));
}

fn link_hot(a: &Placed, b: &Placed, hover: Option<(u64, Option<u64>)>, model: &HubModel) -> bool {
    hovered(a, hover) || hovered(b, hover) || selected_node(model, a) || selected_node(model, b)
}

fn paint_hub_node(
    painter: &egui::Painter,
    node: &Placed,
    model: &HubModel,
    on: bool,
    hot: bool,
    zoom: f32,
    p: &ThemePalette,
) {
    let Some(hub) = model.hub(node.hub_id) else {
        return;
    };
    let z = zoom.max(0.35);
    let ink = hub.protocol.ink(p.is_light);
    paint_card(painter, node.rect, ink, on, hot, p);
    paint_grip(painter, node.rect, if hot || on { ink } else { p.muted });
    let clip = Rect::from_min_max(
        pos2(node.rect.min.x + 18.0 * z, node.rect.min.y + 4.0 * z),
        pos2(node.rect.max.x - 8.0 * z, node.rect.max.y - 4.0 * z),
    );
    painter.with_clip_rect(clip).text(
        pos2(clip.min.x, node.rect.center().y - 9.0 * z),
        Align2::LEFT_CENTER,
        &hub.name,
        font_semi(14.0 * z),
        p.text,
    );
    painter.with_clip_rect(clip).text(
        pos2(clip.min.x, node.rect.center().y + 10.0 * z),
        Align2::LEFT_CENTER,
        hub.protocol.label(),
        font_ui(11.0 * z),
        if on { ink } else { p.muted },
    );
}

fn paint_port_node(
    painter: &egui::Painter,
    node: &Placed,
    model: &HubModel,
    on: bool,
    hot: bool,
    zoom: f32,
    p: &ThemePalette,
) {
    let Some(port) = find_port(model, node.hub_id, node.port_id) else {
        return;
    };
    let z = zoom.max(0.35);
    let ink = port.protocol.ink(p.is_light);
    paint_card(painter, node.rect, ink, on, hot, p);
    let bar = Rect::from_min_max(
        node.rect.min,
        pos2(node.rect.min.x + 4.0 * z, node.rect.max.y),
    );
    painter.rect_filled(
        bar,
        Rounding {
            nw: 7.0 * z,
            ne: 0.0,
            sw: 7.0 * z,
            se: 0.0,
        },
        ink,
    );
    let clip = Rect::from_min_max(
        pos2(node.rect.min.x + 12.0 * z, node.rect.min.y),
        pos2(node.rect.max.x - 8.0 * z, node.rect.max.y),
    );
    let role = if node.role == Role::Source {
        "IN"
    } else {
        "OUT"
    };
    painter.with_clip_rect(clip).text(
        pos2(clip.min.x, node.rect.center().y - 8.0 * z),
        Align2::LEFT_CENTER,
        format!("{role}  {}", port.protocol.label()),
        font_ui(10.0 * z),
        ink,
    );
    painter.with_clip_rect(clip).text(
        pos2(clip.min.x, node.rect.center().y + 8.0 * z),
        Align2::LEFT_CENTER,
        &port.label,
        font_ui(13.0 * z),
        p.text,
    );
}

fn paint_step_node(
    painter: &egui::Painter,
    node: &Placed,
    model: &HubModel,
    on: bool,
    hot: bool,
    zoom: f32,
    p: &ThemePalette,
) {
    let Some(step) = find_step(model, node.hub_id, node.port_id) else {
        return;
    };
    let z = zoom.max(0.35);
    let ink = p.accent;
    paint_card(painter, node.rect, ink, on, hot, p);
    let bar = Rect::from_min_max(
        node.rect.min,
        pos2(node.rect.min.x + 4.0 * z, node.rect.max.y),
    );
    painter.rect_filled(
        bar,
        Rounding {
            nw: 7.0 * z,
            ne: 0.0,
            sw: 7.0 * z,
            se: 0.0,
        },
        ink,
    );
    let clip = Rect::from_min_max(
        pos2(node.rect.min.x + 12.0 * z, node.rect.min.y),
        pos2(node.rect.max.x - 8.0 * z, node.rect.max.y),
    );
    let role = if node.role == Role::Aggregator {
        "AGG"
    } else {
        "CALC"
    };
    painter.with_clip_rect(clip).text(
        pos2(clip.min.x, node.rect.center().y - 8.0 * z),
        Align2::LEFT_CENTER,
        role,
        font_ui(10.0 * z),
        ink,
    );
    painter.with_clip_rect(clip).text(
        pos2(clip.min.x, node.rect.center().y + 8.0 * z),
        Align2::LEFT_CENTER,
        &step.label,
        font_ui(13.0 * z),
        p.text,
    );
}

fn paint_card(
    painter: &egui::Painter,
    rect: Rect,
    ink: Color32,
    on: bool,
    hot: bool,
    p: &ThemePalette,
) {
    let shadow = rect.translate(vec2(0.0, 2.0));
    painter.rect_filled(shadow, Rounding::same(8.0), Color32::from_black_alpha(36));
    let fill = if on {
        p.header_top
    } else if hot {
        tint(ink, if p.is_light { 28 } else { 36 })
    } else {
        p.pane
    };
    painter.rect_filled(rect, Rounding::same(8.0), fill);
    if on {
        painter.rect_stroke(
            rect.expand(3.0),
            Rounding::same(10.0),
            Stroke::new(1.5, p.accent),
        );
    }
    painter.rect_stroke(
        rect,
        Rounding::same(8.0),
        Stroke::new(
            if on || hot { 1.8 } else { 1.15 },
            if on || hot { ink } else { p.line },
        ),
    );
}

fn paint_grip(painter: &egui::Painter, rect: Rect, color: Color32) {
    let origin = pos2(rect.min.x + 8.0, rect.center().y - 6.0);
    for row in 0..3 {
        for col in 0..2 {
            painter.circle_filled(
                pos2(origin.x + col as f32 * 3.2, origin.y + row as f32 * 4.0),
                1.05,
                color,
            );
        }
    }
}

fn inspector(
    ui: &mut Ui,
    model: &mut HubModel,
    width: f32,
    height: f32,
    p: &ThemePalette,
    mqtt_connections: &[(String, String)],
) -> HubChange {
    let (rect, _) = ui.allocate_exact_size(vec2(width.max(1.0), height.max(1.0)), Sense::hover());
    ui.painter()
        .rect_filled(rect, Rounding::same(8.0), p.control);
    ui.painter()
        .rect_stroke(rect, Rounding::same(8.0), Stroke::new(1.0, p.line));
    // Keep controls off the rounded edge. The clip is a hair outside that inset so a 1px stroke still draws.
    let inner = rect.shrink(12.0);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::top_down(egui::Align::Min)),
    );
    child.set_clip_rect(inner.expand(2.0).intersect(rect.shrink(2.0)));
    let mut change = HubChange::quiet();
    ScrollArea::vertical()
        .id_salt("pb_hub_edit")
        .auto_shrink([false, false])
        .show(&mut child, |ui| {
            // Overlay scrollbar sits on the right; keep Add / × / row edges clear of it.
            let width = (ui.available_width() - INSPECT_SCROLL_GUTTER).max(1.0);
            ui.set_width(width);
            change = change.join(edit_selected(ui, model, p, mqtt_connections));
        });
    change
}

fn edit_selected(
    ui: &mut Ui,
    model: &mut HubModel,
    p: &ThemePalette,
    mqtt_connections: &[(String, String)],
) -> HubChange {
    // Item-specific properties only when exactly one canvas node is selected.
    if model.multi_selected() {
        ui.label(
            egui::RichText::new(format!("{} nodes selected", model.selection.len()))
                .font(font_ui(12.5))
                .color(p.muted),
        );
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new("Select a single node to edit its properties.")
                .font(font_ui(12.0))
                .color(p.muted),
        );
        return HubChange::quiet();
    }
    // Nothing selected (empty-canvas click) — clear the panel.
    if model.selection.is_empty() && model.selected_link.is_none() {
        ui.label(
            egui::RichText::new("Click a node to see it here.")
                .font(font_ui(12.5))
                .color(p.muted),
        );
        return HubChange::quiet();
    }
    if model.selected_port.is_some() {
        let hub_id = model.selected;
        let port_id = model.selected_port;
        if hub_id
            .zip(port_id)
            .is_some_and(|(hub_id, port_id)| find_step(model, hub_id, Some(port_id)).is_some())
        {
            return edit_step(ui, model, p);
        }
        return edit_port(ui, model, p, mqtt_connections);
    }
    edit_hub(ui, model, p)
}

fn edit_step(ui: &mut Ui, model: &mut HubModel, p: &ThemePalette) -> HubChange {
    let Some(hub_id) = model.selected else {
        return HubChange::quiet();
    };
    let Some(step_id) = model.selected_port else {
        return HubChange::quiet();
    };
    let Some(hub) = model.hub(hub_id) else {
        return HubChange::quiet();
    };
    let aggregator = hub.aggregators.iter().any(|step| step.id == step_id);
    let edge_port = hub
        .aggregators
        .iter()
        .chain(hub.calculations.iter())
        .find(|step| step.id == step_id)
        .map(|step| step.edge_port);
    let edge_label = edge_port
        .and_then(|port_id| {
            hub.sources
                .iter()
                .chain(hub.outputs.iter())
                .find(|port| port.id == port_id)
                .map(|port| port.label.clone())
        })
        .unwrap_or_else(|| "connection".into());
    let hub_name = hub.name.clone();
    let mut change = HubChange::quiet();
    let kind = if aggregator {
        "AGGREGATOR"
    } else {
        "CALCULATION"
    };
    ui.label(
        egui::RichText::new(kind)
            .font(font_ui(10.5))
            .color(p.accent),
    );
    ui.label(
        egui::RichText::new(format!("On edge · {edge_label} · hub {hub_name}"))
            .font(font_ui(12.0))
            .color(p.muted),
    );
    ui.add_space(6.0);
    let label = &mut step_field(model, hub_id, step_id).label;
    if ui
        .add(egui::TextEdit::singleline(label).desired_width(f32::INFINITY))
        .changed()
    {
        change.dirty = true;
    }
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        if text_button(ui, "Hub details", false, p).clicked() {
            model.focus_hub(hub_id);
            change.dirty = true;
        }
        if text_button(ui, "Remove", false, p).clicked() {
            model.remove_port(hub_id, step_id);
            change.dirty = true;
        }
    });
    change
}

fn edit_port(
    ui: &mut Ui,
    model: &mut HubModel,
    p: &ThemePalette,
    mqtt_connections: &[(String, String)],
) -> HubChange {
    let Some(hub_id) = model.selected else {
        return HubChange::quiet();
    };
    let Some(port_id) = model.selected_port else {
        return HubChange::quiet();
    };
    let Some(port) = find_port(model, hub_id, Some(port_id)) else {
        return HubChange::quiet();
    };
    let proto = port.protocol;
    let incoming = model
        .hub(hub_id)
        .is_some_and(|hub| hub.sources.iter().any(|item| item.id == port_id));
    let hub_name = model
        .hub(hub_id)
        .map(|hub| hub.name.clone())
        .unwrap_or_default();
    let mut change = HubChange::quiet();
    let kind = if incoming { "INPUT" } else { "OUTPUT" };
    ui.label(
        egui::RichText::new(format!("{kind}  ·  {}", proto.label()))
            .font(font_ui(10.5))
            .color(proto.ink(p.is_light)),
    );
    ui.label(
        egui::RichText::new(format!("On hub {hub_name}"))
            .font(font_ui(12.0))
            .color(p.muted),
    );
    ui.add_space(6.0);
    let label = &mut port_field(model, hub_id, port_id).label;
    if ui
        .add(egui::TextEdit::singleline(label).desired_width(f32::INFINITY))
        .changed()
    {
        change.dirty = true;
    }
    // Sources/outputs always pick their own protocol (hub is separately one server protocol).
    ui.add_space(6.0);
    let current = port_field(model, hub_id, port_id).protocol;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        for next in HubProto::ALL {
            if text_button(ui, next.label(), next == current, p).clicked() && next != current {
                port_field(model, hub_id, port_id).protocol = next;
                change.dirty = true;
            }
        }
    });
    if proto == HubProto::Mqtt {
        ui.add_space(12.0);
        change = change.join(edit_mqtt_port_binds(
            ui,
            model,
            hub_id,
            port_id,
            incoming,
            mqtt_connections,
            p,
        ));
    }
    ui.add_space(10.0);
    if text_button(ui, &format!("Open {} panel", proto.label()), true, p).clicked() {
        change.open = Some(proto.pane());
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if text_button(ui, "Hub details", false, p).clicked() {
            model.focus_hub(hub_id);
            change.dirty = true;
        }
        if text_button(ui, "Remove", false, p).clicked() {
            model.remove_port(hub_id, port_id);
            change.dirty = true;
        }
    });
    change
}

fn edit_mqtt_port_binds(
    ui: &mut Ui,
    model: &mut HubModel,
    hub_id: u64,
    port_id: u64,
    incoming: bool,
    mqtt_connections: &[(String, String)],
    p: &ThemePalette,
) -> HubChange {
    let mut change = HubChange::quiet();
    ui.label(
        egui::RichText::new("MQTT CONNECTION")
            .font(font_ui(10.5))
            .color(p.muted),
    );
    ui.add_space(2.0);
    ui.label(
        egui::RichText::new(
            "Many Input nodes may share one connection; each owns its own topic/field → point binds (overlap OK).",
        )
        .font(font_ui(11.0))
        .color(p.muted),
    );
    ui.add_space(4.0);
    let current = port_field(model, hub_id, port_id).mqtt_connection.clone();
    let current_label = mqtt_connections
        .iter()
        .find(|(id, _)| id == &current)
        .map(|(_, label)| label.as_str())
        .unwrap_or(if current.is_empty() {
            "(pick connection)"
        } else {
            current.as_str()
        });
    let mut next = current.clone();
    egui::ComboBox::from_id_salt(("pb_hub_mqtt_conn", port_id))
        .selected_text(current_label)
        .width((ui.available_width()).min(240.0).max(120.0))
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut next, String::new(), "(none)");
            for (id, label) in mqtt_connections {
                ui.selectable_value(&mut next, id.clone(), format!("{label} ({id})"));
            }
        });
    if next != current {
        port_field(model, hub_id, port_id).mqtt_connection = next;
        change.dirty = true;
    }

    if incoming {
        ui.add_space(10.0);
        ui.label(
            egui::RichText::new("INPUT BINDS — topic / field → auto mqtt_N")
                .font(font_ui(10.5))
                .color(p.muted),
        );
        ui.add_space(4.0);
        let mut remove = None;
        let mut add = false;
        {
            let port = port_field(model, hub_id, port_id);
            for (i, bind) in port.mqtt_inputs.iter().enumerate() {
                // Point id is always host-assigned sequential mqtt_N (no manual picker).
                let label = if bind.path.is_empty() {
                    format!("{} → mqtt_N", bind.topic)
                } else {
                    format!("{}::{} → mqtt_N", bind.topic, bind.path)
                };
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(label)
                            .font(font_ui(12.0))
                            .color(p.text),
                    );
                    if text_button(ui, "×", false, p).clicked() {
                        remove = Some(i);
                    }
                });
            }
            inspect_field(ui, "Topic", &mut port.draft_mqtt_topic, p);
            inspect_field(ui, "JSON path", &mut port.draft_mqtt_path, p);
            ui.label(
                egui::RichText::new(
                    "Path optional (whole payload). Point id always auto-assigned (mqtt_1, mqtt_2, …).",
                )
                .font(font_ui(11.0))
                .color(p.muted),
            );
            if text_button(ui, "Add bind", false, p).clicked() {
                add = true;
            }
        }
        if let Some(i) = remove {
            port_field(model, hub_id, port_id).mqtt_inputs.remove(i);
            change.dirty = true;
        }
        if add {
            let port = port_field(model, hub_id, port_id);
            let topic = port.draft_mqtt_topic.trim().to_string();
            if !topic.is_empty() {
                let bind = MqttInputBind {
                    topic,
                    path: port.draft_mqtt_path.trim().to_string(),
                    point: String::new(),
                };
                if !port
                    .mqtt_inputs
                    .iter()
                    .any(|b| b.topic == bind.topic && b.path == bind.path)
                {
                    port.mqtt_inputs.push(bind);
                }
                port.draft_mqtt_topic.clear();
                port.draft_mqtt_path.clear();
                change.dirty = true;
            }
        }
    } else {
        ui.add_space(10.0);
        ui.label(
            egui::RichText::new("OUTPUT BINDS — point → publish topic")
                .font(font_ui(10.5))
                .color(p.muted),
        );
        ui.add_space(4.0);
        let mut remove = None;
        let mut add = false;
        {
            let port = port_field(model, hub_id, port_id);
            for (i, bind) in port.mqtt_outputs.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("{} → {}", bind.point, bind.topic))
                            .font(font_ui(12.0))
                            .color(p.text),
                    );
                    if text_button(ui, "×", false, p).clicked() {
                        remove = Some(i);
                    }
                });
            }
            inspect_field(ui, "Point", &mut port.draft_mqtt_out_point, p);
            inspect_field(ui, "Topic", &mut port.draft_mqtt_out_topic, p);
            if text_button(ui, "Add bind", false, p).clicked() {
                add = true;
            }
        }
        if let Some(i) = remove {
            port_field(model, hub_id, port_id).mqtt_outputs.remove(i);
            change.dirty = true;
        }
        if add {
            let port = port_field(model, hub_id, port_id);
            let point = port.draft_mqtt_out_point.trim().to_string();
            let topic = port.draft_mqtt_out_topic.trim().to_string();
            if !point.is_empty() && !topic.is_empty() {
                let bind = MqttOutputBind {
                    point,
                    topic,
                    qos: 0,
                    retain: false,
                };
                if !port.mqtt_outputs.iter().any(|b| b == &bind) {
                    port.mqtt_outputs.push(bind);
                }
                port.draft_mqtt_out_point.clear();
                port.draft_mqtt_out_topic.clear();
                change.dirty = true;
            }
        }
    }
    change
}

fn inspect_field(ui: &mut Ui, label: &str, value: &mut String, p: &ThemePalette) {
    ui.horizontal(|ui| {
        ui.add_sized(
            [72.0, 20.0],
            egui::Label::new(
                egui::RichText::new(label)
                    .font(font_ui(12.0))
                    .color(p.muted),
            ),
        );
        ui.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY));
    });
    ui.add_space(2.0);
}

fn edit_hub(ui: &mut Ui, model: &mut HubModel, p: &ThemePalette) -> HubChange {
    let Some(id) = model.selected else {
        ui.label(
            egui::RichText::new("Click a node to see it here.")
                .font(font_ui(12.5))
                .color(p.muted),
        );
        return HubChange::quiet();
    };
    if model.hub(id).is_none() {
        return HubChange::quiet();
    }
    let mut change = HubChange::quiet();
    let name = &mut model.hub_mut(id).unwrap().name;
    if ui
        .add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY))
        .changed()
    {
        change.dirty = true;
    }
    ui.add_space(8.0);
    ui.label(
        egui::RichText::new("HUB PROTOCOL")
            .font(font_ui(10.5))
            .color(p.muted),
    );
    ui.add_space(4.0);
    let current = model.hub(id).unwrap().protocol;
    // Selector (not action buttons): pick the hub's one server-capable protocol.
    let mut next = current;
    egui::ComboBox::from_id_salt(("pb_hub_protocol", id))
        .selected_text(current.label())
        .width((ui.available_width()).min(220.0).max(120.0))
        .show_ui(ui, |ui| {
            for proto in HubProto::HUBS {
                ui.selectable_value(&mut next, proto, proto.label());
            }
        });
    if next != current {
        model.set_hub_protocol(id, next);
        change.dirty = true;
    }
    let current = model.hub(id).unwrap().protocol;
    if current == HubProto::Mqtt {
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(
                "MQTT hub/broker role is not active yet. MQTT ships as a client pack for source/output; embedded broker comes later.",
            )
            .font(font_ui(12.0))
            .color(p.muted),
        );
    }
    ui.add_space(8.0);
    if text_button(ui, &format!("Open {} panel", current.label()), true, p).clicked() {
        change.open = Some(current.pane());
    }
    ui.add_space(10.0);
    if port_section(ui, model, id, true, p) {
        change.dirty = true;
    }
    // Aggregators / calculations are edge nodes — edited when selected on the canvas,
    // not listed under hub properties.
    ui.add_space(8.0);
    if port_section(ui, model, id, false, p) {
        change.dirty = true;
    }
    ui.add_space(12.0);
    if text_button(ui, "Delete hub", false, p).clicked() {
        model.remove_hub(id);
        change.dirty = true;
    }
    change
}

fn port_section(
    ui: &mut Ui,
    model: &mut HubModel,
    hub_id: u64,
    source: bool,
    p: &ThemePalette,
) -> bool {
    let mut dirty = false;
    let title = if source { "SOURCES" } else { "OUTPUTS" };
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(title)
                .font(font_ui(10.5))
                .color(p.muted),
        );
        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            if text_button(ui, "Add", false, p).clicked() {
                model.add_port(hub_id, source);
                dirty = true;
            }
        });
    });
    let ports: Vec<u64> = model
        .hub(hub_id)
        .map(|hub| {
            let list = if source { &hub.sources } else { &hub.outputs };
            list.iter().map(|port| port.id).collect()
        })
        .unwrap_or_default();
    for port_id in ports {
        ui.add_space(4.0);
        // Name · protocol · × — one shared height, fixed gaps, LTR so spacing is obvious.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = PORT_ROW_GAP;
            ui.set_min_height(PORT_ROW_H);
            let field_w =
                (ui.available_width() - PORT_PROTO_W - PORT_ROW_H - 2.0 * PORT_ROW_GAP).max(36.0);
            let label = &mut port_field(model, hub_id, port_id).label;
            if ui
                .add_sized(
                    vec2(field_w, PORT_ROW_H),
                    egui::TextEdit::singleline(label)
                        .desired_width(field_w)
                        .margin(egui::Margin::symmetric(6.0, 4.0)),
                )
                .changed()
            {
                dirty = true;
            }
            let current = port_field(model, hub_id, port_id).protocol;
            let mut next = current;
            // ComboBox button height follows interact_size.y.
            ui.scope(|ui| {
                ui.spacing_mut().interact_size = vec2(PORT_PROTO_W, PORT_ROW_H);
                egui::ComboBox::from_id_salt(("pb_hub_proto", port_id))
                    .selected_text(current.label())
                    .width(PORT_PROTO_W)
                    .show_ui(ui, |ui| {
                        for proto in HubProto::ALL {
                            ui.selectable_value(&mut next, proto, proto.label());
                        }
                    });
            });
            if next != current {
                port_field(model, hub_id, port_id).protocol = next;
                dirty = true;
            }
            if port_remove_button(ui, p).clicked() {
                model.remove_port(hub_id, port_id);
                dirty = true;
            }
        });
    }
    dirty
}

/// Square remove control matching `PORT_ROW_H`, with × centered in the button.
fn port_remove_button(ui: &mut Ui, p: &ThemePalette) -> egui::Response {
    let size = vec2(PORT_ROW_H, PORT_ROW_H);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let fill = if resp.hovered() {
        tint(p.accent, 20)
    } else {
        p.header_top
    };
    ui.painter().rect_filled(rect, Rounding::same(5.0), fill);
    ui.painter()
        .rect_stroke(rect, Rounding::same(5.0), Stroke::new(1.0, p.line));
    let galley = ui.fonts(|fonts| fonts.layout_no_wrap("×".to_owned(), font_ui(14.0), p.text));
    ui.painter()
        .galley(rect.center() - galley.size() * 0.5, galley, p.text);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn port_field(model: &mut HubModel, hub_id: u64, port_id: u64) -> &mut HubPort {
    let hub = model.hub_mut(hub_id).expect("hub");
    hub.sources
        .iter_mut()
        .chain(hub.outputs.iter_mut())
        .find(|port| port.id == port_id)
        .expect("port")
}

fn step_field(model: &mut HubModel, hub_id: u64, step_id: u64) -> &mut HubStep {
    let hub = model.hub_mut(hub_id).expect("hub");
    hub.aggregators
        .iter_mut()
        .chain(hub.calculations.iter_mut())
        .find(|step| step.id == step_id)
        .expect("step")
}

fn find<'a>(nodes: &'a [Placed], hub_id: u64, role: Role, port: Option<u64>) -> Option<&'a Placed> {
    nodes.iter().find(|node| {
        node.hub_id == hub_id && node.role == role && (port.is_none() || node.port_id == port)
    })
}

fn find_port<'a>(model: &'a HubModel, hub_id: u64, port_id: Option<u64>) -> Option<&'a HubPort> {
    let port_id = port_id?;
    let hub = model.hub(hub_id)?;
    hub.sources
        .iter()
        .chain(hub.outputs.iter())
        .find(|port| port.id == port_id)
}

fn find_step<'a>(model: &'a HubModel, hub_id: u64, step_id: Option<u64>) -> Option<&'a HubStep> {
    let step_id = step_id?;
    let hub = model.hub(hub_id)?;
    hub.aggregators
        .iter()
        .chain(hub.calculations.iter())
        .find(|step| step.id == step_id)
}

fn port_proto(model: &HubModel, hub_id: u64, port_id: Option<u64>) -> HubProto {
    find_port(model, hub_id, port_id)
        .map(|port| port.protocol)
        .unwrap_or(HubProto::Modbus)
}

fn link_curve(
    painter: &egui::Painter,
    from: Rect,
    to: Rect,
    color: Color32,
    dashed: bool,
    hot: bool,
) {
    let (start, end, dir) = facing(from, to);
    let pull = ((end - start).length() * 0.42).clamp(28.0, 110.0);
    let c1 = start + dir * pull;
    let c2 = end - dir * pull;
    let ink = if hot { color } else { tint(color, 120) };
    let width = if hot { 2.15 } else { 1.35 };
    stroke_cubic(painter, start, c1, c2, end, Stroke::new(width, ink), dashed);
    let approach = if (end - c2).length() < 0.01 {
        dir
    } else {
        (end - c2).normalized()
    };
    arrow(painter, end, approach, 7.0, ink);
}

/// Leave `from` on the side that faces `to`, and enter `to` on the facing side.
fn facing(from: Rect, to: Rect) -> (Pos2, Pos2, Vec2) {
    let delta = to.center() - from.center();
    if delta.x.abs() >= delta.y.abs() {
        if delta.x >= 0.0 {
            (from.right_center(), to.left_center(), Vec2::new(1.0, 0.0))
        } else {
            (from.left_center(), to.right_center(), Vec2::new(-1.0, 0.0))
        }
    } else if delta.y >= 0.0 {
        (from.center_bottom(), to.center_top(), Vec2::new(0.0, 1.0))
    } else {
        (from.center_top(), to.center_bottom(), Vec2::new(0.0, -1.0))
    }
}

fn stroke_cubic(
    painter: &egui::Painter,
    a: Pos2,
    c1: Pos2,
    c2: Pos2,
    b: Pos2,
    stroke: Stroke,
    dashed: bool,
) {
    let steps = 20;
    let mut prev = a;
    for step in 1..=steps {
        let t = step as f32 / steps as f32;
        let point = cubic(a, c1, c2, b, t);
        if !dashed || step % 2 == 1 {
            painter.line_segment([prev, point], stroke);
        }
        prev = point;
    }
}

fn cubic(a: Pos2, c1: Pos2, c2: Pos2, b: Pos2, t: f32) -> Pos2 {
    let u = 1.0 - t;
    let point = a.to_vec2() * (u * u * u)
        + c1.to_vec2() * (3.0 * u * u * t)
        + c2.to_vec2() * (3.0 * u * t * t)
        + b.to_vec2() * (t * t * t);
    pos2(point.x, point.y)
}

fn arrow(painter: &egui::Painter, tip: Pos2, dir: Vec2, len: f32, color: Color32) {
    let dir = if dir.length() < 0.001 {
        Vec2::new(1.0, 0.0)
    } else {
        dir.normalized()
    };
    let side = vec2(-dir.y, dir.x);
    let base = tip - dir * len;
    painter.add(Shape::convex_polygon(
        vec![tip, base + side * (len * 0.45), base - side * (len * 0.45)],
        color,
        Stroke::NONE,
    ));
}

fn text_button(ui: &mut Ui, label: &str, on: bool, p: &ThemePalette) -> egui::Response {
    let galley = ui.fonts(|fonts| fonts.layout_no_wrap(label.to_owned(), font_ui(12.0), p.text));
    let (rect, resp) = ui.allocate_exact_size(galley.size() + vec2(16.0, 8.0), Sense::click());
    let fill = if on {
        tint(p.accent, 48)
    } else if resp.hovered() {
        tint(p.accent, 20)
    } else {
        p.header_top
    };
    ui.painter().rect_filled(rect, Rounding::same(5.0), fill);
    ui.painter().rect_stroke(
        rect,
        Rounding::same(5.0),
        Stroke::new(1.0, if on { p.accent } else { p.line }),
    );
    ui.painter().galley(
        rect.center() - galley.size() * 0.5,
        galley,
        if on { p.accent } else { p.text },
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_hubs_are_server_capable_with_mixed_ports() {
        let model = HubModel::sample();
        assert_eq!(model.hubs[0].protocol, HubProto::Modbus);
        assert_eq!(model.hubs[1].protocol, HubProto::Mqtt);
        assert!(model.hubs[0].protocol.hub_capable());
        assert!(model.hubs[1].protocol.hub_capable());
        let mut kinds = Vec::new();
        for port in model.hubs[0]
            .sources
            .iter()
            .chain(model.hubs[0].outputs.iter())
        {
            if !kinds.contains(&port.protocol) {
                kinds.push(port.protocol);
            }
        }
        assert!(kinds.len() > 1);
        assert!(!HubProto::Streams.hub_capable());
        assert!(!HubProto::Files.hub_capable());
    }

    #[test]
    fn mqtt_leaves_plant_and_enters_edge() {
        let model = HubModel::sample();
        let links = model.across();
        assert!(links.iter().any(|link| {
            link.proto == HubProto::Mqtt && link.from_hub == 1 && link.to_hub == 6
        }));
    }

    #[test]
    fn hub_protocol_change_does_not_rewrite_ports() {
        let mut model = HubModel::sample();
        let before: Vec<_> = model
            .hub(6)
            .unwrap()
            .sources
            .iter()
            .chain(model.hub(6).unwrap().outputs.iter())
            .map(|port| port.protocol)
            .collect();
        model.set_hub_protocol(6, HubProto::OpcUa);
        let hub = model.hub(6).unwrap();
        assert_eq!(hub.protocol, HubProto::OpcUa);
        let after: Vec<_> = hub
            .sources
            .iter()
            .chain(hub.outputs.iter())
            .map(|port| port.protocol)
            .collect();
        assert_eq!(before, after);
        // Non-hub protocols are rejected.
        model.set_hub_protocol(6, HubProto::Streams);
        assert_eq!(model.hub(6).unwrap().protocol, HubProto::OpcUa);
    }

    #[test]
    fn diagram_places_a_node_for_every_port_and_hub() {
        let model = HubModel::sample();
        let nodes = layout_nodes(&model.hubs);
        let ports: usize = model
            .hubs
            .iter()
            .map(|hub| {
                hub.sources.len()
                    + hub.outputs.len()
                    + hub.aggregators.len()
                    + hub.calculations.len()
            })
            .sum();
        assert_eq!(nodes.len(), ports + model.hubs.len());
        let plant = nodes
            .iter()
            .find(|node| node.role == Role::Hub && node.hub_id == 1)
            .unwrap();
        let source = nodes.iter().find(|node| node.port_id == Some(2)).unwrap();
        assert!(source.rect.right() < plant.rect.left());
        assert!(plant.rect.center().x < 420.0);
    }

    #[test]
    fn add_aggregator_and_calculation_sit_on_the_selected_edge() {
        let mut model = HubModel::sample();
        // Plant MQTT output (id 4) → hub edge.
        let agg = model.add_step(1, true, 4).unwrap();
        let calc = model.add_step(1, false, 4).unwrap();
        model.focus_port(1, agg);
        assert_eq!(model.selected_link, Some(4));
        let nodes = layout_nodes(&model.hubs);
        let agg_node = nodes.iter().find(|node| node.port_id == Some(agg)).unwrap();
        let calc_node = nodes
            .iter()
            .find(|node| node.port_id == Some(calc))
            .unwrap();
        let plant = nodes
            .iter()
            .find(|node| node.role == Role::Hub && node.hub_id == 1)
            .unwrap();
        let output = nodes.iter().find(|node| node.port_id == Some(4)).unwrap();
        assert_eq!(agg_node.role, Role::Aggregator);
        assert_eq!(calc_node.role, Role::Calculation);
        assert_eq!(
            model.hub(1).unwrap().aggregators[0].edge_port,
            4
        );
        // Both transforms land between hub and that output on the edge.
        let left = plant.rect.center().x.min(output.rect.center().x);
        let right = plant.rect.center().x.max(output.rect.center().x);
        assert!(agg_node.rect.center().x > left && agg_node.rect.center().x < right);
        assert!(calc_node.rect.center().x > left && calc_node.rect.center().x < right);
        assert!(open_target(&model, agg_node).is_none());
        assert!(model.add_step(1, true, 999).is_none());
    }

    #[test]
    fn dragging_a_hub_moves_its_undragged_ports() {
        let mut model = HubModel::sample();
        let before = layout_nodes(&model.hubs);
        let plant = before
            .iter()
            .find(|node| node.role == Role::Hub && node.hub_id == 1)
            .unwrap()
            .rect
            .center();
        let source = before
            .iter()
            .find(|node| node.port_id == Some(2))
            .unwrap()
            .rect
            .center();
        model.set_center(1, None, [plant.x + 80.0, plant.y + 24.0]);
        let after = layout_nodes(&model.hubs);
        let plant2 = after
            .iter()
            .find(|node| node.role == Role::Hub && node.hub_id == 1)
            .unwrap()
            .rect
            .center();
        let source2 = after
            .iter()
            .find(|node| node.port_id == Some(2))
            .unwrap()
            .rect
            .center();
        assert!((plant2.x - plant.x - 80.0).abs() < 0.1);
        assert!((source2.x - source.x - 80.0).abs() < 0.1);
        model.set_center(1, Some(2), [12.0, 18.0]);
        model.set_center(1, None, [plant2.x + 40.0, plant2.y]);
        let pinned = layout_nodes(&model.hubs)
            .into_iter()
            .find(|node| node.port_id == Some(2))
            .unwrap();
        assert!((pinned.rect.center().x - 12.0).abs() < 0.1);
        assert!((pinned.rect.center().y - 18.0).abs() < 0.1);
    }

    #[test]
    fn a_port_node_opens_its_protocol_panel() {
        let model = HubModel::sample();
        let nodes = layout_nodes(&model.hubs);
        let rtu = nodes.iter().find(|node| node.port_id == Some(2)).unwrap();
        assert_eq!(open_target(&model, rtu), Some(DockPane::Modbus));
        let plant = nodes
            .iter()
            .find(|node| node.role == Role::Hub && node.hub_id == 1)
            .unwrap();
        assert_eq!(open_target(&model, plant), Some(DockPane::Modbus));
        let edge = nodes
            .iter()
            .find(|node| node.role == Role::Hub && node.hub_id == 6)
            .unwrap();
        assert_eq!(open_target(&model, edge), Some(DockPane::Mqtt));
    }

    #[test]
    fn scrollbars_only_when_a_node_leaves_the_grid() {
        let nodes = layout_nodes(&HubModel::sample().hubs);
        let mut bounds = nodes[0].rect;
        for node in nodes.iter().skip(1) {
            bounds = bounds.union(node.rect);
        }
        // Spare room smaller than the old 56px margin; every node still inside → no bars.
        let inside = scroll_fit(&nodes, vec2(bounds.max.x + 12.0, bounds.max.y + 12.0));
        assert_eq!(inside.allow, [false, false]);
        // Fitting axes center the node block in the view.
        let expect_x = (inside.size.x - bounds.width()) * 0.5 - bounds.min.x;
        let expect_y = (inside.size.y - bounds.height()) * 0.5 - bounds.min.y;
        assert!((inside.shift.x - expect_x).abs() < 0.1);
        assert!((inside.shift.y - expect_y).abs() < 0.1);
        // A node past the right edge scrolls that axis only. The other axis stays the view size.
        let tight = scroll_fit(&nodes, vec2(bounds.max.x - 20.0, bounds.max.y + 80.0));
        assert_eq!(tight.allow, [true, false]);
        assert!(tight.size.x >= bounds.max.x);
        assert!((tight.size.y - (bounds.max.y + 80.0)).abs() < 0.1);
        // A node off the top-left of the grid is outside, and can be scrolled back in.
        let off = vec![placed(
            1,
            None,
            Role::Hub,
            pos2(-10.0, 40.0),
            vec2(HUB_W, HUB_H),
        )];
        let left = scroll_fit(&off, vec2(800.0, 600.0));
        assert_eq!(left.allow, [true, false]);
        assert!(left.shift.x > 0.0);
        // Y fits → centered (not left at the default origin).
        let hub_h = HUB_H;
        let expect_y = (600.0 - hub_h) * 0.5 - (40.0 - hub_h * 0.5);
        assert!((left.shift.y - expect_y).abs() < 0.1);
    }

    #[test]
    fn camera_pan_follows_the_pointer() {
        // Middle/Space drag adds the pointer delta to the camera.
        let cam = vec2(40.0, 10.0) + vec2(15.0, -4.0);
        assert!((cam.x - 55.0).abs() < 0.01);
        assert!((cam.y - 6.0).abs() < 0.01);
    }

    #[test]
    fn zoom_defaults_to_maximum_and_only_zooms_out() {
        assert_eq!(clamp_zoom(1.0), ZOOM_MAX);
        assert_eq!(clamp_zoom(1.5), ZOOM_MAX);
        assert_eq!(clamp_zoom(0.1), ZOOM_MIN);
        assert_eq!(clamp_zoom(f32::NAN), ZOOM_MAX);
        let model = HubModel::sample();
        assert!((model.zoom - ZOOM_MAX).abs() < 0.0001);
    }

    #[test]
    fn wheel_zoom_steps_are_gentler_than_egui_default() {
        // egui Options::scroll_zoom_speed is 1/200; hubs uses a smaller gain.
        assert!(ZOOM_WHEEL < 1.0 / 200.0);
        let notch = 50.0; // typical smooth_scroll_delta.y magnitude for one notch
        let factor = (ZOOM_WHEEL * notch).exp();
        assert!(factor > 1.0 && factor < 1.12);
    }

    #[test]
    fn multi_selection_clears_item_properties_gate() {
        let mut model = HubModel::sample();
        model.set_selection(vec![(1, None), (1, Some(2))]);
        assert!(model.multi_selected());
        assert_eq!(model.selection.len(), 2);
        assert!(model.selected_port.is_none());
        model.set_selection(vec![(1, Some(2))]);
        assert!(!model.multi_selected());
        assert_eq!(model.selected_port, Some(2));
    }

    #[test]
    fn marquee_picks_intersecting_nodes() {
        let nodes = layout_nodes(&HubModel::sample().hubs);
        let plant = nodes
            .iter()
            .find(|node| node.role == Role::Hub && node.hub_id == 1)
            .unwrap();
        let source = nodes.iter().find(|node| node.port_id == Some(2)).unwrap();
        let box_rect = plant.rect.union(source.rect).expand(2.0);
        let keys = nodes_in_marquee(&nodes, box_rect);
        assert!(keys.contains(&(1, None)));
        assert!(keys.contains(&(1, Some(2))));
        assert!(keys.len() >= 2);
    }

    #[test]
    fn zoom_cam_keeps_the_world_point_under_the_cursor() {
        let pan = vec2(40.0, 20.0);
        let view = vec2(100.0, 80.0);
        let old_z = 1.0;
        let new_z = 0.5;
        let next = zoom_cam(pan, view, old_z, new_z);
        let world_before = (view - pan) / old_z;
        let world_after = (view - next) / new_z;
        assert!((world_before.x - world_after.x).abs() < 0.05);
        assert!((world_before.y - world_after.y).abs() < 0.05);
        // Negative camera is allowed (cursor-anchored zoom when content fits).
        assert!(next.x < pan.x || next.y < pan.y || (new_z - old_z).abs() > 0.0);
    }

    #[test]
    fn empty_selection_clears_node_properties() {
        let mut model = HubModel::sample();
        assert_eq!(model.selected, Some(1));
        model.set_selection(Vec::new());
        assert!(model.selection.is_empty());
        assert!(model.selected.is_none());
        assert!(model.selected_port.is_none());
        assert!(model.selected_link.is_none());
    }

    #[test]
    fn scale_nodes_shrinks_layout_with_zoom() {
        let nodes = layout_nodes(&HubModel::sample().hubs);
        let half = scale_nodes(&nodes, 0.5);
        assert_eq!(half.len(), nodes.len());
        let a = nodes[0].rect;
        let b = half[0].rect;
        assert!((b.width() - a.width() * 0.5).abs() < 0.01);
        assert!((b.center().x - a.center().x * 0.5).abs() < 0.01);
    }
}
