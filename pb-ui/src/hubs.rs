//! Hub diagram. Each hub is a conversion point: sources in, then aggregate or calculate, outputs out. No protocol I/O.

use egui::{
    pos2, vec2, Align2, Color32, CursorIcon, Id, Layout, Pos2, Rect, Rounding, ScrollArea, Sense,
    Shape, Stroke, Ui, Vec2,
};

use crate::dock::DockPane;
use crate::theme::{blend_scope, font_semi, font_ui, tint, ThemePalette};

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
const INSPECT_SPLIT: f32 = 6.0;

fn default_inspector_width() -> f32 {
    INSPECT_DEFAULT
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

    fn ink(self, light: bool) -> Color32 {
        self.pane().ink(light)
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct HubPort {
    pub id: u64,
    pub protocol: HubProto,
    pub label: String,
    /// Center on the diagram, once the user has dragged this node.
    #[serde(default)]
    pub at: Option<[f32; 2]>,
}

/// Aggregate or calculation step on a hub. Not a protocol endpoint.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct HubStep {
    pub id: u64,
    pub label: String,
    /// Center on the diagram, once the user has dragged this node.
    #[serde(default)]
    pub at: Option<[f32; 2]>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Hub {
    pub id: u64,
    pub name: String,
    /// Set when the hub is one protocol. None means the hub is split across its links.
    pub protocol: Option<HubProto>,
    pub sources: Vec<HubPort>,
    pub outputs: Vec<HubPort>,
    /// After sources come in: roll values up before they leave.
    #[serde(default)]
    pub aggregators: Vec<HubStep>,
    /// After sources come in: derived values before they leave.
    #[serde(default)]
    pub calculations: Vec<HubStep>,
    /// Center on the diagram, once the user has dragged this hub.
    #[serde(default)]
    pub at: Option<[f32; 2]>,
}

impl Hub {
    pub fn split(&self) -> bool {
        self.protocol.is_none()
    }

    /// Source, output, aggregator, or calculation id on this hub.
    fn has_node(&self, id: u64) -> bool {
        self.sources.iter().any(|port| port.id == id)
            || self.outputs.iter().any(|port| port.id == id)
            || self.aggregators.iter().any(|step| step.id == id)
            || self.calculations.iter().any(|step| step.id == id)
    }

    fn process_count(&self) -> usize {
        self.aggregators.len() + self.calculations.len()
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
    /// Set when a source or output node is the one showing details.
    #[serde(default)]
    pub selected_port: Option<u64>,
    /// Width of the right-hand configure panel when the hubs view is wide enough.
    #[serde(default = "default_inspector_width")]
    pub inspector_width: f32,
    #[serde(default)]
    next_id: u64,
}

impl Default for HubModel {
    fn default() -> Self {
        Self::sample()
    }
}

impl HubModel {
    /// Two hubs so the diagram has a split conversion and a single-protocol hub.
    pub fn sample() -> Self {
        Self {
            next_id: 20,
            selected: Some(1),
            selected_port: None,
            inspector_width: INSPECT_DEFAULT,
            hubs: vec![
                Hub {
                    id: 1,
                    name: "Plant".into(),
                    protocol: None,
                    sources: vec![
                        HubPort {
                            id: 2,
                            protocol: HubProto::Modbus,
                            label: "RTU line".into(),
                            at: None,
                        },
                        HubPort {
                            id: 3,
                            protocol: HubProto::Streams,
                            label: "Serial A".into(),
                            at: None,
                        },
                    ],
                    outputs: vec![
                        HubPort {
                            id: 4,
                            protocol: HubProto::Mqtt,
                            label: "Plant topics".into(),
                            at: None,
                        },
                        HubPort {
                            id: 5,
                            protocol: HubProto::OpcUa,
                            label: "SCADA".into(),
                            at: None,
                        },
                    ],
                    aggregators: vec![],
                    calculations: vec![],
                    at: None,
                },
                Hub {
                    id: 6,
                    name: "Edge".into(),
                    protocol: Some(HubProto::Mqtt),
                    sources: vec![HubPort {
                        id: 7,
                        protocol: HubProto::Mqtt,
                        label: "Local broker".into(),
                        at: None,
                    }],
                    outputs: vec![HubPort {
                        id: 8,
                        protocol: HubProto::Mqtt,
                        label: "Upstream".into(),
                        at: None,
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
            if let Some(proto) = hub.protocol {
                for port in hub.sources.iter_mut().chain(hub.outputs.iter_mut()) {
                    port.protocol = proto;
                }
            }
        }
        if self.selected.is_some_and(|id| self.hub(id).is_none()) {
            self.selected = self.hubs.first().map(|hub| hub.id);
            self.selected_port = None;
        }
        if !self.inspector_width.is_finite() || self.inspector_width < INSPECT_MIN {
            self.inspector_width = INSPECT_DEFAULT;
        }
        self.keep_focus();
    }

    /// Drop a port or step selection that no longer belongs to the open hub.
    fn keep_focus(&mut self) {
        let Some(hub_id) = self.selected else {
            self.selected_port = None;
            return;
        };
        let Some(port_id) = self.selected_port else {
            return;
        };
        let belongs = self.hub(hub_id).is_some_and(|hub| hub.has_node(port_id));
        if !belongs {
            self.selected_port = None;
        }
    }

    pub fn focus_hub(&mut self, id: u64) {
        self.selected = Some(id);
        self.selected_port = None;
    }

    pub fn focus_port(&mut self, hub_id: u64, port_id: u64) {
        self.selected = Some(hub_id);
        self.selected_port = Some(port_id);
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

    /// Put every node back on the automatic arrangement.
    pub fn clear_arrangement(&mut self) {
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
            protocol: None,
            sources: vec![HubPort {
                id: source,
                protocol: HubProto::Modbus,
                label: "Source".into(),
                at: None,
            }],
            outputs: vec![HubPort {
                id: output,
                protocol: HubProto::Mqtt,
                label: "Output".into(),
                at: None,
            }],
            aggregators: vec![],
            calculations: vec![],
            at: None,
        });
        self.selected = Some(id);
        self.selected_port = None;
    }

    pub fn remove_hub(&mut self, id: u64) {
        self.hubs.retain(|hub| hub.id != id);
        if self.selected == Some(id) {
            self.selected = self.hubs.first().map(|hub| hub.id);
            self.selected_port = None;
        }
        self.keep_focus();
    }

    pub fn set_split(&mut self, id: u64, split: bool) {
        let Some(hub) = self.hub_mut(id) else {
            return;
        };
        if split {
            hub.protocol = None;
            return;
        }
        let proto = hub
            .protocol
            .or_else(|| hub.sources.first().map(|port| port.protocol))
            .unwrap_or(HubProto::Modbus);
        hub.protocol = Some(proto);
        for port in hub.sources.iter_mut().chain(hub.outputs.iter_mut()) {
            port.protocol = proto;
        }
    }

    pub fn set_hub_protocol(&mut self, id: u64, proto: HubProto) {
        let Some(hub) = self.hub_mut(id) else {
            return;
        };
        hub.protocol = Some(proto);
        for port in hub.sources.iter_mut().chain(hub.outputs.iter_mut()) {
            port.protocol = proto;
        }
    }

    /// Append a source or output and return its id.
    pub fn add_port(&mut self, hub_id: u64, source: bool) -> Option<u64> {
        let proto = self
            .hub(hub_id)
            .and_then(|hub| hub.protocol)
            .unwrap_or(if source {
                HubProto::Modbus
            } else {
                HubProto::Mqtt
            });
        let id = self.alloc();
        let Some(hub) = self.hub_mut(hub_id) else {
            return None;
        };
        let port = HubPort {
            id,
            protocol: proto,
            label: if source { "Source" } else { "Output" }.into(),
            at: None,
        };
        if source {
            hub.sources.push(port);
        } else {
            hub.outputs.push(port);
        }
        Some(id)
    }

    /// Append an aggregator or calculation and return its id.
    pub fn add_step(&mut self, hub_id: u64, aggregator: bool) -> Option<u64> {
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
            hub.aggregators.retain(|step| step.id != port_id);
            hub.calculations.retain(|step| step.id != port_id);
        }
        if self.selected_port == Some(port_id) {
            self.selected_port = None;
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

/// Compact columns. A saved drag wins; an undragged port stays beside its hub.
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
        for step in &hub.aggregators {
            place_dragged(
                &mut nodes,
                &defaults,
                origin,
                center,
                hub.id,
                step.id,
                Role::Aggregator,
                step.at,
            );
        }
        for step in &hub.calculations {
            place_dragged(
                &mut nodes,
                &defaults,
                origin,
                center,
                hub.id,
                step.id,
                Role::Calculation,
                step.at,
            );
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
    let mut y = MARGIN;
    let mut spots = Vec::new();
    for hub in hubs {
        let proc = hub.process_count();
        let rows = hub
            .sources
            .len()
            .max(hub.outputs.len())
            .max(proc)
            .max(1) as f32;
        let stack = rows * PORT_H + (rows - 1.0).max(0.0) * GAP;
        let band = stack.max(HUB_H) + 36.0;
        let mid = y + band * 0.5;
        // Process column only when this hub has aggregators or calculations.
        let proc_x = hub_x + HUB_W * 0.5 + COL_GAP + PORT_W * 0.5;
        let out_x = if proc > 0 {
            proc_x + PORT_W * 0.5 + COL_GAP + PORT_W * 0.5
        } else {
            proc_x
        };
        spots.push(Spot {
            hub_id: hub.id,
            port_id: None,
            role: Role::Hub,
            center: pos2(hub_x, mid),
        });
        push_port_spots(&mut spots, &hub.sources, Role::Source, src_x, mid, hub.id);
        if proc > 0 {
            let mut ids = Vec::with_capacity(proc);
            ids.extend(hub.aggregators.iter().map(|step| (step.id, Role::Aggregator)));
            ids.extend(
                hub.calculations
                    .iter()
                    .map(|step| (step.id, Role::Calculation)),
            );
            push_id_spots(&mut spots, &ids, proc_x, mid, hub.id);
        }
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
    allow: [bool; 2],
    /// Content size. This stays the view size on an axis where every node already fits.
    size: Vec2,
    /// Moves a node that starts before the grid origin into scrollable space.
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

/// Pan offset. An axis that already fits stays at zero so a stale scrollbar cannot shift the canvas.
fn pinned_scroll(current: Vec2, pan: Option<Vec2>, allow: [bool; 2]) -> Option<Vec2> {
    let mut offset = pan.unwrap_or(current);
    if !allow[0] {
        offset.x = 0.0;
    }
    if !allow[1] {
        offset.y = 0.0;
    }
    let changed = (offset.x - current.x).abs() > 0.01 || (offset.y - current.y).abs() > 0.01;
    if pan.is_some() || changed {
        Some(offset)
    } else {
        None
    }
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
pub fn paint(ui: &mut Ui, model: &mut HubModel, p: &ThemePalette) -> HubChange {
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
                    "Drag a node to place it. Click for details. Right-click to add. Middle-drag or Space-drag pans.",
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
        let gap = INSPECT_SPLIT;
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
            change = change.join(inspector(ui, model, inspect_w, height, p));
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
fn hub_inspect_splitter(ui: &mut Ui, height: f32, gap: f32, p: &ThemePalette) -> SplitCmd {
    let (rect, resp) = ui.allocate_exact_size(vec2(gap, height), Sense::click_and_drag());
    let fill = if resp.hovered() || resp.dragged() {
        p.accent
    } else {
        tint(p.line, 153)
    };
    let bar = Rect::from_center_size(rect.center(), vec2(2.0, (height - 8.0).max(2.0)));
    ui.painter()
        .rect_filled(bar, Rounding::same(1.0), fill);
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
    painter.rect_filled(rect, Rounding::same(8.0), blend_scope(p.bg));
    paint_grid(&painter, rect);
    painter.rect_stroke(
        rect,
        Rounding::same(8.0),
        Stroke::new(1.0, tint(p.accent, 150)),
    );
    // The scrollport is the visible grid. Insetting it would count chrome as overflow.
    let grid = rect.shrink(1.0);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(grid)
            .layout(Layout::top_down(egui::Align::Min)),
    );
    child.set_clip_rect(grid);
    let mut change = HubChange::quiet();
    let scroll_id = child.make_persistent_id(Id::new("pb_hub_diagram"));
    let pan = canvas_pan(&child, rect, scroll_id);
    let fit = scroll_fit(&layout_nodes(&model.hubs), grid.size());
    let allow = fit.allow;
    let mut scroll = ScrollArea::both()
        .id_salt("pb_hub_diagram")
        .auto_shrink([false, false])
        .drag_to_scroll(false)
        .scroll(allow);
    if !allow[0] && !allow[1] {
        scroll = scroll.scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden);
    }
    let stored = egui::scroll_area::State::load(child.ctx(), scroll_id)
        .map(|state| state.offset)
        .unwrap_or(Vec2::ZERO);
    if let Some(offset) = pinned_scroll(stored, pan, allow) {
        scroll = scroll.scroll_offset(offset);
    }
    let content_shift = fit.shift;
    let content_size = fit.size;
    scroll.show(&mut child, |ui| {
        let (area, canvas) = ui.allocate_exact_size(content_size, Sense::click());
        if model.hubs.is_empty() {
            ui.painter().text(
                area.center(),
                Align2::CENTER_CENTER,
                "No hubs yet — right-click or use New hub",
                font_ui(14.0),
                p.muted,
            );
            if let Some(cmd) = canvas_menu(&canvas, None) {
                apply_menu(model, cmd, &mut change);
            }
            return;
        }
        let nodes = layout_nodes(&model.hubs);
        let shift = area.min.to_vec2() + content_shift;
        let screen: Vec<Placed> = nodes
            .iter()
            .map(|node| Placed {
                rect: node.rect.translate(shift),
                ..*node
            })
            .collect();
        let gesture = gesture_nodes(ui, model, &screen, pan.is_some());
        if let Some((hub_id, port_id, delta)) = gesture.drag {
            let center = screen
                .iter()
                .find(|node| node.hub_id == hub_id && node.port_id == port_id)
                .map(|node| node.rect.center() - shift + delta);
            if let Some(center) = center {
                model.set_center(hub_id, port_id, [center.x.max(24.0), center.y.max(24.0)]);
                change.dirty = true;
            }
        }
        if let Some((hub_id, port_id)) = gesture.pick {
            match port_id {
                Some(port_id) => model.focus_port(hub_id, port_id),
                None => model.focus_hub(hub_id),
            }
            change.dirty = true;
        }
        if let Some(pane) = gesture.open {
            change.open = Some(pane);
        }
        let target = model
            .selected
            .or_else(|| model.hubs.first().map(|hub| hub.id));
        let menu = gesture.menu.or_else(|| canvas_menu(&canvas, target));
        if let Some(cmd) = menu {
            apply_menu(model, cmd, &mut change);
        }
        let nodes = layout_nodes(&model.hubs);
        let screen: Vec<Placed> = nodes
            .iter()
            .map(|node| Placed {
                rect: node.rect.translate(shift),
                ..*node
            })
            .collect();
        paint_flow(ui.painter(), &screen, model, gesture.hover, p);
    });
    if pan.is_some() {
        child.ctx().set_cursor_icon(egui::CursorIcon::AllScroll);
    }
    change
}

/// Middle-drag, or Space+left-drag, pans like a Fusion node editor.
/// Returns the scroll offset to apply this frame.
fn canvas_pan(ui: &Ui, rect: Rect, scroll_id: Id) -> Option<Vec2> {
    let typing = ui.ctx().wants_keyboard_input();
    let delta = ui.ctx().input(|i| {
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
    })?;
    let current = egui::scroll_area::State::load(ui.ctx(), scroll_id)
        .map(|state| state.offset)
        .unwrap_or(Vec2::ZERO);
    Some(pan_scroll(current, delta))
}

/// Drag the canvas with the pointer. Offset cannot go negative.
fn pan_scroll(offset: Vec2, delta: Vec2) -> Vec2 {
    Vec2::new((offset.x - delta.x).max(0.0), (offset.y - delta.y).max(0.0))
}

enum MenuCmd {
    AddAggregator(u64),
    AddCalculation(u64),
    AddSource(u64),
    AddOutput(u64),
    NewHub,
}

fn apply_menu(model: &mut HubModel, cmd: MenuCmd, change: &mut HubChange) {
    match cmd {
        MenuCmd::AddAggregator(hub_id) => {
            if let Some(step_id) = model.add_step(hub_id, true) {
                model.focus_port(hub_id, step_id);
                change.dirty = true;
            }
        }
        MenuCmd::AddCalculation(hub_id) => {
            if let Some(step_id) = model.add_step(hub_id, false) {
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

/// Shared top of every hubs context menu: aggregator and calculation first.
fn hub_add_menu(ui: &mut Ui, hub_id: u64, cmd: &mut Option<MenuCmd>) {
    if ui.button("Add aggregator").clicked() {
        *cmd = Some(MenuCmd::AddAggregator(hub_id));
        ui.close_menu();
    }
    if ui.button("Add calculation").clicked() {
        *cmd = Some(MenuCmd::AddCalculation(hub_id));
        ui.close_menu();
    }
    if ui.button("Add source").clicked() {
        *cmd = Some(MenuCmd::AddSource(hub_id));
        ui.close_menu();
    }
    if ui.button("Add output").clicked() {
        *cmd = Some(MenuCmd::AddOutput(hub_id));
        ui.close_menu();
    }
}

/// Right-click the empty canvas. `hub_id` is who receives a new node.
fn canvas_menu(resp: &egui::Response, hub_id: Option<u64>) -> Option<MenuCmd> {
    let mut cmd = None;
    resp.context_menu(|ui| {
        ui.set_min_width(168.0);
        if let Some(hub_id) = hub_id {
            hub_add_menu(ui, hub_id, &mut cmd);
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
}

fn gesture_nodes(ui: &mut Ui, model: &HubModel, nodes: &[Placed], panning: bool) -> Gesture {
    let mut gesture = Gesture {
        hover: None,
        pick: None,
        drag: None,
        open: None,
        menu: None,
    };
    for node in nodes {
        let id = Id::new((
            "pb_hub_node",
            node.hub_id,
            node.port_id.unwrap_or(0),
            node.role as u8,
        ));
        let opens = open_target(model, node).is_some();
        let tip = if opens {
            "Drag to move. Click for details. Double-click opens the protocol panel. Right-click to add nodes."
        } else {
            "Drag to move. Click for details. Right-click to add nodes."
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
            hub_add_menu(ui, node.hub_id, &mut menu);
        });
        if menu.is_some() {
            gesture.menu = menu;
        }
    }
    gesture
}

/// Protocol panel for this node, when the node is one protocol.
fn open_target(model: &HubModel, node: &Placed) -> Option<DockPane> {
    match node.role {
        Role::Hub => model
            .hub(node.hub_id)
            .and_then(|hub| hub.protocol)
            .map(HubProto::pane),
        Role::Source | Role::Output => {
            find_port(model, node.hub_id, node.port_id).map(|port| port.protocol.pane())
        }
        Role::Aggregator | Role::Calculation => None,
    }
}

fn paint_grid(painter: &egui::Painter, rect: Rect) {
    let step = 18.0;
    let mut x = rect.min.x;
    while x < rect.max.x {
        painter.line_segment(
            [pos2(x, rect.min.y), pos2(x, rect.max.y)],
            Stroke::new(1.0, crate::theme::scope::grid()),
        );
        x += step;
    }
    let mut y = rect.min.y;
    while y < rect.max.y {
        painter.line_segment(
            [pos2(rect.min.x, y), pos2(rect.max.x, y)],
            Stroke::new(1.0, crate::theme::scope::grid()),
        );
        y += step;
    }
}

fn paint_flow(
    painter: &egui::Painter,
    nodes: &[Placed],
    model: &HubModel,
    hover: Option<(u64, Option<u64>)>,
    p: &ThemePalette,
) {
    for node in nodes {
        if node.role != Role::Source {
            continue;
        }
        let Some(hub) = find(nodes, node.hub_id, Role::Hub, None) else {
            continue;
        };
        let hot = link_hot(node, hub, hover, model);
        link_curve(
            painter,
            node.rect,
            hub.rect,
            port_proto(model, node.hub_id, node.port_id).ink(p.is_light),
            false,
            hot,
        );
    }
    for node in nodes {
        if node.role != Role::Output
            && node.role != Role::Aggregator
            && node.role != Role::Calculation
        {
            continue;
        }
        let Some(hub) = find(nodes, node.hub_id, Role::Hub, None) else {
            continue;
        };
        let hot = link_hot(hub, node, hover, model);
        let ink = match node.role {
            Role::Output => port_proto(model, node.hub_id, node.port_id).ink(p.is_light),
            _ => p.accent,
        };
        link_curve(painter, hub.rect, node.rect, ink, false, hot);
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
        let on = selected_node(model, node);
        match node.role {
            Role::Hub => paint_hub_node(painter, node, model, on, hot, p),
            Role::Source | Role::Output => paint_port_node(painter, node, model, on, hot, p),
            Role::Aggregator | Role::Calculation => {
                paint_step_node(painter, node, model, on, hot, p)
            }
        }
    }
}

fn hovered(node: &Placed, hover: Option<(u64, Option<u64>)>) -> bool {
    hover.is_some_and(|(hub, port)| node.hub_id == hub && node.port_id == port)
}

fn selected_node(model: &HubModel, node: &Placed) -> bool {
    if model.selected != Some(node.hub_id) {
        return false;
    }
    match node.role {
        Role::Hub => model.selected_port.is_none(),
        Role::Source | Role::Output | Role::Aggregator | Role::Calculation => {
            model.selected_port == node.port_id
        }
    }
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
    p: &ThemePalette,
) {
    let Some(hub) = model.hub(node.hub_id) else {
        return;
    };
    let ink = hub
        .protocol
        .map(|proto| proto.ink(p.is_light))
        .unwrap_or(p.accent);
    paint_card(painter, node.rect, ink, on, hot, p);
    paint_grip(painter, node.rect, if hot || on { ink } else { p.muted });
    let clip = Rect::from_min_max(
        pos2(node.rect.min.x + 18.0, node.rect.min.y + 4.0),
        pos2(node.rect.max.x - 8.0, node.rect.max.y - 4.0),
    );
    painter.with_clip_rect(clip).text(
        pos2(clip.min.x, node.rect.center().y - 9.0),
        Align2::LEFT_CENTER,
        &hub.name,
        font_semi(14.0),
        p.text,
    );
    let mode = hub.protocol.map(HubProto::label).unwrap_or("Split");
    painter.with_clip_rect(clip).text(
        pos2(clip.min.x, node.rect.center().y + 10.0),
        Align2::LEFT_CENTER,
        mode,
        font_ui(11.0),
        if on { ink } else { p.muted },
    );
}

fn paint_port_node(
    painter: &egui::Painter,
    node: &Placed,
    model: &HubModel,
    on: bool,
    hot: bool,
    p: &ThemePalette,
) {
    let Some(port) = find_port(model, node.hub_id, node.port_id) else {
        return;
    };
    let ink = port.protocol.ink(p.is_light);
    paint_card(painter, node.rect, ink, on, hot, p);
    let bar = Rect::from_min_max(node.rect.min, pos2(node.rect.min.x + 4.0, node.rect.max.y));
    painter.rect_filled(
        bar,
        Rounding {
            nw: 7.0,
            ne: 0.0,
            sw: 7.0,
            se: 0.0,
        },
        ink,
    );
    let clip = Rect::from_min_max(
        pos2(node.rect.min.x + 12.0, node.rect.min.y),
        pos2(node.rect.max.x - 8.0, node.rect.max.y),
    );
    let role = if node.role == Role::Source {
        "IN"
    } else {
        "OUT"
    };
    painter.with_clip_rect(clip).text(
        pos2(clip.min.x, node.rect.center().y - 8.0),
        Align2::LEFT_CENTER,
        format!("{role}  {}", port.protocol.label()),
        font_ui(10.0),
        ink,
    );
    painter.with_clip_rect(clip).text(
        pos2(clip.min.x, node.rect.center().y + 8.0),
        Align2::LEFT_CENTER,
        &port.label,
        font_ui(13.0),
        p.text,
    );
}

fn paint_step_node(
    painter: &egui::Painter,
    node: &Placed,
    model: &HubModel,
    on: bool,
    hot: bool,
    p: &ThemePalette,
) {
    let Some(step) = find_step(model, node.hub_id, node.port_id) else {
        return;
    };
    let ink = p.accent;
    paint_card(painter, node.rect, ink, on, hot, p);
    let bar = Rect::from_min_max(node.rect.min, pos2(node.rect.min.x + 4.0, node.rect.max.y));
    painter.rect_filled(
        bar,
        Rounding {
            nw: 7.0,
            ne: 0.0,
            sw: 7.0,
            se: 0.0,
        },
        ink,
    );
    let clip = Rect::from_min_max(
        pos2(node.rect.min.x + 12.0, node.rect.min.y),
        pos2(node.rect.max.x - 8.0, node.rect.max.y),
    );
    let role = if node.role == Role::Aggregator {
        "AGG"
    } else {
        "CALC"
    };
    painter.with_clip_rect(clip).text(
        pos2(clip.min.x, node.rect.center().y - 8.0),
        Align2::LEFT_CENTER,
        role,
        font_ui(10.0),
        ink,
    );
    painter.with_clip_rect(clip).text(
        pos2(clip.min.x, node.rect.center().y + 8.0),
        Align2::LEFT_CENTER,
        &step.label,
        font_ui(13.0),
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
            change = change.join(edit_selected(ui, model, p));
        });
    change
}

fn edit_selected(ui: &mut Ui, model: &mut HubModel, p: &ThemePalette) -> HubChange {
    if model.selected_port.is_some() {
        let hub_id = model.selected;
        let port_id = model.selected_port;
        if hub_id
            .zip(port_id)
            .is_some_and(|(hub_id, port_id)| find_step(model, hub_id, Some(port_id)).is_some())
        {
            return edit_step(ui, model, p);
        }
        return edit_port(ui, model, p);
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
        egui::RichText::new(format!("On hub {hub_name}"))
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

fn edit_port(ui: &mut Ui, model: &mut HubModel, p: &ThemePalette) -> HubChange {
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
    let kind = if incoming { "SOURCE" } else { "OUTPUT" };
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
    let locked = model.hub(hub_id).and_then(|hub| hub.protocol).is_some();
    if !locked {
        ui.add_space(6.0);
        let current = port_field(model, hub_id, port_id).protocol;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            for proto in HubProto::ALL {
                if text_button(ui, proto.label(), proto == current, p).clicked() && proto != current
                {
                    port_field(model, hub_id, port_id).protocol = proto;
                    change.dirty = true;
                }
            }
        });
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
    let split = model.hub(id).unwrap().split();
    ui.horizontal(|ui| {
        if text_button(ui, "Split", split, p).clicked() && !split {
            model.set_split(id, true);
            change.dirty = true;
        }
        if text_button(ui, "One protocol", !split, p).clicked() && split {
            model.set_split(id, false);
            change.dirty = true;
        }
    });
    if let Some(current) = model.hub(id).unwrap().protocol {
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            for proto in HubProto::ALL {
                if text_button(ui, proto.label(), proto == current, p).clicked() && proto != current
                {
                    model.set_hub_protocol(id, proto);
                    change.dirty = true;
                }
            }
        });
        ui.add_space(8.0);
        if text_button(ui, &format!("Open {} panel", current.label()), true, p).clicked() {
            change.open = Some(current.pane());
        }
    }
    ui.add_space(10.0);
    if port_section(ui, model, id, true, p) {
        change.dirty = true;
    }
    ui.add_space(8.0);
    if step_section(ui, model, id, true, p) {
        change.dirty = true;
    }
    ui.add_space(8.0);
    if step_section(ui, model, id, false, p) {
        change.dirty = true;
    }
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
    let locked = model.hub(hub_id).and_then(|hub| hub.protocol).is_some();
    for port_id in ports {
        ui.add_space(4.0);
        // Place × first so it stays inside the panel, then the protocol, then the name in what remains.
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                if text_button(ui, "×", false, p).clicked() {
                    model.remove_port(hub_id, port_id);
                    dirty = true;
                }
                if !locked {
                    let current = port_field(model, hub_id, port_id).protocol;
                    let mut next = current;
                    egui::ComboBox::from_id_salt(("pb_hub_proto", port_id))
                        .selected_text(current.label())
                        .width(88.0)
                        .show_ui(ui, |ui| {
                            for proto in HubProto::ALL {
                                ui.selectable_value(&mut next, proto, proto.label());
                            }
                        });
                    if next != current {
                        port_field(model, hub_id, port_id).protocol = next;
                        dirty = true;
                    }
                }
                let field_w = (ui.available_width() - 2.0).max(36.0);
                let label = &mut port_field(model, hub_id, port_id).label;
                if ui
                    .add(egui::TextEdit::singleline(label).desired_width(field_w))
                    .changed()
                {
                    dirty = true;
                }
            });
        });
    }
    dirty
}

fn step_section(
    ui: &mut Ui,
    model: &mut HubModel,
    hub_id: u64,
    aggregator: bool,
    p: &ThemePalette,
) -> bool {
    let mut dirty = false;
    let title = if aggregator {
        "AGGREGATORS"
    } else {
        "CALCULATIONS"
    };
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(title)
                .font(font_ui(10.5))
                .color(p.muted),
        );
        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            if text_button(ui, "Add", false, p).clicked() {
                model.add_step(hub_id, aggregator);
                dirty = true;
            }
        });
    });
    let steps: Vec<u64> = model
        .hub(hub_id)
        .map(|hub| {
            let list = if aggregator {
                &hub.aggregators
            } else {
                &hub.calculations
            };
            list.iter().map(|step| step.id).collect()
        })
        .unwrap_or_default();
    for step_id in steps {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                if text_button(ui, "×", false, p).clicked() {
                    model.remove_port(hub_id, step_id);
                    dirty = true;
                }
                let field_w = (ui.available_width() - 2.0).max(36.0);
                let label = &mut step_field(model, hub_id, step_id).label;
                if ui
                    .add(egui::TextEdit::singleline(label).desired_width(field_w))
                    .changed()
                {
                    dirty = true;
                }
            });
        });
    }
    dirty
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
    fn sample_shows_a_split_hub_and_a_single_protocol_hub() {
        let model = HubModel::sample();
        assert!(model.hubs[0].split());
        assert_eq!(model.hubs[1].protocol, Some(HubProto::Mqtt));
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
    fn one_protocol_rewrites_every_port() {
        let mut model = HubModel::sample();
        model.set_hub_protocol(6, HubProto::OpcUa);
        let hub = model.hub(6).unwrap();
        assert!(hub
            .sources
            .iter()
            .chain(hub.outputs.iter())
            .all(|port| port.protocol == HubProto::OpcUa));
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
    fn add_aggregator_and_calculation_create_canvas_nodes() {
        let mut model = HubModel::sample();
        let agg = model.add_step(1, true).unwrap();
        let calc = model.add_step(1, false).unwrap();
        model.focus_port(1, agg);
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
        assert!(plant.rect.right() < agg_node.rect.left());
        assert!(agg_node.rect.right() < output.rect.left());
        assert!(open_target(&model, agg_node).is_none());
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
        assert!(open_target(&model, plant).is_none());
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
        // A pan on an axis that fits is dropped, so a leftover offset cannot invent a bar.
        let pinned = pinned_scroll(vec2(30.0, 12.0), Some(vec2(10.0, 4.0)), [true, false]);
        assert_eq!(pinned, Some(vec2(10.0, 0.0)));
        assert!(pinned_scroll(Vec2::ZERO, None, [false, false]).is_none());
    }

    #[test]
    fn panning_moves_the_view_with_the_pointer() {
        let next = pan_scroll(vec2(40.0, 10.0), vec2(15.0, -4.0));
        assert!((next.x - 25.0).abs() < 0.01);
        assert!((next.y - 14.0).abs() < 0.01);
        let clamped = pan_scroll(vec2(4.0, 0.0), vec2(20.0, 8.0));
        assert_eq!(clamped, Vec2::ZERO);
    }
}
