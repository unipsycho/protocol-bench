//! Lean Traffic panel — listeners, connected clients, point-update throughput.
//!
//! Not a frame/trace pane. Stats only (PB-107 lean plan).

use std::collections::BTreeMap;
use std::time::Instant;

use egui::{Align2, Layout, RichText, Sense};
use pb_engine::{EngineId, HostSnapshot};

use crate::mqtt_panel::MqttPanelState;
use crate::opcua_panel::OpcUaPanelState;
use crate::panels;
use crate::theme::{apply_form_spacing, font_ui, form_inset, space, ThemePalette};

/// Session UI state for throughput sampling.
#[derive(Debug, Clone)]
pub struct TrafficPanelState {
    last_sample: Instant,
    prev_updated: BTreeMap<String, u64>,
    updates_per_sec: f32,
}

impl Default for TrafficPanelState {
    fn default() -> Self {
        Self {
            last_sample: Instant::now(),
            prev_updated: BTreeMap::new(),
            updates_per_sec: 0.0,
        }
    }
}

impl TrafficPanelState {
    /// Refresh updates/s from point `updated_ms` deltas (≈0.5–2 s window).
    fn sample_point_rate(&mut self, host: &HostSnapshot) {
        let elapsed = self.last_sample.elapsed().as_secs_f32();
        if elapsed < 0.45 {
            return;
        }
        let mut changes = 0u32;
        let mut next = BTreeMap::new();
        for pt in &host.points {
            next.insert(pt.name.clone(), pt.updated_ms);
            match self.prev_updated.get(&pt.name) {
                Some(&prev) if prev != pt.updated_ms => changes += 1,
                None => {
                    // First sight of a point counts as activity once sampling is warm.
                    if !self.prev_updated.is_empty() {
                        changes += 1;
                    }
                }
                _ => {}
            }
        }
        if !self.prev_updated.is_empty() || !host.points.is_empty() {
            self.updates_per_sec = changes as f32 / elapsed.max(0.001);
        }
        self.prev_updated = next;
        self.last_sample = Instant::now();
    }
}

pub fn paint(
    ui: &mut egui::Ui,
    host: &HostSnapshot,
    mqtt: &mut MqttPanelState,
    opcua: &mut OpcUaPanelState,
    state: &mut TrafficPanelState,
    p: &ThemePalette,
) {
    // Keep OPC UA server flag fresh without opening that panel.
    opcua.sync_from_host(host);
    state.sample_point_rate(host);

    let full = ui.available_rect_before_wrap();
    let inner = form_inset(full);
    ui.allocate_new_ui(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::top_down(egui::Align::Min)),
        |ui| paint_body(ui, host, mqtt, opcua, state, p),
    );
    ui.allocate_rect(full, Sense::hover());
}

fn paint_body(
    ui: &mut egui::Ui,
    host: &HostSnapshot,
    mqtt: &MqttPanelState,
    opcua: &OpcUaPanelState,
    state: &TrafficPanelState,
    p: &ThemePalette,
) {
    apply_form_spacing(ui);
    ui.label(
        RichText::new("Listeners, connected clients, and point throughput — not a message trace.")
            .font(font_ui(12.5))
            .color(p.muted),
    );
    ui.add_space(space::SECTION);

    section(ui, "LISTENERS", p);
    paint_listeners(ui, host, mqtt, opcua, p);
    ui.add_space(space::SECTION);

    section(ui, "CONNECTED CLIENTS", p);
    paint_connected(ui, host, mqtt, opcua, p);
    ui.add_space(space::SECTION);

    section(ui, "ACTIVITY", p);
    paint_activity(ui, host, state, p);
}

fn section(ui: &mut egui::Ui, label: &str, p: &ThemePalette) {
    ui.label(
        RichText::new(label)
            .font(font_ui(10.5))
            .color(p.muted),
    );
    ui.add_space(space::AFTER_HEADING);
}

fn paint_listeners(
    ui: &mut egui::Ui,
    host: &HostSnapshot,
    mqtt: &MqttPanelState,
    opcua: &OpcUaPanelState,
    p: &ThemePalette,
) {
    let mut any = false;
    if mqtt.embedded.enabled {
        any = true;
        let running = mqtt_hub_running(host, mqtt);
        let addr = format!("{}:{}", mqtt.embedded.bind.trim(), mqtt.embedded.port);
        let name = mqtt.embedded.display_name().to_string();
        let err = mqtt_hub_error(host);
        status_row(
            ui,
            &format!("MQTT · {name}"),
            &addr,
            if running { "running" } else { "stopped" },
            running,
            err.as_deref(),
            p,
        );
    }
    if opcua.embedded.enabled {
        any = true;
        let running = opcua.server_running;
        let addr = format!("{}:{}", opcua.embedded.bind.trim(), opcua.embedded.port);
        let name = if opcua.embedded.name.trim().is_empty() {
            "Hub OPC UA".into()
        } else {
            opcua.embedded.name.clone()
        };
        let err = if !running && !opcua.server_status.is_empty() && opcua.server_status != "Stopped"
        {
            Some(opcua.server_status.as_str())
        } else {
            None
        };
        status_row(
            ui,
            &format!("OPC UA · {name}"),
            &addr,
            if running { "running" } else { "stopped" },
            running,
            err,
            p,
        );
    }
    // Modbus server connections a hub is serving (running or retrying).
    let (live, pack_loaded) = crate::modbus_panel::live_from_host(host);
    if pack_loaded {
        for l in live.iter().filter(|l| l.role == "server") {
            any = true;
            let err = (!l.error.is_empty()).then_some(l.error.as_str());
            status_row(
                ui,
                &format!("Modbus · {}", l.id),
                &l.endpoint,
                if l.connected { "running" } else { "stopped" },
                l.connected,
                err,
                p,
            );
        }
    }
    if !any {
        ui.label(
            RichText::new("No embedded hubs configured.")
                .font(font_ui(12.5))
                .color(p.muted),
        );
    }
}

fn paint_connected(
    ui: &mut egui::Ui,
    host: &HostSnapshot,
    mqtt: &MqttPanelState,
    opcua: &OpcUaPanelState,
    p: &ThemePalette,
) {
    let hub_id = mqtt.embedded.connection_id().to_string();
    let mut rows = Vec::new();

    // MQTT outbound clients that are up (skip the embedded hub id).
    if let Some(slot) = host.slot(EngineId::Mqtt) {
        if let Ok(arr) = serde_json::from_str::<serde_json::Value>(&slot.detail_json) {
            if let Some(clients) = arr.as_array() {
                for c in clients {
                    let id = c.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    if id.is_empty() || id == hub_id {
                        continue;
                    }
                    let connected = c
                        .get("connected")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    if !connected {
                        continue;
                    }
                    let label = c
                        .get("label")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .unwrap_or(id);
                    let host_s = c.get("host").and_then(|v| v.as_str()).unwrap_or("");
                    let port = c.get("port").and_then(|v| v.as_u64()).unwrap_or(0);
                    let endpoint = if host_s.is_empty() {
                        id.to_string()
                    } else if port > 0 {
                        format!("{host_s}:{port}")
                    } else {
                        host_s.to_string()
                    };
                    rows.push((format!("MQTT · {label}"), endpoint));
                }
            }
        }
    }

    // OPC UA clients that are up.
    if let Some(slot) = host.slot(EngineId::OpcUa) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&slot.detail_json) {
            if let Some(clients) = value.get("clients").and_then(|v| v.as_array()) {
                for c in clients {
                    let connected = c
                        .get("connected")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    if !connected {
                        continue;
                    }
                    let id = c.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    let label = c
                        .get("label")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .unwrap_or(id);
                    let endpoint = c
                        .get("endpoint_url")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    // Prefer panel label when snapshot is sparse.
                    let title = if label.is_empty() {
                        opcua
                            .clients
                            .iter()
                            .find(|pc| pc.id == id)
                            .map(|pc| {
                                if pc.label.trim().is_empty() {
                                    pc.id.clone()
                                } else {
                                    pc.label.clone()
                                }
                            })
                            .unwrap_or_else(|| id.to_string())
                    } else {
                        label.to_string()
                    };
                    let ep = if endpoint.is_empty() {
                        opcua
                            .clients
                            .iter()
                            .find(|pc| pc.id == id)
                            .map(|pc| pc.endpoint_url.clone())
                            .unwrap_or_default()
                    } else {
                        endpoint
                    };
                    rows.push((format!("OPC UA · {title}"), ep));
                }
            }
        }
    }

    // Modbus: outbound client links that are up, and masters connected to our servers.
    let (live, pack_loaded) = crate::modbus_panel::live_from_host(host);
    if pack_loaded {
        for l in &live {
            if l.role == "server" {
                if let Some(n) = l.clients.filter(|n| *n > 0) {
                    rows.push((
                        format!("Modbus · {}", l.id),
                        format!("{n} master{} on {}", if n == 1 { "" } else { "s" }, l.endpoint),
                    ));
                }
            } else if l.connected {
                rows.push((format!("Modbus · {}", l.id), l.endpoint.clone()));
            }
        }
    }

    if rows.is_empty() {
        ui.label(
            RichText::new("No connected clients.")
                .font(font_ui(12.5))
                .color(p.muted),
        );
        return;
    }
    for (title, endpoint) in rows {
        status_row(ui, &title, &endpoint, "connected", true, None, p);
    }
}

fn paint_activity(
    ui: &mut egui::Ui,
    host: &HostSnapshot,
    state: &TrafficPanelState,
    p: &ThemePalette,
) {
    let rate = if state.updates_per_sec < 0.05 {
        "0 point updates/s".into()
    } else if state.updates_per_sec < 10.0 {
        format!("~{:.1} point updates/s", state.updates_per_sec)
    } else {
        format!("~{:.0} point updates/s", state.updates_per_sec)
    };
    let up = panels::format_uptime(host.uptime_ms);
    let alive = if host.alive { "service up" } else { "host stopped" };
    ui.label(
        RichText::new(format!("{rate}  ·  up {up}  ·  {alive}"))
            .font(font_ui(12.5))
            .color(p.text),
    );
    ui.add_space(space::AFTER_BLURB);

    let bad = host
        .points
        .iter()
        .filter(|pt| {
            let q = pt.quality.to_ascii_lowercase();
            q.contains("bad") || q.contains("uncertain") || q == "error"
        })
        .count();
    let errors: Vec<_> = host
        .slots
        .iter()
        .filter(|s| !s.last_error.is_empty())
        .map(|s| format!("{}: {}", s.title, s.last_error))
        .collect();

    if errors.is_empty() && bad == 0 {
        ui.label(
            RichText::new(format!(
                "{} points · no slot errors",
                host.point_count
            ))
            .font(font_ui(12.0))
            .color(p.muted),
        );
    } else {
        if bad > 0 {
            ui.label(
                RichText::new(format!("{bad} point(s) with bad/uncertain quality"))
                    .font(font_ui(12.0))
                    .color(p.warn),
            );
        }
        for err in errors {
            ui.label(
                RichText::new(err)
                    .font(font_ui(12.0))
                    .color(p.warn),
            );
        }
    }
}

fn status_row(
    ui: &mut egui::Ui,
    title: &str,
    detail: &str,
    state: &str,
    ok: bool,
    err: Option<&str>,
    p: &ThemePalette,
) {
    // Title · address · state on one line when they fit, else the address drops below
    // (in a narrow pane the right-aligned state used to paint over the address).
    let width = |t: &str, size: f32| ui.fonts(|f| f.layout_no_wrap(t.to_string(), font_ui(size), p.text).size().x);
    let one_line = width(title, 12.5) + width(detail, 12.0) + width(state, 12.0) + space::ITEM_X * 3.0
        <= ui.available_width();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::ITEM_X;
        ui.label(
            RichText::new(title)
                .font(font_ui(12.5))
                .color(p.text),
        );
        if one_line {
            ui.label(
                RichText::new(detail)
                    .font(font_ui(12.0))
                    .color(p.muted),
            );
        }
        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                RichText::new(state)
                    .font(font_ui(12.0))
                    .color(if ok { p.live } else { p.muted }),
            );
        });
    });
    if !one_line && !detail.is_empty() {
        ui.label(RichText::new(detail).font(font_ui(12.0)).color(p.muted));
    }
    if let Some(e) = err {
        if !e.is_empty() {
            ui.label(
                RichText::new(e)
                    .font(font_ui(11.5))
                    .color(p.warn),
            );
        }
    }
    ui.add_space(2.0);
}

fn mqtt_hub_running(host: &HostSnapshot, mqtt: &MqttPanelState) -> bool {
    if mqtt.broker_running {
        return true;
    }
    host.slot(EngineId::Mqtt)
        .map(|s| s.state == "broker" || s.state.starts_with("broker"))
        .unwrap_or(false)
}

fn mqtt_hub_error(host: &HostSnapshot) -> Option<String> {
    let slot = host.slot(EngineId::Mqtt)?;
    let err = slot.last_error.trim();
    if err.is_empty() {
        return None;
    }
    if err.to_ascii_lowercase().contains("broker") {
        Some(err.to_string())
    } else {
        None
    }
}

#[allow(dead_code)]
fn _align_bits() {
    let _ = Align2::LEFT_CENTER;
}
