//! Hub diagram. Each hub is a conversion point: sources in, outputs out. No protocol I/O.

use egui::{
    pos2, vec2, Align2, Color32, Id, Layout, Pos2, Rect, Rounding, ScrollArea, Sense, Shape,
    Stroke, Ui, Vec2,
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

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HubPort {
    pub id: u64,
    pub protocol: HubProto,
    pub label: String,
    /// Center on the diagram, once the user has dragged this node.
    #[serde(default)]
    pub at: Option<[f32; 2]>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Hub {
    pub id: u64,
    pub name: String,
    /// Set when the hub is one protocol. None means the hub is split across its links.
    pub protocol: Option<HubProto>,
    pub sources: Vec<HubPort>,
    pub outputs: Vec<HubPort>,
    /// Center on the diagram, once the user has dragged this hub.
    #[serde(default)]
    pub at: Option<[f32; 2]>,
}

impl Hub {
    pub fn split(&self) -> bool {
        self.protocol.is_none()
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

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HubModel {
    pub hubs: Vec<Hub>,
    /// Hub whose details are open. A port selection also sets this.
    pub selected: Option<u64>,
    /// Set when a source or output node is the one showing details.
    #[serde(default)]
    pub selected_port: Option<u64>,
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
        self.keep_focus();
    }

    /// Drop a port selection that no longer belongs to the open hub.
    fn keep_focus(&mut self) {
        let Some(hub_id) = self.selected else {
            self.selected_port = None;
            return;
        };
        let Some(port_id) = self.selected_port else {
            return;
        };
        let belongs = self.hub(hub_id).is_some_and(|hub| {
            hub.sources
                .iter()
                .chain(hub.outputs.iter())
                .any(|port| port.id == port_id)
        });
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
        }
    }

    fn port_mut(&mut self, hub_id: u64, port_id: u64) -> Option<&mut HubPort> {
        let hub = self.hub_mut(hub_id)?;
        hub.sources
            .iter_mut()
            .chain(hub.outputs.iter_mut())
            .find(|port| port.id == port_id)
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

    pub fn add_port(&mut self, hub_id: u64, source: bool) {
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
            return;
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
    }

    pub fn remove_port(&mut self, hub_id: u64, port_id: u64) {
        if let Some(hub) = self.hub_mut(hub_id) {
            hub.sources.retain(|port| port.id != port_id);
            hub.outputs.retain(|port| port.id != port_id);
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Hub,
    Source,
    Output,
}

struct Placed {
    hub_id: u64,
    port_id: Option<u64>,
    role: Role,
    rect: Rect,
}

/// Node boxes in content space. Width is the diagram's content width.
fn place_hubs(hubs: &[Hub], width: f32) -> (f32, Vec<Placed>) {
    let width = width.max(520.0);
    let port_w = ((width - HUB_W - 56.0) * 0.5).clamp(108.0, 168.0);
    let left = 16.0;
    let right = width - 16.0 - port_w;
    let mut y = 22.0;
    let mut nodes = Vec::new();
    for hub in hubs {
        let rows = hub.sources.len().max(hub.outputs.len()).max(1) as f32;
        let stack = rows * PORT_H + (rows - 1.0).max(0.0) * GAP;
        let band = stack.max(HUB_H) + 28.0;
        let mid = y + band * 0.5;
        nodes.push(Placed {
            hub_id: hub.id,
            port_id: None,
            role: Role::Hub,
            rect: Rect::from_center_size(pos2(width * 0.5, mid), vec2(HUB_W, HUB_H)),
        });
        stack_ports(
            &mut nodes,
            &hub.sources,
            Role::Source,
            left,
            port_w,
            mid,
            hub.id,
        );
        stack_ports(
            &mut nodes,
            &hub.outputs,
            Role::Output,
            right,
            port_w,
            mid,
            hub.id,
        );
        y += band + 10.0;
    }
    if hubs.is_empty() {
        y = 160.0;
    }
    (y + 12.0, nodes)
}

fn stack_ports(
    nodes: &mut Vec<Placed>,
    ports: &[HubPort],
    role: Role,
    x: f32,
    width: f32,
    mid: f32,
    hub_id: u64,
) {
    if ports.is_empty() {
        return;
    }
    let stack = ports.len() as f32 * PORT_H + (ports.len() - 1) as f32 * GAP;
    let mut y = mid - stack * 0.5;
    for port in ports {
        nodes.push(Placed {
            hub_id,
            port_id: Some(port.id),
            role,
            rect: Rect::from_min_size(pos2(x, y), vec2(width, PORT_H)),
        });
        y += PORT_H + GAP;
    }
}

/// Draw the diagram and the editor. True when the configuration changed.
pub fn paint(ui: &mut Ui, model: &mut HubModel, p: &ThemePalette) -> bool {
    let mut dirty = false;
    ui.horizontal(|ui| {
        if text_button(ui, "New hub", true, p).clicked() {
            model.add_hub();
            dirty = true;
        }
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(
                "Sources enter a hub. Outputs leave. A dashed line is that protocol continuing into another hub.",
            )
            .font(font_ui(12.0))
            .color(p.muted),
        );
    });
    ui.add_space(8.0);
    let wide = ui.available_width() > 760.0;
    if wide {
        let inspect_w = 276.0;
        let gap = 10.0;
        let diagram_w = (ui.available_width() - inspect_w - gap).max(280.0);
        let height = ui.available_height().max(160.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            dirty |= diagram_view(ui, model, diagram_w, height, p);
            dirty |= inspector(ui, model, inspect_w, height, p);
        });
    } else {
        let height = (ui.available_height() * 0.58).clamp(180.0, 420.0);
        dirty |= diagram_view(ui, model, ui.available_width(), height, p);
        ui.add_space(8.0);
        dirty |= inspector(
            ui,
            model,
            ui.available_width(),
            ui.available_height().max(120.0),
            p,
        );
    }
    dirty
}

fn diagram_view(
    ui: &mut Ui,
    model: &mut HubModel,
    width: f32,
    height: f32,
    p: &ThemePalette,
) -> bool {
    let (rect, _) = ui.allocate_exact_size(vec2(width.max(1.0), height.max(1.0)), Sense::hover());
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, Rounding::same(8.0), blend_scope(p.bg));
    paint_grid(&painter, rect);
    painter.rect_stroke(
        rect,
        Rounding::same(8.0),
        Stroke::new(1.0, tint(p.accent, 150)),
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink(6.0))
            .layout(Layout::top_down(egui::Align::Min)),
    );
    child.set_clip_rect(rect.shrink(1.0));
    let mut dirty = false;
    ScrollArea::both()
        .id_salt("pb_hub_diagram")
        .auto_shrink([false, false])
        .show(&mut child, |ui| {
            let content_w = ui.available_width().max(520.0);
            let (content_h, mut nodes) = place_hubs(&model.hubs, content_w);
            let (area, _) =
                ui.allocate_exact_size(vec2(content_w, content_h.max(48.0)), Sense::hover());
            // place_hubs is in content space. The painter is in screen space.
            let shift = area.min.to_vec2();
            for node in &mut nodes {
                node.rect = node.rect.translate(shift);
            }
            if model.hubs.is_empty() {
                ui.painter().text(
                    area.center(),
                    Align2::CENTER_CENTER,
                    "No hubs yet",
                    font_ui(14.0),
                    p.muted,
                );
                return;
            }
            paint_flow(ui.painter(), &nodes, model, p);
            dirty |= hit_nodes(ui, &nodes, model);
        });
    dirty
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

fn paint_flow(painter: &egui::Painter, nodes: &[Placed], model: &HubModel, p: &ThemePalette) {
    for node in nodes {
        if node.role != Role::Source {
            continue;
        }
        let Some(hub) = find(nodes, node.hub_id, Role::Hub, None) else {
            continue;
        };
        let proto = port_proto(model, node.hub_id, node.port_id);
        curve(
            painter,
            node.rect.right_center(),
            hub.rect.left_center(),
            proto.ink(p.is_light),
            false,
        );
    }
    for node in nodes {
        if node.role != Role::Output {
            continue;
        }
        let Some(hub) = find(nodes, node.hub_id, Role::Hub, None) else {
            continue;
        };
        let proto = port_proto(model, node.hub_id, node.port_id);
        curve(
            painter,
            hub.rect.right_center(),
            node.rect.left_center(),
            proto.ink(p.is_light),
            false,
        );
    }
    for (index, link) in model.across().iter().enumerate() {
        let Some(from) = find(nodes, link.from_hub, Role::Output, Some(link.from_port)) else {
            continue;
        };
        let Some(to) = find(nodes, link.to_hub, Role::Source, Some(link.to_port)) else {
            continue;
        };
        let ink = link.proto.ink(p.is_light);
        let shift = (index as f32) * 10.0;
        dashed_around(
            painter,
            from.rect.right_center(),
            to.rect.left_center(),
            shift,
            ink,
        );
        painter.text(
            pos2(
                (from.rect.right() + to.rect.left()) * 0.5,
                (from.rect.center().y + to.rect.center().y) * 0.5 - 8.0 - shift * 0.2,
            ),
            Align2::CENTER_CENTER,
            link.proto.label(),
            font_ui(10.0),
            ink,
        );
    }
    for node in nodes {
        match node.role {
            Role::Hub => paint_hub_node(painter, node, model, p),
            Role::Source | Role::Output => paint_port_node(painter, node, model, p),
        }
    }
}

fn hit_nodes(ui: &mut Ui, nodes: &[Placed], model: &mut HubModel) -> bool {
    let mut pick = None;
    for node in nodes {
        let id = Id::new((
            "pb_hub_hit",
            node.hub_id,
            node.port_id.unwrap_or(0),
            node.role as u8,
        ));
        let resp = ui.interact(node.rect, id, Sense::click());
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() {
            pick = Some(node.hub_id);
        }
    }
    if let Some(id) = pick {
        if model.selected != Some(id) {
            model.selected = Some(id);
            return true;
        }
    }
    false
}

fn paint_hub_node(painter: &egui::Painter, node: &Placed, model: &HubModel, p: &ThemePalette) {
    let Some(hub) = model.hub(node.hub_id) else {
        return;
    };
    let on = model.selected == Some(hub.id);
    let ink = hub
        .protocol
        .map(|proto| proto.ink(p.is_light))
        .unwrap_or(p.accent);
    painter.rect_filled(
        node.rect,
        Rounding::same(8.0),
        if on { p.header_top } else { p.pane },
    );
    painter.rect_stroke(
        node.rect,
        Rounding::same(8.0),
        Stroke::new(if on { 2.0 } else { 1.25 }, ink),
    );
    let clip = node.rect.shrink2(vec2(8.0, 4.0));
    painter.with_clip_rect(clip).text(
        pos2(clip.min.x, node.rect.center().y - 8.0),
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
        p.muted,
    );
}

fn paint_port_node(painter: &egui::Painter, node: &Placed, model: &HubModel, p: &ThemePalette) {
    let Some(port) = find_port(model, node.hub_id, node.port_id) else {
        return;
    };
    let ink = port.protocol.ink(p.is_light);
    painter.rect_filled(node.rect, Rounding::same(6.0), p.control);
    painter.rect_stroke(node.rect, Rounding::same(6.0), Stroke::new(1.0, p.line));
    let bar = Rect::from_min_max(node.rect.min, pos2(node.rect.min.x + 3.0, node.rect.max.y));
    painter.rect_filled(
        bar,
        Rounding {
            nw: 6.0,
            ne: 0.0,
            sw: 6.0,
            se: 0.0,
        },
        ink,
    );
    let clip = Rect::from_min_max(
        pos2(node.rect.min.x + 10.0, node.rect.min.y),
        pos2(node.rect.max.x - 6.0, node.rect.max.y),
    );
    painter.with_clip_rect(clip).text(
        pos2(clip.min.x, node.rect.center().y - 7.0),
        Align2::LEFT_CENTER,
        port.protocol.label(),
        font_ui(10.0),
        ink,
    );
    painter.with_clip_rect(clip).text(
        pos2(clip.min.x, node.rect.center().y + 7.0),
        Align2::LEFT_CENTER,
        &port.label,
        font_ui(12.0),
        p.text,
    );
}

fn inspector(ui: &mut Ui, model: &mut HubModel, width: f32, height: f32, p: &ThemePalette) -> bool {
    let (rect, _) = ui.allocate_exact_size(vec2(width.max(1.0), height.max(1.0)), Sense::hover());
    ui.painter()
        .rect_filled(rect, Rounding::same(8.0), p.control);
    ui.painter()
        .rect_stroke(rect, Rounding::same(8.0), Stroke::new(1.0, p.line));
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink(10.0))
            .layout(Layout::top_down(egui::Align::Min)),
    );
    child.set_clip_rect(rect.shrink(1.0));
    let mut dirty = false;
    ScrollArea::vertical()
        .id_salt("pb_hub_edit")
        .auto_shrink([false, false])
        .show(&mut child, |ui| {
            dirty |= edit_selected(ui, model, p);
        });
    dirty
}

fn edit_selected(ui: &mut Ui, model: &mut HubModel, p: &ThemePalette) -> bool {
    let Some(id) = model.selected else {
        ui.label(
            egui::RichText::new("Create a hub to choose what converts.")
                .font(font_ui(12.5))
                .color(p.muted),
        );
        return false;
    };
    if model.hub(id).is_none() {
        return false;
    }
    let mut dirty = false;
    ui.label(
        egui::RichText::new("HUB")
            .font(font_ui(10.5))
            .color(p.muted),
    );
    let name = &mut model.hub_mut(id).unwrap().name;
    if ui
        .add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY))
        .changed()
    {
        dirty = true;
    }
    ui.add_space(8.0);
    let split = model.hub(id).unwrap().split();
    ui.horizontal(|ui| {
        if text_button(ui, "Split", split, p).clicked() && !split {
            model.set_split(id, true);
            dirty = true;
        }
        if text_button(ui, "One protocol", !split, p).clicked() && split {
            model.set_split(id, false);
            dirty = true;
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
                    dirty = true;
                }
            }
        });
    }
    ui.add_space(10.0);
    dirty |= port_section(ui, model, id, true, p);
    ui.add_space(8.0);
    dirty |= port_section(ui, model, id, false, p);
    ui.add_space(12.0);
    if text_button(ui, "Delete hub", false, p).clicked() {
        model.remove_hub(id);
        dirty = true;
    }
    dirty
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
        ui.horizontal(|ui| {
            let label = &mut port_field(model, hub_id, port_id).label;
            if ui
                .add(egui::TextEdit::singleline(label).desired_width(120.0))
                .changed()
            {
                dirty = true;
            }
            if !locked {
                let current = port_field(model, hub_id, port_id).protocol;
                let mut next = current;
                egui::ComboBox::from_id_salt(("pb_hub_proto", port_id))
                    .selected_text(current.label())
                    .width(96.0)
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
            if text_button(ui, "×", false, p).clicked() {
                model.remove_port(hub_id, port_id);
                dirty = true;
            }
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

fn port_proto(model: &HubModel, hub_id: u64, port_id: Option<u64>) -> HubProto {
    find_port(model, hub_id, port_id)
        .map(|port| port.protocol)
        .unwrap_or(HubProto::Modbus)
}

fn curve(painter: &egui::Painter, from: Pos2, to: Pos2, color: Color32, dashed: bool) {
    let dx = ((to.x - from.x).abs() * 0.45).max(12.0);
    let c1 = pos2(from.x + dx, from.y);
    let c2 = pos2(to.x - dx, to.y);
    stroke_cubic(painter, from, c1, c2, to, Stroke::new(1.6, color), dashed);
    let dir = (to - c2).normalized();
    arrow(painter, to, dir, 7.0, color);
}

fn dashed_around(painter: &egui::Painter, from: Pos2, to: Pos2, shift: f32, color: Color32) {
    let lane = from.x.max(to.x) + 28.0 + shift;
    let c1 = pos2(lane, from.y);
    let c2 = pos2(lane, to.y);
    stroke_cubic(
        painter,
        from,
        c1,
        c2,
        to,
        Stroke::new(1.35, tint(color, 210)),
        true,
    );
    arrow(painter, to, Vec2::new(-1.0, 0.0), 7.0, color);
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
        let (_height, nodes) = place_hubs(&model.hubs, 800.0);
        let ports: usize = model
            .hubs
            .iter()
            .map(|hub| hub.sources.len() + hub.outputs.len())
            .sum();
        assert_eq!(nodes.len(), ports + model.hubs.len());
        assert!(nodes
            .iter()
            .any(|node| node.role == Role::Hub && node.hub_id == 1));
    }
}
