//! Pane bodies. Tables are empty on purpose: this build does not read or write a link.
//! MQTT lives in `mqtt_panel` (connections); hub Input/Output nodes own point binds.

use egui::{Align2, Color32, Pos2, Rect, Rounding, Sense, Stroke, Vec2};
use pb_engine::{EngineHandle, HostSnapshot};

use crate::art;
use crate::dock::DockPane;
use crate::hubs::HubModel;
use crate::mqtt_panel::{self, MqttPanelState};
use crate::point_rows::{self, PointsTableRow, PointsTableState};
use crate::theme::{font_semi, font_ui, scope, tint, ThemePalette};

/// Points panel filter + sort (session UI state).
#[derive(Debug, Clone)]
pub struct PointsPanelState {
    pub filter: String,
    pub table: PointsTableState,
}

impl Default for PointsPanelState {
    fn default() -> Self {
        Self {
            filter: String::new(),
            table: PointsTableState::default(),
        }
    }
}

/// Fill the pane body for one dock leaf.
pub fn paint(
    ui: &mut egui::Ui,
    pane: DockPane,
    host: &HostSnapshot,
    p: &ThemePalette,
    hubs: &mut HubModel,
    engine: &EngineHandle,
    mqtt: &mut MqttPanelState,
    points: &mut PointsPanelState,
) -> crate::hubs::HubChange {
    match pane {
        DockPane::Project => project_panel(ui, hubs, p),
        DockPane::Hubs => {
            // Sync trees so Input inspector can pick topics from live traffic.
            mqtt.sync_trees_from_host(host);
            let conns = mqtt.connection_choices();
            crate::hubs::paint(ui, hubs, p, &conns, mqtt, host)
        }
        DockPane::Mqtt => {
            mqtt_panel::paint(ui, host, p, engine, mqtt, hubs);
            crate::hubs::HubChange::quiet()
        }
        DockPane::Points => {
            points_panel(ui, host, p, points);
            crate::hubs::HubChange::quiet()
        }
        _ => {
            scaffold_panel(ui, pane, host, p);
            crate::hubs::HubChange::quiet()
        }
    }
}

fn points_panel(
    ui: &mut egui::Ui,
    host: &HostSnapshot,
    p: &ThemePalette,
    state: &mut PointsPanelState,
) {
    // Same inset as MQTT panel fields (theme item_spacing / button_padding rhythm).
    const SIDE: f32 = 8.0;
    const LABEL_W: f32 = 100.0;
    let full = ui.available_rect_before_wrap();
    let inner = Rect::from_min_max(
        Pos2::new(full.min.x + SIDE, full.min.y + 4.0),
        Pos2::new(full.max.x - SIDE, full.max.y),
    );
    ui.allocate_new_ui(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::top_down(egui::Align::Min)),
        |ui| points_panel_body(ui, host, p, state, LABEL_W),
    );
    ui.allocate_rect(full, Sense::hover());
}

fn points_panel_body(
    ui: &mut egui::Ui,
    host: &HostSnapshot,
    p: &ThemePalette,
    state: &mut PointsPanelState,
    label_w: f32,
) {
    ui.label(
        egui::RichText::new("Live values from wired inputs — every protocol lands here.")
            .font(font_ui(12.5))
            .color(p.muted),
    );
    ui.add_space(8.0);

    // Search filters name OR source — MQTT-style label width + field gutters.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        ui.add_sized(
            [label_w, 20.0],
            egui::Label::new(
                egui::RichText::new("Search")
                    .font(font_ui(12.0))
                    .color(p.muted),
            ),
        );
        let w = (ui.available_width() - 8.0).clamp(140.0, 360.0);
        ui.add(
            egui::TextEdit::singleline(&mut state.filter)
                .desired_width(w)
                .hint_text("name or source…"),
        );
    });
    ui.add_space(8.0);

    if host.points.is_empty() {
        ui.label(
            egui::RichText::new("No points yet. Connect an input and subscribe to populate.")
                .font(font_ui(12.5))
                .color(p.muted),
        );
        return;
    }

    let filter = state.filter.trim().to_ascii_lowercase();
    let mut rows: Vec<PointsTableRow> = host
        .points
        .iter()
        .filter(|pt| {
            if filter.is_empty() {
                return true;
            }
            pt.name.to_ascii_lowercase().contains(&filter)
                || pt.source.to_ascii_lowercase().contains(&filter)
        })
        .map(PointsTableRow::from_snapshot)
        .collect();

    if rows.is_empty() {
        ui.label(
            egui::RichText::new("No points match this search.")
                .font(font_ui(12.5))
                .color(p.muted),
        );
        return;
    }

    egui::ScrollArea::vertical()
        .id_salt("pb_points_live")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let _action = point_rows::paint_points_table(
                ui,
                &mut rows,
                &mut state.table,
                p,
                "points-panel",
                false,
            );
        });
}

fn scaffold_panel(ui: &mut egui::Ui, pane: DockPane, host: &HostSnapshot, p: &ThemePalette) {
    let tall = ui.available_height() > 380.0;
    let text = blurb(pane, host);
    let resp = ui.add(
        egui::Label::new(egui::RichText::new(&text).font(font_ui(12.5)).color(p.muted)).truncate(),
    );
    resp.on_hover_text(&text);
    ui.add_space(8.0);
    if tall {
        hero_well(ui, pane, p);
        ui.add_space(8.0);
    }
    table_block(ui, pane, p);
}

fn blurb(pane: DockPane, host: &HostSnapshot) -> String {
    if let Some(id) = pane.engine() {
        if let Some(slot) = host.slot(id) {
            return slot.note.clone();
        }
    }
    match pane {
        DockPane::Project => "Hubs, inputs, outputs, and edge transforms for this project.".into(),
        DockPane::Points => "Live values from every wired protocol input.".into(),
        DockPane::Mappings => "A point that arrives on one link can leave on another.".into(),
        DockPane::Traffic => "Bytes and frames from every link, when capture exists.".into(),
        _ => String::new(),
    }
}

fn hero_well(ui: &mut egui::Ui, pane: DockPane, p: &ThemePalette) {
    let width = ui.available_width().max(1.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 150.0), Sense::hover());
    let painter = ui.painter().with_clip_rect(rect);
    // Scope blue over the theme background (lighter on Frost/Daylight). Art stays on top.
    painter.rect_filled(
        rect,
        Rounding::same(8.0),
        crate::theme::blend_scope(p.bg, p.is_light),
    );
    let step = 18.0;
    let grid = Stroke::new(1.0, scope::grid(p.is_light));
    let mut x = rect.min.x;
    while x < rect.max.x {
        painter.line_segment(
            [Pos2::new(x, rect.min.y), Pos2::new(x, rect.max.y)],
            grid,
        );
        x += step;
    }
    let mut y = rect.min.y;
    while y < rect.max.y {
        painter.line_segment(
            [Pos2::new(rect.min.x, y), Pos2::new(rect.max.x, y)],
            grid,
        );
        y += step;
    }
    art::paint_hero(&painter, pane, rect.shrink(14.0));
    painter.line_segment(
        [
            Pos2::new(rect.min.x + 12.0, rect.min.y + 7.0),
            Pos2::new(rect.max.x - 12.0, rect.min.y + 7.0),
        ],
        Stroke::new(1.0, Color32::from_white_alpha(24)),
    );
    ui.painter().rect_stroke(
        rect,
        Rounding::same(8.0),
        Stroke::new(1.0, tint(p.accent, 150)),
    );
}

fn table_block(ui: &mut egui::Ui, pane: DockPane, p: &ThemePalette) {
    let width = ui.available_width().max(1.0);
    let height = ui.available_height().max(72.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, Rounding::same(6.0), p.control);
    painter.rect_stroke(rect, Rounding::same(6.0), Stroke::new(1.0, p.line));

    let header_h = 28.0;
    let header = Rect::from_min_size(rect.min, Vec2::new(rect.width(), header_h));
    painter.rect_filled(
        header,
        Rounding {
            nw: 6.0,
            ne: 6.0,
            sw: 0.0,
            se: 0.0,
        },
        p.header_top,
    );
    let cols = columns(pane);
    let usable = (header.width() - 20.0).max(1.0);
    let mut x = header.min.x + 10.0;
    for (name, weight) in cols {
        let col_w = usable * weight;
        let col = Rect::from_min_size(
            Pos2::new(x, header.min.y),
            Vec2::new((col_w - 6.0).max(1.0), header.height()),
        );
        painter.with_clip_rect(col).text(
            Pos2::new(x, header.center().y),
            Align2::LEFT_CENTER,
            *name,
            font_ui(10.5),
            p.muted,
        );
        x += col_w;
    }
    painter.line_segment(
        [
            Pos2::new(header.min.x, header.max.y),
            Pos2::new(header.max.x, header.max.y),
        ],
        Stroke::new(1.0, p.line),
    );

    let body = Rect::from_min_max(Pos2::new(rect.min.x, header.max.y), rect.max);
    let message = empty_message(pane);
    if body.height() > 72.0 {
        let icon = Rect::from_center_size(
            Pos2::new(body.center().x, body.center().y - 16.0),
            Vec2::splat(26.0),
        );
        art::paint_pane_icon(painter, icon, pane, pane.ink(p.is_light));
        painter.text(
            Pos2::new(body.center().x, body.center().y + 16.0),
            Align2::CENTER_CENTER,
            message,
            font_ui(13.0),
            p.muted,
        );
    } else {
        painter.text(
            body.center(),
            Align2::CENTER_CENTER,
            message,
            font_ui(12.5),
            p.muted,
        );
    }
}

fn columns(pane: DockPane) -> &'static [(&'static str, f32)] {
    match pane {
        DockPane::Points => &[
            ("NAME", 0.24),
            ("VALUE", 0.16),
            ("QUALITY", 0.18),
            ("SOURCE", 0.22),
            ("UPDATED", 0.20),
        ],
        DockPane::Mappings => &[("POINT", 0.26), ("FROM", 0.37), ("TO", 0.37)],
        DockPane::Traffic => &[
            ("TIME", 0.16),
            ("LINK", 0.22),
            ("DIR", 0.12),
            ("LEN", 0.12),
            ("SUMMARY", 0.38),
        ],
        DockPane::Streams => &[("NAME", 0.26), ("KIND", 0.22), ("ENDPOINT", 0.52)],
        DockPane::Modbus => &[
            ("NAME", 0.28),
            ("ROLE", 0.20),
            ("TRANSPORT", 0.32),
            ("UNIT", 0.20),
        ],
        DockPane::Mqtt => &[("TOPIC", 0.50), ("DIR", 0.22), ("QOS", 0.28)],
        DockPane::OpcUa => &[("NODE", 0.42), ("ACCESS", 0.24), ("POINT", 0.34)],
        DockPane::Files => &[("PATH", 0.48), ("PARSER", 0.24), ("OFFSET", 0.28)],
        DockPane::Project | DockPane::Hubs => &[("NAME", 1.0)],
    }
}

fn empty_message(pane: DockPane) -> &'static str {
    match pane {
        DockPane::Points => "No points yet",
        DockPane::Mappings => "No mappings yet",
        DockPane::Traffic => "No frames yet",
        DockPane::Streams => "No endpoints yet",
        DockPane::Modbus => "No sessions yet",
        DockPane::Mqtt => "No topics yet",
        DockPane::OpcUa => "No nodes yet",
        DockPane::Files => "No files yet",
        DockPane::Project | DockPane::Hubs => "",
    }
}

fn project_panel(
    ui: &mut egui::Ui,
    hubs: &HubModel,
    p: &ThemePalette,
) -> crate::hubs::HubChange {
    egui::ScrollArea::vertical()
        .id_salt("pb_project_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            title_row(ui, p);
            ui.add_space(10.0);
            stat_row(ui, hubs, p);
            ui.add_space(14.0);
            list_section(
                ui,
                "INPUTS",
                &port_names(hubs, true),
                DockPane::Streams.ink(p.is_light),
                p,
            );
            ui.add_space(10.0);
            list_section(
                ui,
                "OUTPUTS",
                &port_names(hubs, false),
                DockPane::Mqtt.ink(p.is_light),
                p,
            );
            ui.add_space(10.0);
            list_section(ui, "HUBS", &hub_names(hubs), DockPane::Hubs.ink(p.is_light), p);
            ui.add_space(10.0);
            list_section(ui, "AGGREGATORS", &step_names(hubs, true), p.accent, p);
            ui.add_space(10.0);
            list_section(ui, "CALCULATIONS", &step_names(hubs, false), p.accent, p);
        });
    crate::hubs::HubChange::quiet()
}

fn hub_names(hubs: &HubModel) -> Vec<String> {
    hubs.hubs
        .iter()
        .map(|hub| format!("{} ({})", hub.name, hub.protocol.label()))
        .collect()
}

fn port_names(hubs: &HubModel, inputs: bool) -> Vec<String> {
    let mut names = Vec::new();
    for hub in &hubs.hubs {
        let list = if inputs { &hub.sources } else { &hub.outputs };
        for port in list {
            names.push(format!(
                "{} ({}) · {}",
                port.label,
                port.protocol.label(),
                hub.name
            ));
        }
    }
    names
}

fn step_names(hubs: &HubModel, aggregator: bool) -> Vec<String> {
    let mut names = Vec::new();
    for hub in &hubs.hubs {
        let list = if aggregator {
            &hub.aggregators
        } else {
            &hub.calculations
        };
        for step in list {
            let edge = hub
                .sources
                .iter()
                .chain(hub.outputs.iter())
                .find(|port| port.id == step.edge_port)
                .map(|port| port.label.as_str())
                .unwrap_or("?");
            names.push(format!("{} · {} · {}", step.label, edge, hub.name));
        }
    }
    names
}

fn input_count(hubs: &HubModel) -> usize {
    hubs.hubs.iter().map(|hub| hub.sources.len()).sum()
}

fn output_count(hubs: &HubModel) -> usize {
    hubs.hubs.iter().map(|hub| hub.outputs.len()).sum()
}

fn aggregator_count(hubs: &HubModel) -> usize {
    hubs.hubs.iter().map(|hub| hub.aggregators.len()).sum()
}

fn calculation_count(hubs: &HubModel) -> usize {
    hubs.hubs.iter().map(|hub| hub.calculations.len()).sum()
}

fn title_row(ui: &mut egui::Ui, p: &ThemePalette) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::hover());
        art::paint_mark(ui.painter(), rect, p);
        ui.add_space(4.0);
        ui.vertical(|ui| {
            ui.label(
                egui::RichText::new("Untitled")
                    .font(font_semi(16.0))
                    .color(p.text),
            );
            ui.label(
                egui::RichText::new("Scaffold — nothing is stored yet")
                    .font(font_ui(11.5))
                    .color(p.muted),
            );
        });
    });
}

fn stat_row(ui: &mut egui::Ui, hubs: &HubModel, p: &ThemePalette) {
    let gap = 8.0;
    let width = ((ui.available_width() - gap * 2.0) / 3.0).max(48.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        stat_tile(
            ui,
            width,
            "INPUTS",
            &input_count(hubs).to_string(),
            DockPane::Streams.ink(p.is_light),
            p,
        );
        stat_tile(
            ui,
            width,
            "OUTPUTS",
            &output_count(hubs).to_string(),
            DockPane::Mqtt.ink(p.is_light),
            p,
        );
        stat_tile(
            ui,
            width,
            "HUBS",
            &hubs.hubs.len().to_string(),
            DockPane::Hubs.ink(p.is_light),
            p,
        );
    });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        stat_tile(
            ui,
            width,
            "AGGREGATORS",
            &aggregator_count(hubs).to_string(),
            p.accent,
            p,
        );
        stat_tile(
            ui,
            width,
            "CALCULATIONS",
            &calculation_count(hubs).to_string(),
            p.accent,
            p,
        );
    });
}

fn stat_tile(
    ui: &mut egui::Ui,
    width: f32,
    label: &str,
    value: &str,
    ink: Color32,
    p: &ThemePalette,
) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 54.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, Rounding::same(6.0), p.control);
    painter.rect_stroke(rect, Rounding::same(6.0), Stroke::new(1.0, p.line));
    painter.rect_filled(
        Rect::from_min_size(rect.min, Vec2::new(rect.width(), 2.0)),
        Rounding {
            nw: 6.0,
            ne: 6.0,
            sw: 0.0,
            se: 0.0,
        },
        ink,
    );
    painter.text(
        Pos2::new(rect.min.x + 10.0, rect.min.y + 22.0),
        Align2::LEFT_CENTER,
        value,
        font_semi(18.0),
        p.text,
    );
    painter.text(
        Pos2::new(rect.min.x + 10.0, rect.max.y - 12.0),
        Align2::LEFT_CENTER,
        label,
        font_ui(10.0),
        p.muted,
    );
}

fn list_section(ui: &mut egui::Ui, title: &str, names: &[String], ink: Color32, p: &ThemePalette) {
    // Expand/collapse so long hub / transform lists stay manageable.
    let id = ui.make_persistent_id(("pb_project_list", title));
    let mut state =
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true);
    let open = state.is_open();
    let header = format!("{title}  ·  {}", names.len());
    let (bar, resp) = ui.allocate_exact_size(
        Vec2::new(ui.available_width().max(1.0), 22.0),
        Sense::click(),
    );
    if resp.hovered() {
        ui.painter()
            .rect_filled(bar, Rounding::same(4.0), tint(p.accent, 16));
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    // Chevron: ▶ collapsed, ▼ open.
    let chevron = if open { "▼" } else { "▶" };
    ui.painter().text(
        Pos2::new(bar.min.x + 4.0, bar.center().y),
        Align2::LEFT_CENTER,
        chevron,
        font_ui(9.0),
        p.muted,
    );
    ui.painter().text(
        Pos2::new(bar.min.x + 18.0, bar.center().y),
        Align2::LEFT_CENTER,
        header,
        font_ui(10.5),
        p.muted,
    );
    if resp.clicked() {
        state.toggle(ui);
    }
    state.store(ui.ctx());
    if !state.is_open() {
        return;
    }
    ui.add_space(2.0);
    if names.is_empty() {
        ui.label(
            egui::RichText::new("None yet")
                .font(font_ui(12.0))
                .color(p.muted),
        );
        return;
    }
    for name in names {
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 26.0), Sense::hover());
        ui.painter()
            .circle_filled(Pos2::new(rect.min.x + 10.0, rect.center().y), 3.5, ink);
        let clip = Rect::from_min_max(
            Pos2::new(rect.min.x + 22.0, rect.min.y),
            Pos2::new(rect.max.x - 6.0, rect.max.y),
        );
        ui.painter().with_clip_rect(clip).text(
            Pos2::new(clip.min.x, rect.center().y),
            Align2::LEFT_CENTER,
            name,
            font_ui(12.5),
            p.text,
        );
    }
}

pub fn format_uptime(ms: u64) -> String {
    let total = ms / 1000;
    format!("{}:{:02}", total / 60, total % 60)
}
