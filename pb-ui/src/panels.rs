//! Pane bodies. Tables are empty on purpose: this build does not read or write a link.
//! MQTT lives in `mqtt_panel` (connections); hub Input/Output nodes own point binds.

use egui::{Align2, Color32, Pos2, Rect, Rounding, Sense, Stroke, Vec2};
use crate::engine_api::EngineHandle;
use pb_engine::HostSnapshot;

use crate::art;
use crate::dock::DockPane;
use crate::hubs::HubModel;
use crate::http_panel::{self, HttpPanelState};
use crate::mqtt_panel::{self, MqttPanelState};
use crate::opcua_panel::{self, OpcUaPanelState};
use crate::point_rows::{self, PointsTableRow, PointsTableState};
use crate::project::ProjectSettings;
use crate::streams_panel::{self, StreamsPanelState};
use crate::theme::{
    after_field, apply_form_spacing, font_semi, font_ui, form_inset, space, scope, tint,
    ThemePalette,
};
use crate::traffic_panel::{self, TrafficPanelState};

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
    opcua: &mut OpcUaPanelState,
    http: &mut HttpPanelState,
    streams: &mut StreamsPanelState,
    points: &mut PointsPanelState,
    traffic: &mut TrafficPanelState,
    settings: &mut ProjectSettings,
    project_path: &std::path::Path,
) -> crate::hubs::HubChange {
    match pane {
        DockPane::Project => project_panel(ui, hubs, mqtt, opcua, host, settings, project_path, p),
        DockPane::Hubs => {
            // Sync trees so Input inspector can pick topics from live traffic.
            mqtt.sync_trees_from_host(host);
            opcua.sync_from_host(host);
            let conns = mqtt.connection_choices();
            crate::hubs::paint(ui, hubs, p, &conns, mqtt, opcua, http, streams, host, engine)
        }
        DockPane::Mqtt => {
            mqtt_panel::paint(ui, host, p, engine, mqtt, hubs);
            crate::hubs::HubChange::quiet()
        }
        DockPane::OpcUa => {
            opcua_panel::paint(ui, host, p, engine, opcua, hubs);
            crate::hubs::HubChange::quiet()
        }
        DockPane::Http => {
            let dirty = http_panel::paint(ui, p, http);
            let mut change = crate::hubs::HubChange::quiet();
            change.dirty = dirty;
            change
        }
        DockPane::Streams => {
            let dirty = streams_panel::paint(ui, p, streams);
            let mut change = crate::hubs::HubChange::quiet();
            change.dirty = dirty;
            change
        }
        DockPane::Points => {
            points_panel(ui, host, hubs, p, points);
            crate::hubs::HubChange::quiet()
        }
        DockPane::Traffic => {
            traffic_panel::paint(ui, host, mqtt, opcua, traffic, p);
            crate::hubs::HubChange::quiet()
        }
        DockPane::Calculations => {
            // Legacy leaf — sanitized out of the tree; keep match exhaustive.
            scaffold_panel(ui, pane, host, p);
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
    hubs: &HubModel,
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
        |ui| points_panel_body(ui, host, hubs, p, state, LABEL_W),
    );
    ui.allocate_rect(full, Sense::hover());
}

fn points_panel_body(
    ui: &mut egui::Ui,
    host: &HostSnapshot,
    hubs: &HubModel,
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

    // Bus snapshot + Hub Input bind placeholders (honest empty only when no binds).
    let mut rows = point_rows::rows_for_points_panel(hubs, &host.points);
    if rows.is_empty() {
        ui.label(
            egui::RichText::new(
                "No points yet. Bind a topic on a Hub Input to create one.",
            )
            .font(font_ui(12.5))
            .color(p.muted),
        );
        return;
    }

    let filter = state.filter.trim().to_ascii_lowercase();
    if !filter.is_empty() {
        rows.retain(|row| {
            row.id.to_ascii_lowercase().contains(&filter)
                || row.source.to_ascii_lowercase().contains(&filter)
        });
    }

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
        DockPane::Calculations => "Moved into calculation node properties on the Hubs diagram.".into(),
        DockPane::Mappings => "A point that arrives on one link can leave on another.".into(),
        DockPane::Traffic => {
            "Listeners, connected clients, and point throughput.".into()
        }
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
        DockPane::Http => &[("NAME", 0.28), ("URL", 0.52), ("AUTH", 0.20)],
        DockPane::Project | DockPane::Hubs | DockPane::Calculations => &[("NAME", 1.0)],
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
        DockPane::Http => "No clients yet",
        DockPane::Project | DockPane::Hubs | DockPane::Calculations => "",
    }
}

fn project_panel(
    ui: &mut egui::Ui,
    hubs: &HubModel,
    mqtt: &MqttPanelState,
    opcua: &OpcUaPanelState,
    host: &HostSnapshot,
    settings: &mut ProjectSettings,
    project_path: &std::path::Path,
    p: &ThemePalette,
) -> crate::hubs::HubChange {
    let mut change = crate::hubs::HubChange::quiet();
    let full = ui.available_rect_before_wrap();
    let inner = form_inset(full);
    ui.allocate_new_ui(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::top_down(egui::Align::Min)),
        |ui| {
            apply_form_spacing(ui);
            egui::ScrollArea::vertical()
                .id_salt("pb_project_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    apply_form_spacing(ui);
                    // Identity — name + description (PB-58). File → Save As covers JSON export.
                    ui.label(
                        egui::RichText::new("IDENTITY")
                            .font(font_ui(10.5))
                            .color(p.muted),
                    );
                    ui.add_space(space::AFTER_HEADING);
                    let fallback = project_path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("Project");
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = space::ITEM_X;
                        let (mark, _) =
                            ui.allocate_exact_size(Vec2::splat(32.0), Sense::hover());
                        art::paint_mark(ui.painter(), mark, p);
                        ui.vertical(|ui| {
                            if ui
                                .add(
                                    egui::TextEdit::singleline(&mut settings.name)
                                        .desired_width(ui.available_width().max(120.0))
                                        .hint_text(fallback)
                                        .margin(egui::Margin::symmetric(6.0, 3.0)),
                                )
                                .changed()
                            {
                                change.dirty = true;
                            }
                            ui.label(
                                egui::RichText::new(project_path.display().to_string())
                                    .font(font_ui(11.0))
                                    .color(p.muted),
                            );
                        });
                    });
                    after_field(ui);
                    ui.label(
                        egui::RichText::new("Description")
                            .font(font_ui(12.0))
                            .color(p.muted),
                    );
                    ui.add_space(2.0);
                    if ui
                        .add(
                            egui::TextEdit::multiline(&mut settings.description)
                                .desired_width(ui.available_width().max(120.0))
                                .desired_rows(3)
                                .hint_text("Optional notes for this project")
                                .margin(egui::Margin::symmetric(6.0, 4.0)),
                        )
                        .changed()
                    {
                        change.dirty = true;
                    }
                    ui.add_space(space::SECTION);

                    // Summary counts + which embedded hubs are running.
                    ui.label(
                        egui::RichText::new("SUMMARY")
                            .font(font_ui(10.5))
                            .color(p.muted),
                    );
                    ui.add_space(space::AFTER_HEADING);
                    summary_row(ui, hubs, host.point_count, p);
                    ui.add_space(space::AFTER_FIELD);
                    running_hubs_line(ui, mqtt, opcua, p);
                    ui.add_space(space::AFTER_BLURB);
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("Open Hub diagram")
                                    .font(font_ui(12.5))
                                    .color(p.text),
                            )
                            .fill(p.control)
                            .stroke(Stroke::new(1.0, p.line)),
                        )
                        .clicked()
                    {
                        change.open = Some(DockPane::Hubs);
                    }
                    ui.add_space(space::SECTION);

                    // Collapsed inventory — accordion per type (default closed).
                    ui.label(
                        egui::RichText::new("INVENTORY")
                            .font(font_ui(10.5))
                            .color(p.muted),
                    );
                    ui.add_space(space::AFTER_HEADING);
                    // Compact accordion stack (PB-94): override form ITEM_Y between sections.
                    let prev_y = ui.spacing().item_spacing.y;
                    ui.spacing_mut().item_spacing.y = 2.0;
                    list_section(
                        ui,
                        "Hubs",
                        &hub_names(hubs),
                        DockPane::Hubs.ink(p.is_light),
                        p,
                    );
                    list_section(
                        ui,
                        "Inputs",
                        &port_names(hubs, true),
                        DockPane::Streams.ink(p.is_light),
                        p,
                    );
                    list_section(
                        ui,
                        "Outputs",
                        &port_names(hubs, false),
                        DockPane::Mqtt.ink(p.is_light),
                        p,
                    );
                    list_section(
                        ui,
                        "Calcs / Aggs",
                        &step_names_combined(hubs),
                        p.accent,
                        p,
                    );
                    ui.spacing_mut().item_spacing.y = prev_y;
                });
        },
    );
    ui.allocate_rect(full, Sense::hover());
    change
}

fn running_hubs_line(
    ui: &mut egui::Ui,
    mqtt: &MqttPanelState,
    opcua: &OpcUaPanelState,
    p: &ThemePalette,
) {
    let mut parts = Vec::new();
    if mqtt.embedded.enabled {
        parts.push(if mqtt.broker_running {
            format!("MQTT hub · running (:{})", mqtt.embedded.port)
        } else {
            "MQTT hub · stopped".into()
        });
    }
    if opcua.embedded.enabled {
        parts.push(if opcua.server_running {
            format!("OPC UA hub · running (:{})", opcua.embedded.port)
        } else {
            "OPC UA hub · stopped".into()
        });
    }
    let text = if parts.is_empty() {
        "No embedded hubs configured.".into()
    } else {
        parts.join("  ·  ")
    };
    ui.label(
        egui::RichText::new(text)
            .font(font_ui(12.0))
            .color(if mqtt.broker_running || opcua.server_running {
                p.live
            } else {
                p.muted
            }),
    );
}

fn summary_row(ui: &mut egui::Ui, hubs: &HubModel, points: usize, p: &ThemePalette) {
    let gap = space::CHIP;
    let cols = 3.0;
    let width = ((ui.available_width() - gap * (cols - 1.0)) / cols).max(48.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        stat_tile(
            ui,
            width,
            "HUBS",
            &hubs.hubs.len().to_string(),
            DockPane::Hubs.ink(p.is_light),
            p,
        );
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
    });
    ui.add_space(space::AFTER_FIELD);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        stat_tile(
            ui,
            width,
            "CALCS",
            &calculation_count(hubs).to_string(),
            p.accent,
            p,
        );
        stat_tile(
            ui,
            width,
            "AGGS",
            &aggregator_count(hubs).to_string(),
            p.accent,
            p,
        );
        stat_tile(
            ui,
            width,
            "POINTS",
            &points.to_string(),
            DockPane::Points.ink(p.is_light),
            p,
        );
    });
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

fn step_names_combined(hubs: &HubModel) -> Vec<String> {
    let mut names = Vec::new();
    for hub in &hubs.hubs {
        for step in &hub.calculations {
            let edge = hub
                .sources
                .iter()
                .chain(hub.outputs.iter())
                .find(|port| port.id == step.edge_port)
                .map(|port| port.label.as_str())
                .unwrap_or(if step.edge_port == 0 { "free" } else { "?" });
            names.push(format!("Calc · {} · {} · {}", step.label, edge, hub.name));
        }
        for step in &hub.aggregators {
            let edge = hub
                .sources
                .iter()
                .chain(hub.outputs.iter())
                .find(|port| port.id == step.edge_port)
                .map(|port| port.label.as_str())
                .unwrap_or(if step.edge_port == 0 { "free" } else { "?" });
            names.push(format!("Agg · {} · {} · {}", step.label, edge, hub.name));
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
    // Accordion inventory — default collapsed so the diagram stays the source of truth (PB-58).
    // Compact header + rows (PB-94).
    let id = ui.make_persistent_id(("pb_project_list", title));
    let mut state =
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false);
    let open = state.is_open();
    let header = format!("{title}  ·  {}", names.len());
    let (bar, resp) = ui.allocate_exact_size(
        Vec2::new(ui.available_width().max(1.0), 18.0),
        Sense::click(),
    );
    if resp.hovered() {
        ui.painter()
            .rect_filled(bar, Rounding::same(3.0), tint(p.accent, 16));
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    // Chevron: ▶ collapsed, ▼ open.
    let chevron = if open { "▼" } else { "▶" };
    ui.painter().text(
        Pos2::new(bar.min.x + 4.0, bar.center().y),
        Align2::LEFT_CENTER,
        chevron,
        font_ui(8.5),
        p.muted,
    );
    ui.painter().text(
        Pos2::new(bar.min.x + 16.0, bar.center().y),
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
    if names.is_empty() {
        ui.label(
            egui::RichText::new("None yet")
                .font(font_ui(11.5))
                .color(p.muted),
        );
        return;
    }
    let row_y = ui.spacing().item_spacing.y;
    ui.spacing_mut().item_spacing.y = 1.0;
    for name in names {
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 18.0), Sense::hover());
        ui.painter()
            .circle_filled(Pos2::new(rect.min.x + 10.0, rect.center().y), 2.75, ink);
        let clip = Rect::from_min_max(
            Pos2::new(rect.min.x + 20.0, rect.min.y),
            Pos2::new(rect.max.x - 6.0, rect.max.y),
        );
        ui.painter().with_clip_rect(clip).text(
            Pos2::new(clip.min.x, rect.center().y),
            Align2::LEFT_CENTER,
            name,
            font_ui(12.0),
            p.text,
        );
    }
    ui.spacing_mut().item_spacing.y = row_y;
}

pub fn format_uptime(ms: u64) -> String {
    let total = ms / 1000;
    format!("{}:{:02}", total / 60, total % 60)
}
