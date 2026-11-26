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
        HubProto::Wits | HubProto::WebSocket | HubProto::Sparkplug | HubProto::Dnp3 | HubProto::Can => {
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
                    egui::Grid::new("pb_mappings_grid")
                        .num_columns(4)
                        .spacing([14.0, 4.0])
                        .striped(true)
                        .show(ui, |ui| {
                            for h in ["POINT", "FROM", "TO", "HUB"] {
                                ui.label(RichText::new(h).font(font_ui(10.5)).color(p.muted));
                            }
                            ui.end_row();
                            for r in &rows {
                                ui.label(RichText::new(&r.point).font(font_ui(12.0)).color(p.text));
                                ui.label(RichText::new(&r.from).font(font_ui(12.0)).color(p.muted));
                                ui.label(RichText::new(&r.to).font(font_ui(12.0)).color(p.text));
                                ui.label(RichText::new(&r.hub).font(font_ui(12.0)).color(p.muted));
                                ui.end_row();
                            }
                        });
                });
        },
    );
    ui.allocate_rect(full, egui::Sense::hover());
}
