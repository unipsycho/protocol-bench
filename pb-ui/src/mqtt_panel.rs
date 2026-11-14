//! MQTT panel: embedded hub broker (rumqttd) + external broker connections.
//!
//! Point promotion lives on hub Input / Output nodes — many Inputs may share
//! one connection; each node owns its own topic/field → point binds.

use egui::{Color32, Layout, Pos2, Rect, Sense, Stroke, UiBuilder, Vec2};
use pb_engine::{EngineCommand, EngineHandle, EngineId, HostSnapshot};
use serde::{Deserialize, Serialize};

use std::collections::BTreeMap;

use crate::hubs::HubModel;
use crate::theme::{font_semi, font_ui, ThemePalette};
use crate::topic_tree::{self, TopicTree, TreeMode};

const SIDE_BY_SIDE_MIN: f32 = 720.0;
const TRANSPORTS: &[&str] = &["tcp", "tls", "ws", "wss"];
const LABEL_W: f32 = 100.0;

/// CONNECT auth user stub for the embedded hub broker (rumqttd later).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrokerUserStub {
    pub username: String,
    pub password: String,
}

fn default_broker_bind() -> String {
    "0.0.0.0".into()
}

fn default_broker_listen_port() -> u16 {
    1883
}

/// Single-project embedded MQTT broker (LAN listen). No live egress yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbeddedBrokerConfig {
    /// When true, UI intends the broker to run (start/stop wired later).
    #[serde(default)]
    pub enabled: bool,
    /// Listen address — LAN default `0.0.0.0`.
    #[serde(default = "default_broker_bind")]
    pub bind: String,
    #[serde(default = "default_broker_listen_port")]
    pub port: u16,
    /// Connect users; empty = anonymous OK.
    #[serde(default)]
    pub users: Vec<BrokerUserStub>,
}

impl Default for EmbeddedBrokerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind: default_broker_bind(),
            port: default_broker_listen_port(),
            users: Vec::new(),
        }
    }
}

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
    /// When true, each Connect uses a fresh random MQTT client id (TopicBench-style).
    pub unique_client_id: bool,
    pub username: String,
    pub password: String,
    pub transport: String,
    /// Optional wide subscribe for traffic / bring-up (not point promotion).
    pub subscriptions: Vec<String>,
    /// Legacy — migrated onto hub Input nodes; kept empty after migrate.
    pub parse_rules: Vec<MqttParseRule>,
    /// Legacy — migrated onto hub Output nodes; kept empty after migrate.
    pub outputs: Vec<MqttOutput>,
    pub auto_connect: bool,
    pub draft_sub: String,
}

impl MqttClient {
    pub fn default_local() -> Self {
        Self {
            id: "local".into(),
            label: "Local broker".into(),
            host: "127.0.0.1".into(),
            port: "1883".into(),
            client_id: "pb-local".into(),
            unique_client_id: false,
            username: String::new(),
            password: String::new(),
            transport: "tcp".into(),
            subscriptions: vec!["pb/#".into()],
            parse_rules: Vec::new(),
            outputs: Vec::new(),
            auto_connect: false,
            draft_sub: String::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct MqttPanelState {
    pub clients: Vec<MqttClient>,
    /// Index into `clients` when a connection is selected; `None` = list only.
    pub selected: Option<usize>,
    /// Per-connection topic trees from pack traffic (UI-owned).
    pub trees: BTreeMap<String, TopicTree>,
    /// Project-wide embedded hub broker (single MQTT broker hub).
    pub embedded: EmbeddedBrokerConfig,
    /// Draft row for adding a Connect user (Enter to add).
    pub draft_broker_user: String,
    pub draft_broker_pass: String,
    /// Live rumqttd listen state (session UI; not persisted).
    pub broker_running: bool,
    pub broker_status: String,
}

impl Default for MqttPanelState {
    fn default() -> Self {
        Self {
            clients: vec![MqttClient::default_local()],
            selected: None,
            trees: BTreeMap::new(),
            embedded: EmbeddedBrokerConfig::default(),
            draft_broker_user: String::new(),
            draft_broker_pass: String::new(),
            broker_running: false,
            broker_status: String::new(),
        }
    }
}

impl MqttPanelState {
    pub fn selected(&self) -> Option<&MqttClient> {
        self.selected.and_then(|i| self.clients.get(i))
    }

    pub fn selected_mut(&mut self) -> Option<&mut MqttClient> {
        self.selected.and_then(|i| self.clients.get_mut(i))
    }

    pub fn connection_choices(&self) -> Vec<(String, String)> {
        self.clients
            .iter()
            .map(|c| {
                let label = if c.label.is_empty() {
                    c.id.clone()
                } else {
                    c.label.clone()
                };
                (c.id.clone(), label)
            })
            .collect()
    }

    /// Refresh topic trees from pack traffic, merged with live point sources.
    /// Points are a fallback when `topics_json` traffic is empty/truncated so the
    /// tree stays in sync with what the Points panel already shows.
    pub fn sync_trees_from_host(&mut self, host: &HostSnapshot) {
        let detail = host
            .slot(EngineId::Mqtt)
            .map(|s| s.detail_json.as_str())
            .unwrap_or("[]");
        // Truncated / invalid JSON must not wipe an existing tree (wildcard flood).
        let by_client = topic_tree::traffic_from_detail_ok(detail);
        let known: std::collections::BTreeSet<_> =
            self.clients.iter().map(|c| c.id.clone()).collect();
        self.trees.retain(|id, _| known.contains(id));
        for c in &self.clients {
            let mut rows = by_client
                .as_ref()
                .and_then(|m| m.get(&c.id).cloned())
                .unwrap_or_default();
            merge_point_topics(&mut rows, host, &c.id);
            if rows.is_empty() {
                // Keep prior tree; do not clear on empty poll.
                self.trees.entry(c.id.clone()).or_default();
                continue;
            }
            self.trees
                .entry(c.id.clone())
                .or_default()
                .sync_from_traffic(&rows);
        }
    }

    pub fn tree_for(&self, conn_id: &str) -> Option<&TopicTree> {
        self.trees.get(conn_id)
    }

    fn ensure_clients(&mut self) {
        if self.clients.is_empty() {
            self.clients.push(MqttClient::default_local());
            self.selected = None;
        } else if let Some(i) = self.selected {
            if i >= self.clients.len() {
                self.selected = None;
            }
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

    /// Fill empty / colliding static MQTT client ids so each connection is unique.
    pub fn ensure_unique_mqtt_client_ids(&mut self) {
        let mut seen = std::collections::BTreeSet::new();
        for c in &mut self.clients {
            let trimmed = c.client_id.trim().to_string();
            if trimmed.is_empty() || seen.contains(&trimmed) {
                c.client_id = fresh_mqtt_client_id(&c.id);
            }
            seen.insert(c.client_id.clone());
        }
    }
}

/// Short random MQTT client id: `pb-<conn>-<hex>`.
pub fn fresh_mqtt_client_id(conn_id: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static SEQ: AtomicU64 = AtomicU64::new(1);
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let mix = t ^ n.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    let conn = if conn_id.trim().is_empty() {
        "mqtt"
    } else {
        conn_id.trim()
    };
    format!("pb-{conn}-{:x}", mix & 0xffff_ffff)
}

/// Push multi-client document to the MQTT pack.
/// Hub Input/Output nodes supply parse/output binds; panel subscriptions are optional wide filters.
pub fn apply_mqtt_session(engine: &EngineHandle, state: &mut MqttPanelState, hubs: &HubModel) {
    state.ensure_unique_mqtt_client_ids();
    let clients: Vec<serde_json::Value> = state
        .clients
        .iter()
        .map(|c| {
            let port: u16 = c.port.parse().unwrap_or(1883);
            let (hub_inputs, hub_outs, extra_subs) = hubs.mqtt_binds_for_connection(&c.id);
            let mut rules: Vec<MqttParseRule> = hub_inputs
                .into_iter()
                .map(|b| MqttParseRule {
                    topic: b.topic,
                    path: b.path,
                    point: b.point,
                })
                .collect();
            let mut outs: Vec<MqttOutput> = hub_outs
                .into_iter()
                .map(|b| MqttOutput {
                    point: b.point,
                    topic: b.topic,
                    qos: b.qos,
                    retain: b.retain,
                })
                .collect();
            // Legacy client-level binds (pre hub-node reshape) still apply until migrated.
            for r in &c.parse_rules {
                if !rules.iter().any(|x| x == r) {
                    rules.push(r.clone());
                }
            }
            for o in &c.outputs {
                if !outs.iter().any(|x| x == o) {
                    outs.push(o.clone());
                }
            }
            let mut subscriptions = c.subscriptions.clone();
            for t in extra_subs {
                if !t.is_empty() && !subscriptions.iter().any(|s| s == &t) {
                    subscriptions.push(t);
                }
            }
            serde_json::json!({
                "id": c.id,
                "label": c.label,
                "host": c.host,
                "port": port,
                "client_id": c.client_id,
                "unique_client_id": c.unique_client_id,
                "username": c.username,
                "password": c.password,
                "transport": c.transport,
                "keep_alive_secs": 30,
                "subscriptions": subscriptions,
                "parse_rules": rules.iter().map(|r| serde_json::json!({
                    "topic": r.topic,
                    "path": r.path,
                    "point": r.point,
                })).collect::<Vec<_>>(),
                "outputs": outs.iter().map(|o| serde_json::json!({
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
    hubs: &HubModel,
) {
    state.ensure_clients();
    state.sync_trees_from_host(host);
    // Claim the full pane body with an explicit rect — nested horizontal rows
    // collapse available_height and clip the config editor.
    let full = ui.available_rect_before_wrap();
    let inner = Rect::from_min_max(
        Pos2::new(full.min.x + 8.0, full.min.y + 4.0),
        Pos2::new(full.max.x - 8.0, full.max.y),
    );
    ui.allocate_new_ui(
        UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::top_down(egui::Align::Min)),
        |ui| paint_body(ui, host, p, engine, state, hubs),
    );
    // Consume the outer rect so the dock body does not layout underneath.
    ui.allocate_rect(full, Sense::hover());
}

fn paint_body(
    ui: &mut egui::Ui,
    host: &HostSnapshot,
    p: &ThemePalette,
    engine: &EngineHandle,
    state: &mut MqttPanelState,
    hubs: &HubModel,
) {
    let slot = host.slot(EngineId::Mqtt);
    let pack_loaded = slot.map(|s| s.pack_loaded).unwrap_or(false);
    if !pack_loaded {
        ui.label(
            egui::RichText::new("MQTT pack not loaded. Build pb-mqtt into modules/.")
                .font(font_ui(12.5))
                .color(p.muted),
        );
        ui.add_space(8.0);
    }

    edit_embedded_broker(ui, state, engine, p);
    ui.add_space(18.0);
    panel_heading(ui, "External brokers / connections", p.live, p);
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(
            "Outbound clients for subscribe / Input traffic and Output publish. Point binds live on hub nodes.",
        )
        .font(font_ui(12.0))
        .color(p.muted),
    );
    ui.add_space(8.0);

    clients_bar(ui, state, engine, hubs, p);
    ui.add_space(10.0);

    if state.selected().is_none() {
        ui.label(
            egui::RichText::new("Select a connection to edit broker settings.")
                .font(font_ui(12.5))
                .color(p.muted),
        );
        return;
    }

    let rem = ui.available_rect_before_wrap();
    let rem_size = rem.size();
    let side_by_side = rem_size.x >= SIDE_BY_SIDE_MIN;
    let col_h = rem_size.y.max(120.0);

    if side_by_side {
        let gap = 16.0;
        let left_w = (rem_size.x * 0.55).clamp(340.0, rem_size.x - 260.0);
        let right_w = (rem_size.x - left_w - gap).max(220.0);
        let left = Rect::from_min_size(rem.min, Vec2::new(left_w, col_h));
        let right = Rect::from_min_size(
            Pos2::new(rem.min.x + left_w + gap, rem.min.y),
            Vec2::new(right_w, col_h),
        );

        ui.allocate_new_ui(
            UiBuilder::new()
                .max_rect(left)
                .layout(Layout::top_down(egui::Align::Min)),
            |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("pb_mqtt_cfg")
                    .max_height(col_h)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_min_width((left_w - 4.0).max(1.0));
                        selected_client_editor(ui, state, engine, hubs, p);
                    });
            },
        );
        ui.allocate_new_ui(
            UiBuilder::new()
                .max_rect(right)
                .layout(Layout::top_down(egui::Align::Min)),
            |ui| topic_tree_preview(ui, state, p, true, col_h),
        );
        ui.allocate_rect(rem, Sense::hover());
    } else {
        ui.allocate_new_ui(
            UiBuilder::new()
                .max_rect(rem)
                .layout(Layout::top_down(egui::Align::Min)),
            |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("pb_mqtt_stack")
                    .max_height(col_h)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_min_width((rem_size.x - 4.0).max(1.0));
                        topic_tree_preview(ui, state, p, false, 220.0);
                        ui.add_space(12.0);
                        selected_client_editor(ui, state, engine, hubs, p);
                    });
            },
        );
        ui.allocate_rect(rem, Sense::hover());
    }
}

fn clients_bar(
    ui: &mut egui::Ui,
    state: &mut MqttPanelState,
    engine: &EngineHandle,
    hubs: &HubModel,
    p: &ThemePalette,
) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
        let mut select = None;
        let mut remove = None;
        for (i, client) in state.clients.iter().enumerate() {
            let selected = state.selected == Some(i);
            let label = if client.label.is_empty() {
                client.id.as_str()
            } else {
                client.label.as_str()
            };
            if text_btn(ui, label, selected, p).clicked() {
                select = Some(i);
            }
            if state.clients.len() > 1
                && text_btn(ui, "×", false, p)
                    .on_hover_text("Remove connection")
                    .clicked()
            {
                remove = Some(i);
            }
        }
        if let Some(i) = select {
            state.selected = if state.selected == Some(i) {
                None
            } else {
                Some(i)
            };
        }
        if let Some(i) = remove {
            let was = state.selected;
            state.clients.remove(i);
            state.selected = match was {
                Some(s) if s == i => None,
                Some(s) if s > i => Some(s - 1),
                other => other,
            };
            state.ensure_clients();
            apply_mqtt_session(engine, state, hubs);
        }
        ui.add_space(8.0);
        if secondary_btn(ui, "+ Add", p).clicked() {
            let id = state.unique_id("broker");
            let mut c = MqttClient::default_local();
            c.id = id.clone();
            c.label = id.clone();
            c.client_id = fresh_mqtt_client_id(&id);
            state.clients.push(c);
            state.selected = Some(state.clients.len() - 1);
            apply_mqtt_session(engine, state, hubs);
        }
    });
}

fn selected_client_editor(
    ui: &mut egui::Ui,
    state: &mut MqttPanelState,
    engine: &EngineHandle,
    hubs: &HubModel,
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
        client_id_field(ui, c, p);
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
            apply_mqtt_session(engine, state, hubs);
        }
        ui.add_space(12.0);
        if text_btn(ui, "Disconnect", false, p).clicked() {
            if let Some(c) = state.selected_mut() {
                c.auto_connect = false;
            }
            apply_mqtt_session(engine, state, hubs);
        }
    });
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new("Password stored in the project JSON (no keyring yet).")
            .font(font_ui(11.0))
            .color(p.muted),
    );

    ui.add_space(14.0);
    wide_subscribe_section(ui, state, engine, hubs, p);
}

fn wide_subscribe_section(
    ui: &mut egui::Ui,
    state: &mut MqttPanelState,
    engine: &EngineHandle,
    hubs: &HubModel,
    p: &ThemePalette,
) {
    section(ui, "WIDE SUBSCRIBE — traffic / bring-up", p);
    ui.label(
        egui::RichText::new(
            "Optional broker filters (e.g. pb/#). Traffic may be broad; only hub Input node paths become points.",
        )
        .font(font_ui(12.0))
        .color(p.muted),
    );
    ui.add_space(6.0);

    let mut remove_sub = None;
    let mut add_sub = false;
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
        if text_btn(ui, "Add filter", false, p).clicked() {
            add_sub = true;
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
    if dirty {
        apply_mqtt_session(engine, state, hubs);
    }
}

fn topic_tree_preview(
    ui: &mut egui::Ui,
    state: &MqttPanelState,
    p: &ThemePalette,
    side: bool,
    col_h: f32,
) {
    section(ui, "TOPIC TREE", p);
    ui.label(
        egui::RichText::new("Subscribed traffic preview — bind topics on hub Input nodes.")
            .font(font_ui(11.0))
            .color(p.muted),
    );
    ui.add_space(4.0);
    let client_id = state.selected().map(|c| c.id.as_str()).unwrap_or("");
    let empty = TopicTree::default();
    let tree = state.tree_for(client_id).unwrap_or(&empty);
    let height = if side {
        (col_h - 48.0).max(80.0)
    } else {
        220.0_f32.min(col_h)
    };
    egui::ScrollArea::vertical()
        .id_salt("pb_mqtt_topic_tree")
        .max_height(height)
        .auto_shrink([false, !side])
        .show(ui, |ui| {
            let _ = topic_tree::paint(ui, tree, client_id, TreeMode::Preview, p);
        });
}

/// Strong panel section title (accent / live color).
fn panel_heading(ui: &mut egui::Ui, title: &str, color: egui::Color32, _p: &ThemePalette) {
    ui.label(
        egui::RichText::new(title)
            .font(font_semi(15.0))
            .color(color),
    );
}

/// Embedded hub broker editor (MQTT panel only). Autosave via `MqttProject`.
fn edit_embedded_broker(
    ui: &mut egui::Ui,
    state: &mut MqttPanelState,
    engine: &EngineHandle,
    p: &ThemePalette,
) {
    panel_heading(ui, "Embedded hub broker", p.accent, p);
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(
            "Single project rumqttd listener for LAN. Empty Connect users allow anonymous.",
        )
        .font(font_ui(12.0))
        .color(p.muted),
    );
    ui.add_space(6.0);
    if ui
        .checkbox(&mut state.embedded.enabled, "Enabled")
        .changed()
    {
        // Persist only — Start still required to listen.
    }
    ui.add_space(4.0);
    {
        let mut bind = state.embedded.bind.clone();
        field(ui, "Bind", &mut bind, p);
        if bind != state.embedded.bind {
            state.embedded.bind = bind;
        }
    }
    {
        let mut port = state.embedded.port.to_string();
        field(ui, "Port", &mut port, p);
        if let Ok(n) = port.trim().parse::<u16>() {
            if n != state.embedded.port {
                state.embedded.port = n;
            }
        }
    }
    ui.add_space(8.0);
    ui.label(
        egui::RichText::new("Connect users")
            .font(font_ui(10.5))
            .color(p.muted),
    );
    ui.add_space(4.0);
    let mut remove = None;
    for (i, user) in state.embedded.users.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!("{}  ·  ••••", user.username))
                    .font(font_ui(12.0))
                    .color(p.text),
            );
            if text_btn(ui, "×", false, p).clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        state.embedded.users.remove(i);
    }
    // Draft fields — Enter in either field adds the user (no Add user button).
    let mut add_user = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        ui.add_sized(
            [LABEL_W, 20.0],
            egui::Label::new(
                egui::RichText::new("Username")
                    .font(font_ui(12.0))
                    .color(p.muted),
            ),
        );
        let r = ui.add(
            egui::TextEdit::singleline(&mut state.draft_broker_user).desired_width(f32::INFINITY),
        );
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            add_user = true;
        }
    });
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        ui.add_sized(
            [LABEL_W, 20.0],
            egui::Label::new(
                egui::RichText::new("Password")
                    .font(font_ui(12.0))
                    .color(p.muted),
            ),
        );
        let r = ui.add(
            egui::TextEdit::singleline(&mut state.draft_broker_pass)
                .password(true)
                .desired_width(f32::INFINITY),
        );
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            add_user = true;
        }
    });
    if add_user {
        let u = state.draft_broker_user.trim().to_string();
        if !u.is_empty() && !state.embedded.users.iter().any(|x| x.username == u) {
            state.embedded.users.push(BrokerUserStub {
                username: u,
                password: state.draft_broker_pass.clone(),
            });
            state.draft_broker_user.clear();
            state.draft_broker_pass.clear();
        }
    }

    ui.add_space(14.0);
    if !state.broker_status.is_empty() {
        let color = if state.broker_running { p.ok } else { p.warn };
        ui.label(
            egui::RichText::new(&state.broker_status)
                .font(font_ui(12.0))
                .color(color),
        );
        ui.add_space(6.0);
    }
    let (start, stop) = {
        let can_start = state.embedded.enabled && !state.broker_running;
        let running = state.broker_running;
        let start_label = if running { "Running" } else { "Start broker" };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let start = text_btn(ui, start_label, can_start, p).clicked() && can_start;
            ui.add_space(18.0);
            let stop = text_btn(ui, "Stop", running, p).clicked() && running;
            (start, stop)
        })
        .inner
    };
    if start {
        start_embedded_broker(engine, state);
    }
    if stop {
        stop_embedded_broker(engine, state);
    }
}

fn start_embedded_broker(engine: &EngineHandle, state: &mut MqttPanelState) {
    let users: Vec<serde_json::Value> = state
        .embedded
        .users
        .iter()
        .map(|u| {
            serde_json::json!({
                "username": u.username,
                "password": u.password,
            })
        })
        .collect();
    let json = serde_json::json!({
        "op": "broker_start",
        "bind": state.embedded.bind,
        "port": state.embedded.port,
        "users": users,
    })
    .to_string();
    engine.submit(EngineId::Mqtt, EngineCommand::Configure { json });
    let addr = format!("{}:{}", state.embedded.bind, state.embedded.port);
    // Engine applies configure asynchronously; probe loopback after a short wait.
    let port = state.embedded.port;
    let probe = std::thread::spawn(move || {
        for _ in 0..20 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            let sock = std::net::SocketAddr::from(([127, 0, 0, 1], port));
            if std::net::TcpStream::connect_timeout(&sock, std::time::Duration::from_millis(100))
                .is_ok()
            {
                return true;
            }
        }
        false
    });
    // Block briefly so the status line is honest before the next frame.
    let ok = probe.join().unwrap_or(false);
    if ok {
        state.broker_running = true;
        state.broker_status = format!("Listening on {addr}");
    } else {
        state.broker_running = false;
        state.broker_status = format!("Could not confirm listen on {addr}");
    }
}

fn stop_embedded_broker(engine: &EngineHandle, state: &mut MqttPanelState) {
    let json = serde_json::json!({ "op": "broker_stop" }).to_string();
    engine.submit(EngineId::Mqtt, EngineCommand::Configure { json });
    state.broker_running = false;
    state.broker_status = "Stopped".into();
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

fn client_id_field(ui: &mut egui::Ui, c: &mut MqttClient, p: &ThemePalette) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        ui.add_sized(
            [LABEL_W, 20.0],
            egui::Label::new(
                egui::RichText::new("Client ID")
                    .font(font_ui(12.0))
                    .color(p.muted),
            ),
        );
        let w = (ui.available_width() - 100.0).clamp(120.0, 280.0);
        let edit = egui::TextEdit::singleline(&mut c.client_id).desired_width(w);
        if c.unique_client_id {
            ui.add_enabled(false, edit);
        } else {
            ui.add(edit);
        }
        ui.checkbox(
            &mut c.unique_client_id,
            egui::RichText::new("Unique")
                .font(font_ui(12.0))
                .color(p.text),
        )
        .on_hover_text(
            "Fresh random MQTT client id on each Connect — avoids collisions across sessions.",
        );
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

fn secondary_btn(ui: &mut egui::Ui, label: &str, p: &ThemePalette) -> egui::Response {
    ui.add(
        egui::Button::new(
            egui::RichText::new(label)
                .font(font_ui(12.5))
                .color(Color32::from_rgb(0x12, 0x14, 0x18)),
        )
        .fill(p.live)
        .stroke(Stroke::new(1.0, p.line))
        .min_size(Vec2::new(72.0, 26.0)),
    )
}

/// Pull topic leaves from host points (`mqtt/<conn>/<topic>[::path]`).
fn merge_point_topics(
    rows: &mut Vec<(String, String, u64)>,
    host: &HostSnapshot,
    conn_id: &str,
) {
    let prefix = format!("mqtt/{conn_id}/");
    for pt in &host.points {
        let src = pt.source.as_str();
        let Some(rest) = src.strip_prefix(&prefix) else {
            continue;
        };
        let topic = rest.split_once("::").map(|(t, _)| t).unwrap_or(rest);
        if topic.is_empty() {
            continue;
        }
        // Whole-topic sources win over field-only paths for the leaf payload.
        let from_field = rest.contains("::");
        if let Some((_, payload, count)) = rows.iter_mut().find(|(t, _, _)| t == topic) {
            if !from_field || payload.is_empty() {
                *payload = pt.value.clone();
            }
            *count = count.saturating_add(1);
        } else {
            rows.push((topic.to_string(), pt.value.clone(), 1));
        }
    }
}

#[allow(dead_code)]
fn _sense() {
    let _ = Sense::click();
    let _ = font_semi(1.0);
}
