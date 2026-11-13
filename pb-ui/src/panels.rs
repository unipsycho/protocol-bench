//! Pane bodies. Tables are empty on purpose: this build does not read or write a link.
//! MQTT is the exception: a thin connect form drives the loaded client pack.

use egui::{Align2, Color32, Pos2, Rect, Rounding, Sense, Stroke, Vec2};
use pb_engine::{EngineCommand, EngineHandle, EngineId, HostSnapshot};

use crate::art;
use crate::dock::DockPane;
use crate::hubs::HubModel;
use crate::theme::{font_semi, font_ui, scope, tint, ThemePalette};

/// One JSON field path → point rule (edited on the MQTT panel).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MqttParseRule {
    pub topic: String,
    pub path: String,
    pub point: String,
}

/// Connect fields for one MQTT client (UI-only; pack gets JSON on Connect / Apply paths).
#[derive(Debug, Clone)]
pub struct MqttForm {
    pub host: String,
    pub port: String,
    pub client_id: String,
    pub username: String,
    pub password: String,
    /// tcp | tls | ws | wss
    pub transport: String,
    pub subscribe_topic: String,
    pub publish_topic: String,
    pub publish_payload: String,
    /// Topic selected from live values for JSON path editing.
    pub inspect_topic: String,
    pub parse_path: String,
    pub parse_point: String,
    pub parse_rules: Vec<MqttParseRule>,
}

impl Default for MqttForm {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: "1883".into(),
            client_id: "protocol-bench".into(),
            username: String::new(),
            password: String::new(),
            transport: "tcp".into(),
            subscribe_topic: "pb/#".into(),
            publish_topic: "pb/out".into(),
            publish_payload: "hello".into(),
            inspect_topic: String::new(),
            parse_path: String::new(),
            parse_point: String::new(),
            parse_rules: Vec::new(),
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
    mqtt: &mut MqttForm,
) -> crate::hubs::HubChange {
    match pane {
        DockPane::Project => project_panel(ui, hubs, p),
        DockPane::Hubs => crate::hubs::paint(ui, hubs, p),
        DockPane::Mqtt => {
            mqtt_panel(ui, host, p, engine, mqtt);
            crate::hubs::HubChange::quiet()
        }
        DockPane::Points => {
            points_panel(ui, host, p);
            crate::hubs::HubChange::quiet()
        }
        _ => {
            scaffold_panel(ui, pane, host, p);
            crate::hubs::HubChange::quiet()
        }
    }
}

fn mqtt_panel(
    ui: &mut egui::Ui,
    host: &HostSnapshot,
    p: &ThemePalette,
    engine: &EngineHandle,
    form: &mut MqttForm,
) {
    egui::ScrollArea::vertical()
        .id_salt("pb_mqtt_panel")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            // Keep labels/controls off the pane edge.
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    mqtt_panel_body(ui, host, p, engine, form);
                });
                ui.add_space(8.0);
            });
            ui.add_space(8.0);
        });
}

fn mqtt_panel_body(
    ui: &mut egui::Ui,
    host: &HostSnapshot,
    p: &ThemePalette,
    engine: &EngineHandle,
    form: &mut MqttForm,
) {
    let slot = host.slot(EngineId::Mqtt);
    let connected = slot.map(|s| s.connected).unwrap_or(false);
    let state = slot.map(|s| s.state.as_str()).unwrap_or("idle");
    let pack_loaded = slot.map(|s| s.pack_loaded).unwrap_or(false);
    let last_error = slot.map(|s| s.last_error.as_str()).unwrap_or("");
    let topics = slot.map(|s| s.topics.clone()).unwrap_or_default();

    ui.label(
        egui::RichText::new(if pack_loaded {
            "MQTT client pack loaded. Broker / hub role is not active yet — use as source or output."
        } else {
            "MQTT client pack not loaded. Build pb-mqtt into modules/ (see docs/protocol-pack-loading.md)."
        })
        .font(font_ui(12.5))
        .color(p.muted),
    );
    ui.add_space(8.0);

    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!("State: {state}"))
                .font(font_semi(13.0))
                .color(if connected { p.accent } else { p.text }),
        );
        ui.add_space(12.0);
        ui.label(
            egui::RichText::new(format!("Points: {}", host.point_count))
                .font(font_ui(12.5))
                .color(p.muted),
        );
    });
    if !last_error.is_empty() {
        ui.label(
            egui::RichText::new(format!("Last error: {last_error}"))
                .font(font_ui(12.0))
                .color(Color32::from_rgb(0xc0, 0x4a, 0x3a)),
        );
    }
    if !topics.is_empty() {
        ui.label(
            egui::RichText::new(format!("Subscribed: {}", topics.join(", ")))
                .font(font_ui(12.0))
                .color(p.muted),
        );
    }
    ui.add_space(8.0);
    mqtt_live_values(ui, host, &topics, form, p);
    ui.add_space(12.0);

    section_label(ui, "CONNECTION", p);
    field_row(ui, "Host", &mut form.host, p);
    field_row(ui, "Port", &mut form.port, p);
    field_row(ui, "Client ID", &mut form.client_id, p);
    field_row(ui, "Username", &mut form.username, p);
    field_row_password(ui, "Password", &mut form.password, p);
    field_row(ui, "Transport", &mut form.transport, p);
    ui.add_space(2.0);
    ui.label(
        egui::RichText::new("tcp · tls · ws · wss (password in config; no keyring yet)")
            .font(font_ui(11.0))
            .color(p.muted),
    );
    ui.add_space(10.0);

    ui.horizontal(|ui| {
        if text_btn(ui, "Connect", true, p).clicked() {
            push_mqtt_config(engine, form);
            engine.submit(EngineId::Mqtt, EngineCommand::Connect);
            if !form.subscribe_topic.trim().is_empty() {
                engine.submit(
                    EngineId::Mqtt,
                    EngineCommand::Subscribe {
                        topic: form.subscribe_topic.trim().to_string(),
                        qos: 0,
                    },
                );
            }
        }
        ui.add_space(12.0);
        if text_btn(ui, "Disconnect", false, p).clicked() {
            engine.submit(EngineId::Mqtt, EngineCommand::Disconnect);
        }
    });
    ui.add_space(12.0);
    field_row(ui, "Subscribe", &mut form.subscribe_topic, p);
    ui.add_space(4.0);
    if text_btn(ui, "Subscribe now", false, p).clicked() {
        engine.submit(
            EngineId::Mqtt,
            EngineCommand::Subscribe {
                topic: form.subscribe_topic.trim().to_string(),
                qos: 0,
            },
        );
    }
    ui.add_space(10.0);
    field_row(ui, "Publish topic", &mut form.publish_topic, p);
    field_row(ui, "Payload", &mut form.publish_payload, p);
    ui.add_space(4.0);
    if text_btn(ui, "Publish", false, p).clicked() {
        engine.submit(
            EngineId::Mqtt,
            EngineCommand::Publish {
                topic: form.publish_topic.trim().to_string(),
                payload: form.publish_payload.as_bytes().to_vec(),
                qos: 0,
                retain: false,
            },
        );
    }

    ui.add_space(14.0);
    mqtt_json_paths(ui, host, form, engine, p);
}

fn push_mqtt_config(engine: &EngineHandle, form: &MqttForm) {
    let port: u16 = form.port.parse().unwrap_or(1883);
    let parse_rules: Vec<serde_json::Value> = form
        .parse_rules
        .iter()
        .map(|r| {
            serde_json::json!({
                "topic": r.topic,
                "path": r.path,
                "point": r.point,
            })
        })
        .collect();
    let json = serde_json::json!({
        "host": form.host,
        "port": port,
        "client_id": form.client_id,
        "username": form.username,
        "password": form.password,
        "transport": form.transport,
        "keep_alive_secs": 30,
        "parse_rules": parse_rules,
    });
    engine.submit(
        EngineId::Mqtt,
        EngineCommand::Configure {
            json: json.to_string(),
        },
    );
}

fn section_label(ui: &mut egui::Ui, label: &str, p: &ThemePalette) {
    ui.label(
        egui::RichText::new(label)
            .font(font_ui(10.5))
            .color(p.muted),
    );
    ui.add_space(6.0);
}

fn mqtt_json_paths(
    ui: &mut egui::Ui,
    host: &HostSnapshot,
    form: &mut MqttForm,
    engine: &EngineHandle,
    p: &ThemePalette,
) {
    section_label(ui, "JSON PATHS → POINTS", p);
    ui.label(
        egui::RichText::new(
            "Click a live topic to inspect its payload, then add dotted paths (e.g. temp or $.room.temp). Matching fields become host points.",
        )
        .font(font_ui(12.0))
        .color(p.muted),
    );
    ui.add_space(8.0);

    field_row(ui, "Topic", &mut form.inspect_topic, p);
    let inspect = form.inspect_topic.trim().to_string();
    let latest = if inspect.is_empty() {
        None
    } else {
        host.points
            .iter()
            .find(|pt| {
                pt.source == "mqtt"
                    && !pt.name.contains("::")
                    && pt.name.strip_prefix("mqtt/").unwrap_or(pt.name.as_str()) == inspect
            })
            .map(|pt| pt.value.clone())
    };
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new("Message")
            .font(font_ui(12.0))
            .color(p.muted),
    );
    ui.add_space(2.0);
    let mut message = latest.unwrap_or_else(|| {
        if inspect.is_empty() {
            "(select a live topic above)".into()
        } else {
            "(no payload yet for this topic)".into()
        }
    });
    ui.add(
        egui::TextEdit::multiline(&mut message)
            .desired_width(ui.available_width())
            .desired_rows(4)
            .interactive(false),
    );
    ui.add_space(8.0);
    field_row(ui, "Path", &mut form.parse_path, p);
    field_row(ui, "Point id", &mut form.parse_point, p);
    ui.label(
        egui::RichText::new("Point id optional — default mqtt/<topic>::<path>")
            .font(font_ui(11.0))
            .color(p.muted),
    );
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if text_btn(ui, "Add path", true, p).clicked() {
            let topic = form.inspect_topic.trim().to_string();
            let path = form.parse_path.trim().to_string();
            if !path.is_empty() {
                let point = form.parse_point.trim().to_string();
                let rule = MqttParseRule {
                    topic: if topic.is_empty() {
                        form.subscribe_topic.trim().to_string()
                    } else {
                        topic
                    },
                    path,
                    point,
                };
                if !form.parse_rules.iter().any(|r| r == &rule) {
                    form.parse_rules.push(rule);
                }
                form.parse_path.clear();
                form.parse_point.clear();
                push_mqtt_config(engine, form);
            }
        }
        ui.add_space(12.0);
        if text_btn(ui, "Apply paths", false, p).clicked() {
            push_mqtt_config(engine, form);
        }
    });

    if form.parse_rules.is_empty() {
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new("No parse paths yet.")
                .font(font_ui(12.0))
                .color(p.muted),
        );
        return;
    }
    ui.add_space(8.0);
    let mut remove: Option<usize> = None;
    for (i, rule) in form.parse_rules.iter().enumerate() {
        ui.horizontal(|ui| {
            let label = if rule.point.is_empty() {
                format!("{}  ·  {}", rule.topic, rule.path)
            } else {
                format!("{}  ·  {}  →  {}", rule.topic, rule.path, rule.point)
            };
            ui.label(
                egui::RichText::new(label)
                    .font(font_ui(12.0))
                    .color(p.text),
            );
            ui.add_space(8.0);
            if text_btn(ui, "Remove", false, p).clicked() {
                remove = Some(i);
            }
        });
        ui.add_space(2.0);
    }
    if let Some(i) = remove {
        form.parse_rules.remove(i);
        push_mqtt_config(engine, form);
    }
}

/// Live values for subscribed MQTT topics (from the host points bus).
fn mqtt_live_values(
    ui: &mut egui::Ui,
    host: &HostSnapshot,
    subscriptions: &[String],
    form: &mut MqttForm,
    p: &ThemePalette,
) {
    section_label(ui, "LIVE VALUES", p);
    // Whole-topic payloads first; field points (…::path) listed after.
    let mqtt_points: Vec<_> = host.points_for_source("mqtt").collect();
    if mqtt_points.is_empty() {
        let hint = if subscriptions.is_empty() {
            "Subscribe to a topic — inbound messages become points here."
        } else {
            "Waiting for inbound messages on the subscribed topics…"
        };
        ui.label(
            egui::RichText::new(hint)
                .font(font_ui(12.0))
                .color(p.muted),
        );
        return;
    }
    let height = ((mqtt_points.len() as f32) * 22.0 + 28.0)
        .clamp(72.0, ui.available_height().min(220.0).max(72.0));
    egui::ScrollArea::vertical()
        .id_salt("pb_mqtt_live")
        .max_height(height)
        .show(ui, |ui| {
            egui::Grid::new("pb_mqtt_live_grid")
                .num_columns(3)
                .spacing([12.0, 4.0])
                .striped(true)
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new("TOPIC / POINT")
                            .font(font_ui(10.5))
                            .color(p.muted),
                    );
                    ui.label(
                        egui::RichText::new("VALUE")
                            .font(font_ui(10.5))
                            .color(p.muted),
                    );
                    ui.label(
                        egui::RichText::new("QUALITY")
                            .font(font_ui(10.5))
                            .color(p.muted),
                    );
                    ui.end_row();
                    for point in mqtt_points {
                        let display = point
                            .name
                            .strip_prefix("mqtt/")
                            .unwrap_or(point.name.as_str());
                        let is_field = display.contains("::");
                        let topic_only = display.split("::").next().unwrap_or(display);
                        let selected = form.inspect_topic == topic_only;
                        let topic_resp = ui.add(
                            egui::Label::new(
                                egui::RichText::new(display)
                                    .font(font_ui(12.0))
                                    .color(if selected { p.accent } else { p.text }),
                            )
                            .sense(Sense::click()),
                        );
                        if topic_resp.clicked() && !is_field {
                            form.inspect_topic = topic_only.to_string();
                        } else if topic_resp.clicked() {
                            form.inspect_topic = topic_only.to_string();
                            if let Some((_, path)) = display.split_once("::") {
                                form.parse_path = path.to_string();
                            }
                        }
                        topic_resp.on_hover_text("Click to inspect for JSON paths");
                        ui.label(
                            egui::RichText::new(truncate_value(&point.value, 64))
                                .font(font_ui(12.0))
                                .color(p.text),
                        );
                        ui.label(
                            egui::RichText::new(&point.quality)
                                .font(font_ui(12.0))
                                .color(p.muted),
                        );
                        ui.end_row();
                    }
                });
        });
}

fn truncate_value(value: &str, max: usize) -> String {
    let flat = value.replace('\n', " ");
    if flat.chars().count() <= max {
        flat
    } else {
        let mut out: String = flat.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn points_panel(ui: &mut egui::Ui, host: &HostSnapshot, p: &ThemePalette) {
    ui.label(
        egui::RichText::new(
            "Host points bus — MQTT subscriptions (and later other inputs) land here for mapping/hub conversion.",
        )
        .font(font_ui(12.5))
        .color(p.muted),
    );
    ui.add_space(8.0);
    if host.points.is_empty() {
        ui.label(
            egui::RichText::new("No points yet. Connect MQTT and subscribe to populate the bus.")
                .font(font_ui(12.5))
                .color(p.muted),
        );
        return;
    }
    egui::ScrollArea::vertical()
        .id_salt("pb_points_live")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Grid::new("pb_points_grid")
                .num_columns(5)
                .spacing([10.0, 4.0])
                .striped(true)
                .min_col_width(40.0)
                .show(ui, |ui| {
                    for (label, _) in [
                        ("NAME", 0),
                        ("VALUE", 0),
                        ("QUALITY", 0),
                        ("SOURCE", 0),
                        ("UPDATED", 0),
                    ] {
                        ui.label(
                            egui::RichText::new(label)
                                .font(font_ui(10.5))
                                .color(p.muted),
                        );
                    }
                    ui.end_row();
                    for point in &host.points {
                        ui.label(
                            egui::RichText::new(&point.name)
                                .font(font_ui(12.0))
                                .color(p.text),
                        );
                        ui.label(
                            egui::RichText::new(&point.value)
                                .font(font_ui(12.0))
                                .color(p.text),
                        );
                        ui.label(
                            egui::RichText::new(&point.quality)
                                .font(font_ui(12.0))
                                .color(p.muted),
                        );
                        ui.label(
                            egui::RichText::new(&point.source)
                                .font(font_ui(12.0))
                                .color(p.muted),
                        );
                        ui.label(
                            egui::RichText::new(format_updated(point.updated_ms))
                                .font(font_ui(12.0))
                                .color(p.muted),
                        );
                        ui.end_row();
                    }
                });
        });
}

fn format_updated(updated_ms: u64) -> String {
    if updated_ms == 0 {
        return "—".into();
    }
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(updated_ms);
    let age_secs = now_ms.saturating_sub(updated_ms) / 1000;

    // Fresh samples read as live.
    if age_secs < 5 {
        return "just now".into();
    }

    const WEEK_SECS: u64 = 7 * 24 * 3600;
    if age_secs < WEEK_SECS {
        if age_secs < 60 {
            return format!(
                "{} second{} ago",
                age_secs,
                if age_secs == 1 { "" } else { "s" }
            );
        }
        let minutes = age_secs / 60;
        if minutes < 60 {
            return format!(
                "{} minute{} ago",
                minutes,
                if minutes == 1 { "" } else { "s" }
            );
        }
        // ≥1h and <1 week: hours only (including multi-day ages).
        let hours = age_secs / 3600;
        return format!(
            "{} hour{} ago",
            hours,
            if hours == 1 { "" } else { "s" }
        );
    }

    // Older than a week: local date + time to the minute.
    format_absolute_minutes(updated_ms)
}

fn format_absolute_minutes(updated_ms: u64) -> String {
    let secs = (updated_ms / 1000) as i64;
    match chrono::DateTime::from_timestamp(secs, 0) {
        Some(utc) => utc
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M")
            .to_string(),
        None => "—".into(),
    }
}

const FIELD_LABEL_W: f32 = 100.0;

fn field_row(ui: &mut egui::Ui, label: &str, value: &mut String, p: &ThemePalette) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        ui.add_sized(
            [FIELD_LABEL_W, 20.0],
            egui::Label::new(
                egui::RichText::new(label)
                    .font(font_ui(12.0))
                    .color(p.muted),
            ),
        );
        let edit_w = (ui.available_width() - 8.0).clamp(140.0, 360.0);
        ui.add(egui::TextEdit::singleline(value).desired_width(edit_w));
    });
    ui.add_space(4.0);
}

fn field_row_password(ui: &mut egui::Ui, label: &str, value: &mut String, p: &ThemePalette) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        ui.add_sized(
            [FIELD_LABEL_W, 20.0],
            egui::Label::new(
                egui::RichText::new(label)
                    .font(font_ui(12.0))
                    .color(p.muted),
            ),
        );
        let edit_w = (ui.available_width() - 8.0).clamp(140.0, 360.0);
        ui.add(
            egui::TextEdit::singleline(value)
                .password(true)
                .desired_width(edit_w),
        );
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
            .min_size(Vec2::new(96.0, 28.0)),
    )
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
        DockPane::Points => "One value, quality, and timestamp for every named point.".into(),
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
