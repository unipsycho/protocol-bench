//! MQTT panel: multi-client list, per-client broker config, Inputs / Outputs.
//!
//! Inputs create/bind host points (subscribe + JSON parse paths).
//! Outputs bind host points → publish topics on this client.
//! Cross-protocol point routing stays in the Mappings panel.

use egui::{Color32, Sense, Stroke, Vec2};
use pb_engine::{EngineCommand, EngineHandle, EngineId, HostSnapshot};

use crate::theme::{font_semi, font_ui, ThemePalette};

const SIDE_BY_SIDE_MIN: f32 = 720.0;
const TRANSPORTS: &[&str] = &["tcp", "tls", "ws", "wss"];
const LABEL_W: f32 = 100.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MqttParseRule {
    pub topic: String,
    pub path: String,
    pub point: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MqttOutput {
    pub point: String,
    pub topic: String,
    pub qos: u8,
    pub retain: bool,
}

#[derive(Debug, Clone)]
pub struct MqttClient {
    pub id: String,
    pub label: String,
    pub host: String,
    pub port: String,
    pub client_id: String,
    pub username: String,
    pub password: String,
    pub transport: String,
    pub subscriptions: Vec<String>,
    pub parse_rules: Vec<MqttParseRule>,
    pub outputs: Vec<MqttOutput>,
    pub auto_connect: bool,
    // Draft fields for Inputs / Outputs editors.
    pub draft_sub: String,
    pub draft_path_topic: String,
    pub draft_path: String,
    pub draft_path_point: String,
    pub draft_out_point: String,
    pub draft_out_topic: String,
}

impl MqttClient {
    pub fn default_local() -> Self {
        Self {
            id: "local".into(),
            label: "Local broker".into(),
            host: "127.0.0.1".into(),
            port: "1883".into(),
            client_id: "protocol-bench".into(),
            username: String::new(),
            password: String::new(),
            transport: "tcp".into(),
            subscriptions: vec!["pb/#".into()],
            parse_rules: Vec::new(),
            outputs: Vec::new(),
            auto_connect: false,
            draft_sub: String::new(),
            draft_path_topic: String::new(),
            draft_path: String::new(),
            draft_path_point: String::new(),
            draft_out_point: String::new(),
            draft_out_topic: String::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct MqttPanelState {
    pub clients: Vec<MqttClient>,
    pub selected: usize,
}

impl Default for MqttPanelState {
    fn default() -> Self {
        Self {
            clients: vec![MqttClient::default_local()],
            selected: 0,
        }
    }
}

impl MqttPanelState {
    pub fn selected(&self) -> Option<&MqttClient> {
        self.clients.get(self.selected)
    }

    pub fn selected_mut(&mut self) -> Option<&mut MqttClient> {
        self.clients.get_mut(self.selected)
    }

    fn ensure_selection(&mut self) {
        if self.clients.is_empty() {
            self.clients.push(MqttClient::default_local());
            self.selected = 0;
        } else if self.selected >= self.clients.len() {
            self.selected = self.clients.len() - 1;
        }
    }

    fn unique_id(&self, base: &str) -> String {
        let mut id = base.to_string();
        let mut n = 2;
        while self.clients.iter().any(|c| c.id == id) {
            id = format!("{base}-{n}");
            n += 1;
        }
        id
    }
}

/// Push full multi-client document to the MQTT pack (reconcile + optional connect).
pub fn apply_mqtt_session(engine: &EngineHandle, state: &MqttPanelState) {
    let clients: Vec<serde_json::Value> = state
        .clients
        .iter()
        .map(|c| {
            let port: u16 = c.port.parse().unwrap_or(1883);
            serde_json::json!({
                "id": c.id,
                "label": c.label,
                "host": c.host,
                "port": port,
                "client_id": c.client_id,
                "username": c.username,
                "password": c.password,
                "transport": c.transport,
                "keep_alive_secs": 30,
                "subscriptions": c.subscriptions,
                "parse_rules": c.parse_rules.iter().map(|r| serde_json::json!({
                    "topic": r.topic,
                    "path": r.path,
                    "point": r.point,
                })).collect::<Vec<_>>(),
                "outputs": c.outputs.iter().map(|o| serde_json::json!({
                    "point": o.point,
                    "topic": o.topic,
                    "qos": o.qos,
                    "retain": o.retain,
                })).collect::<Vec<_>>(),
                "auto_connect": c.auto_connect,
            })
        })
        .collect();
    let json = serde_json::json!({ "clients": clients }).to_string();
    engine.submit(EngineId::Mqtt, EngineCommand::Configure { json });
}

pub fn paint(
    ui: &mut egui::Ui,
    host: &HostSnapshot,
    p: &ThemePalette,
    engine: &EngineHandle,
    state: &mut MqttPanelState,
) {
    state.ensure_selection();
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        let width = (ui.available_width() - 8.0).max(1.0);
        ui.allocate_ui_with_layout(
            Vec2::new(width, ui.available_height()),
            egui::Layout::top_down(egui::Align::Min),
            |ui| paint_body(ui, host, p, engine, state),
        );
        ui.add_space(8.0);
    });
}

fn paint_body(
    ui: &mut egui::Ui,
    host: &HostSnapshot,
    p: &ThemePalette,
    engine: &EngineHandle,
    state: &mut MqttPanelState,
) {
    let slot = host.slot(EngineId::Mqtt);
    let pack_loaded = slot.map(|s| s.pack_loaded).unwrap_or(false);
    ui.label(
        egui::RichText::new(if pack_loaded {
            "MQTT clients are sources/outputs only — broker hub role is not active. Cross-protocol routing uses the Mappings panel later."
        } else {
            "MQTT pack not loaded. Build pb-mqtt into modules/."
        })
        .font(font_ui(12.5))
        .color(p.muted),
    );
    ui.add_space(8.0);

    clients_bar(ui, state, engine, p);
    ui.add_space(10.0);

    let side_by_side = ui.available_width() >= SIDE_BY_SIDE_MIN;
    if side_by_side {
        let total = ui.available_width();
        let gap = 16.0;
        let left_w = (total * 0.55).clamp(340.0, total - 260.0);
        let right_w = (total - left_w - gap).max(220.0);
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(left_w, ui.available_height()),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("pb_mqtt_cfg")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            selected_client_editor(ui, host, state, engine, p);
                        });
                },
            );
            ui.add_space(gap);
            ui.allocate_ui_with_layout(
                Vec2::new(right_w, ui.available_height()),
                egui::Layout::top_down(egui::Align::Min),
                |ui| live_values(ui, host, state, p, true),
            );
        });
    } else {
        egui::ScrollArea::vertical()
            .id_salt("pb_mqtt_stack")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                live_values(ui, host, state, p, false);
                ui.add_space(12.0);
                selected_client_editor(ui, host, state, engine, p);
            });
    }
}

fn clients_bar(
    ui: &mut egui::Ui,
    state: &mut MqttPanelState,
    engine: &EngineHandle,
    p: &ThemePalette,
) {
    section(ui, "CONNECTIONS", p);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
        let mut select = None;
        let mut remove = None;
        for (i, client) in state.clients.iter().enumerate() {
            let selected = i == state.selected;
            let label = if client.label.is_empty() {
                client.id.as_str()
            } else {
                client.label.as_str()
            };
            if text_btn(ui, label, selected, p).clicked() {
                select = Some(i);
            }
            if state.clients.len() > 1
                && text_btn(ui, "×", false, p).on_hover_text("Remove connection").clicked()
            {
                remove = Some(i);
            }
        }
        if let Some(i) = select {
            state.selected = i;
        }
        if let Some(i) = remove {
            state.clients.remove(i);
            state.ensure_selection();
            apply_mqtt_session(engine, state);
        }
        ui.add_space(8.0);
        if text_btn(ui, "+ Add", true, p).clicked() {
            let id = state.unique_id("broker");
            let mut c = MqttClient::default_local();
            c.id = id.clone();
            c.label = id;
            c.client_id = format!("pb-{}", c.id);
            state.clients.push(c);
            state.selected = state.clients.len() - 1;
            apply_mqtt_session(engine, state);
        }
    });
}

fn selected_client_editor(
    ui: &mut egui::Ui,
    host: &HostSnapshot,
    state: &mut MqttPanelState,
    engine: &EngineHandle,
    p: &ThemePalette,
) {
    let Some(_) = state.selected() else {
        return;
    };
    section(ui, "BROKER / CLIENT", p);
    {
        let c = state.selected_mut().unwrap();
        field(ui, "Id", &mut c.id, p);
        field(ui, "Label", &mut c.label, p);
        field(ui, "Host", &mut c.host, p);
        field(ui, "Port", &mut c.port, p);
        field(ui, "Client ID", &mut c.client_id, p);
        field(ui, "Username", &mut c.username, p);
        password(ui, "Password", &mut c.password, p);
        transport(ui, &mut c.transport, p);
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if text_btn(ui, "Apply / Connect", true, p).clicked() {
            if let Some(c) = state.selected_mut() {
                c.auto_connect = true;
            }
            apply_mqtt_session(engine, state);
        }
        ui.add_space(12.0);
        if text_btn(ui, "Disconnect", false, p).clicked() {
            if let Some(c) = state.selected_mut() {
                c.auto_connect = false;
            }
            // Reconcile: this client stops; others with auto_connect stay up.
            apply_mqtt_session(engine, state);
        }
    });
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new("Password stored in the project JSON (no keyring yet).")
            .font(font_ui(11.0))
            .color(p.muted),
    );

    ui.add_space(14.0);
    inputs_section(ui, state, engine, p);
    ui.add_space(14.0);
    outputs_section(ui, host, state, engine, p);
}

fn inputs_section(
    ui: &mut egui::Ui,
    state: &mut MqttPanelState,
    engine: &EngineHandle,
    p: &ThemePalette,
) {
    section(ui, "INPUTS — subscribe → points", p);
    ui.label(
        egui::RichText::new(
            "Subscriptions create host points `mqtt/<client_name>/<topic>`. JSON paths create field points `mqtt/<client_name>/<topic>::<path>`.",
        )
        .font(font_ui(12.0))
        .color(p.muted),
    );
    ui.add_space(6.0);

    let mut remove_sub = None;
    let mut add_sub = false;
    let mut remove_rule = None;
    let mut add_rule = false;
    {
        let c = state.selected_mut().unwrap();
        for (i, topic) in c.subscriptions.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(topic)
                        .font(font_ui(12.0))
                        .color(p.text),
                );
                ui.add_space(8.0);
                if text_btn(ui, "Remove", false, p).clicked() {
                    remove_sub = Some(i);
                }
            });
        }
        field(ui, "Subscribe", &mut c.draft_sub, p);
        if text_btn(ui, "Add subscription", false, p).clicked() {
            add_sub = true;
        }
        ui.add_space(10.0);
        ui.label(
            egui::RichText::new("Parse paths")
                .font(font_ui(12.0))
                .color(p.muted),
        );
        for (i, rule) in c.parse_rules.iter().enumerate() {
            ui.horizontal(|ui| {
                let label = if rule.point.is_empty() {
                    format!("{} · {}", rule.topic, rule.path)
                } else {
                    format!("{} · {} → {}", rule.topic, rule.path, rule.point)
                };
                ui.label(
                    egui::RichText::new(label)
                        .font(font_ui(12.0))
                        .color(p.text),
                );
                if text_btn(ui, "Remove", false, p).clicked() {
                    remove_rule = Some(i);
                }
            });
        }
        field(ui, "Topic filter", &mut c.draft_path_topic, p);
        field(ui, "JSON path", &mut c.draft_path, p);
        field(ui, "Point id", &mut c.draft_path_point, p);
        ui.label(
            egui::RichText::new("Point id optional — default mqtt/<client_name>/<topic>::<path>")
                .font(font_ui(11.0))
                .color(p.muted),
        );
        if text_btn(ui, "Add parse path", false, p).clicked() {
            add_rule = true;
        }
    }
    let mut dirty = false;
    if let Some(i) = remove_sub {
        state.selected_mut().unwrap().subscriptions.remove(i);
        dirty = true;
    }
    if add_sub {
        let c = state.selected_mut().unwrap();
        let topic = c.draft_sub.trim().to_string();
        if !topic.is_empty() && !c.subscriptions.iter().any(|t| t == &topic) {
            c.subscriptions.push(topic);
            c.draft_sub.clear();
            dirty = true;
        }
    }
    if let Some(i) = remove_rule {
        state.selected_mut().unwrap().parse_rules.remove(i);
        dirty = true;
    }
    if add_rule {
        let c = state.selected_mut().unwrap();
        let path = c.draft_path.trim().to_string();
        if !path.is_empty() {
            let topic = if c.draft_path_topic.trim().is_empty() {
                c.subscriptions.first().cloned().unwrap_or_default()
            } else {
                c.draft_path_topic.trim().to_string()
            };
            let rule = MqttParseRule {
                topic,
                path,
                point: c.draft_path_point.trim().to_string(),
            };
            if !c.parse_rules.iter().any(|r| r == &rule) {
                c.parse_rules.push(rule);
            }
            c.draft_path.clear();
            c.draft_path_point.clear();
            dirty = true;
        }
    }
    if dirty {
        apply_mqtt_session(engine, state);
    }
}

fn outputs_section(
    ui: &mut egui::Ui,
    host: &HostSnapshot,
    state: &mut MqttPanelState,
    engine: &EngineHandle,
    p: &ThemePalette,
) {
    section(ui, "OUTPUTS — points → publish", p);
    ui.label(
        egui::RichText::new(
            "Bind host points to publish topics on this client. Cross-protocol links stay in Mappings.",
        )
        .font(font_ui(12.0))
        .color(p.muted),
    );
    ui.add_space(6.0);

    let mut remove = None;
    let mut add = false;
    let mut publish_point: Option<String> = None;
    {
        let c = state.selected_mut().unwrap();
        for (i, out) in c.outputs.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("{} → {}", out.point, out.topic))
                        .font(font_ui(12.0))
                        .color(p.text),
                );
                ui.add_space(8.0);
                if text_btn(ui, "Publish", false, p).clicked() {
                    publish_point = Some(out.point.clone());
                }
                if text_btn(ui, "Remove", false, p).clicked() {
                    remove = Some(i);
                }
            });
        }
        field(ui, "Point", &mut c.draft_out_point, p);
        field(ui, "Topic", &mut c.draft_out_topic, p);
        if text_btn(ui, "Add output", false, p).clicked() {
            add = true;
        }
    }
    let mut dirty = false;
    if let Some(i) = remove {
        state.selected_mut().unwrap().outputs.remove(i);
        dirty = true;
    }
    if add {
        let c = state.selected_mut().unwrap();
        let point = c.draft_out_point.trim().to_string();
        let topic = c.draft_out_topic.trim().to_string();
        if !point.is_empty() && !topic.is_empty() {
            let out = MqttOutput {
                point,
                topic,
                qos: 0,
                retain: false,
            };
            if !c.outputs.iter().any(|o| o == &out) {
                c.outputs.push(out);
            }
            c.draft_out_point.clear();
            c.draft_out_topic.clear();
            dirty = true;
        }
    }
    if dirty {
        apply_mqtt_session(engine, state);
    }
    if let Some(point) = publish_point {
        let value = host
            .points
            .iter()
            .find(|pt| pt.name == point)
            .map(|pt| pt.value.clone())
            .unwrap_or_default();
        engine.submit(
            EngineId::Mqtt,
            EngineCommand::WritePoint { name: point, value },
        );
    }
}

fn live_values(
    ui: &mut egui::Ui,
    host: &HostSnapshot,
    state: &MqttPanelState,
    p: &ThemePalette,
    side: bool,
) {
    section(ui, "LIVE VALUES", p);
    let client_id = state.selected().map(|c| c.id.as_str()).unwrap_or("");
    let prefix = format!("mqtt/{client_id}/");
    let points: Vec<_> = host
        .points_for_source("mqtt")
        .filter(|pt| client_id.is_empty() || pt.name.starts_with(&prefix))
        .collect();
    if points.is_empty() {
        ui.label(
            egui::RichText::new("No points for this client yet — add an Input subscription.")
                .font(font_ui(12.0))
                .color(p.muted),
        );
        return;
    }
    let height = if side {
        ui.available_height().max(120.0)
    } else {
        ((points.len() as f32) * 22.0 + 28.0).clamp(72.0, 220.0)
    };
    egui::ScrollArea::vertical()
        .id_salt("pb_mqtt_live")
        .max_height(height)
        .auto_shrink([false, !side])
        .show(ui, |ui| {
            egui::Grid::new("pb_mqtt_live_grid")
                .num_columns(3)
                .spacing([10.0, 4.0])
                .striped(true)
                .show(ui, |ui| {
                    for h in ["POINT", "VALUE", "QUALITY"] {
                        ui.label(
                            egui::RichText::new(h)
                                .font(font_ui(10.5))
                                .color(p.muted),
                        );
                    }
                    ui.end_row();
                    for pt in points {
                        let name = pt
                            .name
                            .strip_prefix(&prefix)
                            .unwrap_or(pt.name.as_str());
                        ui.label(
                            egui::RichText::new(name)
                                .font(font_ui(12.0))
                                .color(p.text),
                        );
                        ui.label(
                            egui::RichText::new(truncate(&pt.value, 48))
                                .font(font_ui(12.0))
                                .color(p.text),
                        );
                        ui.label(
                            egui::RichText::new(&pt.quality)
                                .font(font_ui(12.0))
                                .color(p.muted),
                        );
                        ui.end_row();
                    }
                });
        });
}

fn section(ui: &mut egui::Ui, label: &str, p: &ThemePalette) {
    ui.label(
        egui::RichText::new(label)
            .font(font_ui(10.5))
            .color(p.muted),
    );
    ui.add_space(6.0);
}

fn field(ui: &mut egui::Ui, label: &str, value: &mut String, p: &ThemePalette) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        ui.add_sized(
            [LABEL_W, 20.0],
            egui::Label::new(
                egui::RichText::new(label)
                    .font(font_ui(12.0))
                    .color(p.muted),
            ),
        );
        let w = (ui.available_width() - 8.0).clamp(140.0, 360.0);
        ui.add(egui::TextEdit::singleline(value).desired_width(w));
    });
    ui.add_space(4.0);
}

fn password(ui: &mut egui::Ui, label: &str, value: &mut String, p: &ThemePalette) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        ui.add_sized(
            [LABEL_W, 20.0],
            egui::Label::new(
                egui::RichText::new(label)
                    .font(font_ui(12.0))
                    .color(p.muted),
            ),
        );
        let w = (ui.available_width() - 8.0).clamp(140.0, 360.0);
        ui.add(
            egui::TextEdit::singleline(value)
                .password(true)
                .desired_width(w),
        );
    });
    ui.add_space(4.0);
}

fn transport(ui: &mut egui::Ui, value: &mut String, p: &ThemePalette) {
    if !TRANSPORTS.iter().any(|t| *t == value.as_str()) {
        *value = "tcp".into();
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        ui.add_sized(
            [LABEL_W, 20.0],
            egui::Label::new(
                egui::RichText::new("Transport")
                    .font(font_ui(12.0))
                    .color(p.muted),
            ),
        );
        let w = (ui.available_width() - 8.0).clamp(140.0, 220.0);
        egui::ComboBox::from_id_salt("pb_mqtt_transport")
            .selected_text(value.as_str())
            .width(w)
            .show_ui(ui, |ui| {
                for opt in TRANSPORTS {
                    ui.selectable_value(value, (*opt).to_string(), *opt);
                }
            });
    });
    ui.add_space(4.0);
}

fn text_btn(ui: &mut egui::Ui, label: &str, accent: bool, p: &ThemePalette) -> egui::Response {
    let fill = if accent { p.accent } else { p.control };
    let text = if accent {
        Color32::from_rgb(0x12, 0x14, 0x18)
    } else {
        p.text
    };
    ui.add(
        egui::Button::new(egui::RichText::new(label).font(font_ui(12.5)).color(text))
            .fill(fill)
            .stroke(Stroke::new(1.0, p.line))
            .min_size(Vec2::new(72.0, 26.0)),
    )
}

fn truncate(value: &str, max: usize) -> String {
    let flat = value.replace('\n', " ");
    if flat.chars().count() <= max {
        flat
    } else {
        let mut out: String = flat.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

#[allow(dead_code)]
fn _sense() {
    let _ = Sense::click();
    let _ = font_semi(1.0);
}
