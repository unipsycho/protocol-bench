//! Mappings panel — read-only view of every point route in the project.
//!
//! Mappings are type-preserving copies (PRD settled #7). They are *authored* on hub Output nodes
//! (and the edges feeding them); this panel lists the result so a technician can see what leaves
//! where without opening each hub.

use egui::{RichText, ScrollArea};
use pb_engine::HostSnapshot;

use crate::hubs::{HubModel, HubPort, HubProto};
use crate::theme::{font_ui, form_inset, space, ThemePalette};

/// One route: host point → output address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappingRow {
    pub point: String,
    pub from: String,
    pub to: String,
    pub hub: String,
}

fn output_routes(port: &HubPort) -> Vec<(String, String)> {
    let mut out = Vec::new();
    match port.protocol {
        HubProto::Mqtt => {
            for b in &port.mqtt_outputs {
                let conn = if port.mqtt_connection.is_empty() { "mqtt" } else { port.mqtt_connection.as_str() };
                out.push((b.point.clone(), format!("mqtt/{conn}/{}", b.topic)));
            }
        }
        HubProto::Modbus => {
            for b in &port.modbus_outputs {
                let conn = if port.modbus_connection.is_empty() { "modbus" } else { port.modbus_connection.as_str() };
                out.push((b.point.clone(), format!("modbus/{conn}::{}", b.describe())));
            }
        }
        HubProto::Streams => {
            for b in &port.stream_outputs {
                let ep = if port.stream_endpoint.is_empty() { "stream" } else { port.stream_endpoint.as_str() };
                out.push((b.point.clone(), format!("stream/{ep}")));
            }
        }
        HubProto::Http => {
            for b in &port.http_outputs {
                out.push((b.point.clone(), format!("http/{}::{}", port.http_connection, b.json_path)));
            }
        }
        HubProto::Las => {
            for b in &port.las_outputs {
                out.push((b.point.clone(), format!("las/{}::{}", port.las_endpoint, b.curve)));
            }
        }
        HubProto::Wits | HubProto::WebSocket | HubProto::Sparkplug | HubProto::Dnp3 | HubProto::Can | HubProto::Gen => {
            if let Some(e) = port.protocol.ext() {
                let conn = if port.ext_connection.is_empty() { e.id() } else { port.ext_connection.as_str() };
                for b in &port.ext_outputs {
                    let point = crate::ext_proto::text_of(b, "point");
                    out.push((point, format!("{}/{conn}::{}", e.id(), crate::ext_proto::bind_label(e, b))));
                }
            }
        }
        HubProto::OpcUa | HubProto::Files | HubProto::Snmp => {}
    }
    out
}

pub fn rows(hubs: &HubModel, host: &HostSnapshot) -> Vec<MappingRow> {
    let mut rows = Vec::new();
    for hub in &hubs.hubs {
        if hub.protocol == HubProto::OpcUa {
            for e in &hub.opcua_catalog {
                if e.point.trim().is_empty() || e.browse_name.trim().is_empty() {
                    continue;
                }
                let from = host
                    .points
                    .iter()
                    .find(|p| p.name == e.point)
                    .map(|p| p.source.clone())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "—".into());
                rows.push(MappingRow {
                    point: e.point.clone(),
                    from,
                    to: format!("opcua/hub/{}", e.full_path()),
                    hub: hub.name.clone(),
                });
            }
        }
        for port in &hub.outputs {
            for (point, to) in output_routes(port) {
                if point.trim().is_empty() {
                    continue;
                }
                let from = host
                    .points
                    .iter()
                    .find(|p| p.name == point)
                    .map(|p| p.source.clone())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "—".into());
                rows.push(MappingRow { point, from, to, hub: hub.name.clone() });
            }
        }
    }
    rows
}

const HEADERS: [&str; 4] = ["POINT", "FROM", "TO", "HUB"];
const GRID_GAP: f32 = 14.0;

/// Which of POINT / FROM / TO / HUB fit in `avail`. Point and To always show (To is what
/// tells two rows for the same point apart); From, then Hub, are added while they fit.
/// Hub is left out when every row belongs to the same hub.
fn plan_columns(widths: [f32; 4], avail: f32, gap: f32, single_hub: bool) -> [bool; 4] {
    let mut show = [true, false, true, false];
    let mut used = widths[0] + gap + widths[2];
    for i in [1, 3] {
        if i == 3 && single_hub {
            continue;
        }
        if used + gap + widths[i] <= avail {
            show[i] = true;
            used += gap + widths[i];
        }
    }
    show
}

pub fn paint(ui: &mut egui::Ui, host: &HostSnapshot, hubs: &HubModel, p: &ThemePalette) {
    let full = ui.available_rect_before_wrap();
    let inner = form_inset(full);
    ui.allocate_new_ui(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::top_down(egui::Align::Min)),
        |ui| {
            ui.label(
                RichText::new(
                    "Every point that leaves on another link (type-preserving copy). \
                     Edit routes on hub Output nodes.",
                )
                .font(font_ui(12.5))
                .color(p.muted),
            );
            ui.add_space(space::AFTER_BLURB);
            let rows = rows(hubs, host);
            if rows.is_empty() {
                ui.label(
                    RichText::new("No mappings yet. Add binds on a hub Output node.")
                        .font(font_ui(12.5))
                        .color(p.muted),
                );
                return;
            }
            ScrollArea::vertical()
                .id_salt("pb_mappings")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let measure = |t: &str| ui.fonts(|f| f.layout_no_wrap(t.to_string(), font_ui(12.0), p.text).size().x);
                    let mut widths = [0.0f32; 4];
                    for (i, h) in HEADERS.iter().enumerate() {
                        widths[i] = measure(h);
                    }
                    for r in &rows {
                        for (i, t) in [&r.point, &r.from, &r.to, &r.hub].into_iter().enumerate() {
                            widths[i] = widths[i].max(measure(t));
                        }
                    }
                    let single_hub = rows.iter().all(|r| r.hub == rows[0].hub);
                    let show = plan_columns(widths, ui.available_width(), GRID_GAP, single_hub);
                    let cols: Vec<usize> = (0..4).filter(|i| show[*i]).collect();
                    egui::Grid::new("pb_mappings_grid")
                        .num_columns(cols.len())
                        .spacing([GRID_GAP, 4.0])
                        .striped(true)
                        .show(ui, |ui| {
                            for &i in &cols {
                                ui.label(RichText::new(HEADERS[i]).font(font_ui(10.5)).color(p.muted));
                            }
                            ui.end_row();
                            for r in &rows {
                                let cells = [&r.point, &r.from, &r.to, &r.hub];
                                for &i in &cols {
                                    let color = if i == 1 || i == 3 { p.muted } else { p.text };
                                    let resp = ui.label(RichText::new(cells[i]).font(font_ui(12.0)).color(color));
                                    if i == 0 && cols.len() < 4 {
                                        resp.on_hover_text(format!("{}  →  {}\nhub: {}", r.from, r.to, r.hub));
                                    }
                                }
                                ui.end_row();
                            }
                        });
                });
        },
    );
    ui.allocate_rect(full, egui::Sense::hover());
}

#[cfg(test)]
mod tests {
    use super::plan_columns;

    const W: [f32; 4] = [120.0, 200.0, 220.0, 90.0];

    #[test]
    fn wide_pane_shows_all_four() {
        assert_eq!(plan_columns(W, 1000.0, 14.0, false), [true, true, true, true]);
    }

    #[test]
    fn single_hub_drops_the_hub_column() {
        assert_eq!(plan_columns(W, 1000.0, 14.0, true), [true, true, true, false]);
    }

    #[test]
    fn narrow_pane_keeps_to_and_drops_from() {
        // Room for point + to only: the destination tells duplicate-looking rows apart.
        assert_eq!(plan_columns(W, 360.0, 14.0, false), [true, false, true, false]);
        // Room for hub but not from: hub is added.
        assert_eq!(plan_columns(W, 460.0, 14.0, false), [true, false, true, true]);
    }
}
