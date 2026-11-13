//! Pane bodies. Tables are empty on purpose: this build does not read or write a link.

use egui::{Align2, Color32, Pos2, Rect, Rounding, Sense, Stroke, Vec2};
use pb_engine::HostSnapshot;

use crate::art;
use crate::dock::DockPane;
use crate::hubs::HubModel;
use crate::theme::{font_semi, font_ui, scope, tint, ThemePalette};

/// Fill the pane body for one dock leaf.
pub fn paint(
    ui: &mut egui::Ui,
    pane: DockPane,
    host: &HostSnapshot,
    p: &ThemePalette,
    hubs: &mut HubModel,
) -> crate::hubs::HubChange {
    match pane {
        DockPane::Project => {
            project_panel(ui, hubs, p);
            crate::hubs::HubChange::quiet()
        }
        DockPane::Hubs => crate::hubs::paint(ui, hubs, p),
        _ => {
            scaffold_panel(ui, pane, host, p);
            crate::hubs::HubChange::quiet()
        }
    }
}

fn scaffold_panel(ui: &mut egui::Ui, pane: DockPane, host: &HostSnapshot, p: &ThemePalette) {
    let tall = ui.available_height() > 380.0;
    let text = blurb(pane, host);
    let resp = ui.add(
        egui::Label::new(egui::RichText::new(text).font(font_ui(12.5)).color(p.muted)).truncate(),
    );
    resp.on_hover_text(text);
    ui.add_space(8.0);
    if tall {
        hero_well(ui, pane, p);
        ui.add_space(8.0);
    }
    table_block(ui, pane, p);
}

fn blurb(pane: DockPane, host: &HostSnapshot) -> &'static str {
    if let Some(id) = pane.engine() {
        if let Some(slot) = host.slot(id) {
            return slot.note;
        }
    }
    match pane {
        DockPane::Project => "Hubs, endpoints, points, mappings, and edge transforms for this project.",
        DockPane::Points => "One value, quality, and timestamp for every named point.",
        DockPane::Mappings => "A point that arrives on one link can leave on another.",
        DockPane::Traffic => "Bytes and frames from every link, when capture exists.",
        _ => "",
    }
}

fn hero_well(ui: &mut egui::Ui, pane: DockPane, p: &ThemePalette) {
    let width = ui.available_width().max(1.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 150.0), Sense::hover());
    let painter = ui.painter().with_clip_rect(rect);
    // Scope blue at half strength over the theme background. The drawing on top stays.
    painter.rect_filled(rect, Rounding::same(8.0), crate::theme::blend_scope(p.bg));
    let step = 18.0;
    let mut x = rect.min.x;
    while x < rect.max.x {
        painter.line_segment(
            [Pos2::new(x, rect.min.y), Pos2::new(x, rect.max.y)],
            Stroke::new(1.0, scope::grid()),
        );
        x += step;
    }
    let mut y = rect.min.y;
    while y < rect.max.y {
        painter.line_segment(
            [Pos2::new(rect.min.x, y), Pos2::new(rect.max.x, y)],
            Stroke::new(1.0, scope::grid()),
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

fn project_panel(ui: &mut egui::Ui, hubs: &HubModel, p: &ThemePalette) {
    egui::ScrollArea::vertical()
        .id_salt("pb_project_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            title_row(ui, p);
            ui.add_space(10.0);
            stat_row(ui, hubs, p);
            ui.add_space(14.0);
            list_section(ui, "HUBS", &hub_names(hubs), DockPane::Hubs.ink(p.is_light), p);
            ui.add_space(10.0);
            list_section(
                ui,
                "AGGREGATORS",
                &step_names(hubs, true),
                p.accent,
                p,
            );
            ui.add_space(10.0);
            list_section(
                ui,
                "CALCULATIONS",
                &step_names(hubs, false),
                p.accent,
                p,
            );
        });
}

fn hub_names(hubs: &HubModel) -> Vec<String> {
    hubs.hubs
        .iter()
        .map(|hub| {
            let proto = hub
                .protocol
                .map(|p| p.label())
                .unwrap_or("Split");
            format!("{} ({})", hub.name, proto)
        })
        .collect()
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

fn endpoint_count(hubs: &HubModel) -> usize {
    hubs.hubs
        .iter()
        .map(|hub| hub.sources.len() + hub.outputs.len())
        .sum()
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
            "ENDPOINTS",
            &endpoint_count(hubs).to_string(),
            DockPane::Streams.ink(p.is_light),
            p,
        );
        // Points / mappings stay zero until live stores land.
        stat_tile(
            ui,
            width,
            "POINTS",
            "0",
            DockPane::Points.ink(p.is_light),
            p,
        );
        stat_tile(
            ui,
            width,
            "MAPPINGS",
            "0",
            DockPane::Mappings.ink(p.is_light),
            p,
        );
    });
    ui.add_space(8.0);
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
    ui.label(
        egui::RichText::new(title)
            .font(font_ui(10.5))
            .color(p.muted),
    );
    ui.add_space(4.0);
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
