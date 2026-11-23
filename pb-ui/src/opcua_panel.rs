//! OPC UA panel: embedded UA server hub + external UA clients.
//! Shared browse widget lives in `opcua_browse` (panel + Hub Input).

use egui::{Color32, Layout, Pos2, Rect, Sense, UiBuilder, Vec2};
use crate::engine_api::EngineHandle;
use pb_engine::{EngineCommand, EngineId, HostSnapshot};
use serde::{Deserialize, Serialize};

use std::collections::BTreeMap;

use crate::connection_list::{self, RowAction};
use crate::hubs::HubModel;
use crate::opcua_browse::{self, BrowseAction, BrowseMode, BrowseTree, FlatBrowseRow};
use crate::theme::{
    after_field, apply_form_spacing, font_semi, font_ui, form_inset, space, ThemePalette,
};

const LABEL_W: f32 = 120.0;
/// Match MQTT: when the pane is wide enough, config + browse sit side-by-side.
const SIDE_BY_SIDE_MIN: f32 = 720.0;
/// Floor for the client detail / browse flex region (PB-90).
const CLIENT_AREA_MIN_H: f32 = 360.0;

fn default_true() -> bool {
    true
}
fn default_port() -> u16 {
    4840
}
fn default_bind() -> String {
    "0.0.0.0".into()
}
fn default_hub_id() -> String {
    "hub".into()
}
fn default_hub_name() -> String {
    "Hub OPC UA".into()
}
fn default_app_uri() -> String {
    "urn:protocol-bench:opcua:hub".into()
}
fn default_app_name() -> String {
    "Protocol Bench".into()
}
fn default_policy() -> String {
    "None".into()
}
fn default_mode() -> String {
    "None".into()
}
fn default_auth() -> String {
    "anonymous".into()
}
fn default_trust() -> String {
    "strict".into()
}
fn default_interval() -> u64 {
    500
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbeddedOpcUaConfig {
    #[serde(default = "default_hub_id")]
    pub id: String,
    #[serde(default = "default_hub_name")]
    pub name: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_app_uri")]
    pub application_uri: String,
    #[serde(default = "default_app_name")]
    pub application_name: String,
    #[serde(default = "default_true")]
    pub allow_anonymous: bool,
    /// Offer Basic256Sha256 Sign&Encrypt endpoint alongside None.
    #[serde(default)]
    pub enable_sign_and_encrypt: bool,
    #[serde(default = "default_true")]
    pub auto_generate_cert: bool,
    #[serde(default)]
    pub auto_trust_clients: bool,
    #[serde(default)]
    pub pki_dir: String,
}

impl Default for EmbeddedOpcUaConfig {
    fn default() -> Self {
        Self {
            id: default_hub_id(),
            name: default_hub_name(),
            enabled: false,
            bind: default_bind(),
            port: default_port(),
            application_uri: default_app_uri(),
            application_name: default_app_name(),
            allow_anonymous: true,
            enable_sign_and_encrypt: false,
            auto_generate_cert: true,
            auto_trust_clients: false,
            pki_dir: String::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct OpcUaClient {
    pub id: String,
    pub label: String,
    pub endpoint_url: String,
    pub security_policy: String,
    pub security_mode: String,
    pub auth: String,
    pub username: String,
    pub password: String,
    pub trust_server: String,
    pub pki_dir: String,
    pub subscription_interval_ms: u64,
    pub auto_connect: bool,
    pub draft_node_id: String,
}

impl OpcUaClient {
    pub fn default_plant() -> Self {
        Self {
            id: "plant".into(),
            label: "Plant UA".into(),
            endpoint_url: "opc.tcp://127.0.0.1:4840".into(),
            security_policy: default_policy(),
            security_mode: default_mode(),
            auth: default_auth(),
            username: String::new(),
            password: String::new(),
            trust_server: default_trust(),
            pki_dir: String::new(),
            subscription_interval_ms: default_interval(),
            auto_connect: false,
            draft_node_id: String::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct OpcUaPanelState {
    pub clients: Vec<OpcUaClient>,
    pub selected: Option<usize>,
    pub embedded: EmbeddedOpcUaConfig,
    pub server_running: bool,
    pub server_status: String,
    pub server_checklist: String,
    pub trees: BTreeMap<String, BrowseTree>,
    pub last_error: String,
    /// Pending list-row delete confirm: (index, display name). Session UI only.
    pub pending_delete: Option<(usize, String)>,
}

impl Default for OpcUaPanelState {
    fn default() -> Self {
        Self {
            clients: vec![OpcUaClient::default_plant()],
            selected: None,
            embedded: EmbeddedOpcUaConfig::default(),
            server_running: false,
            server_status: String::new(),
            server_checklist: String::new(),
            trees: BTreeMap::new(),
            last_error: String::new(),
            pending_delete: None,
        }
    }
}

impl OpcUaPanelState {
    pub fn selected(&self) -> Option<&OpcUaClient> {
        self.selected.and_then(|i| self.clients.get(i))
    }

    pub fn selected_mut(&mut self) -> Option<&mut OpcUaClient> {
        self.selected.and_then(|i| self.clients.get_mut(i))
    }

    pub fn ensure_clients(&mut self) {
        if self.clients.is_empty() {
            self.clients.push(OpcUaClient::default_plant());
        }
    }

    pub fn unique_id(&self, prefix: &str) -> String {
        let mut n = 1;
        loop {
            let id = format!("{prefix}{n}");
            if !self.clients.iter().any(|c| c.id == id) {
                return id;
            }
            n += 1;
        }
    }

    /// Sync browse trees + server status from pack detail_json snapshot.
    pub fn sync_from_host(&mut self, host: &HostSnapshot) {
        let Some(slot) = host.slot(EngineId::OpcUa) else {
            return;
        };
        let raw = slot.detail_json.trim();
        if raw.is_empty() || raw == "{}" || raw == "[]" {
            return;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
            return;
        };
        if let Some(server) = value.get("server") {
            self.server_running = server
                .get("running")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let err = server
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let url = server
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if self.server_running {
                self.server_status = if url.is_empty() {
                    "Running".into()
                } else {
                    format!("Running · {url}")
                };
                self.server_checklist.clear();
            } else if !err.is_empty() {
                self.server_status = err.clone();
            } else {
                self.server_status = "Stopped".into();
            }
        }
        if let Some(clients) = value.get("clients").and_then(|v| v.as_array()) {
            for c in clients {
                let id = c.get("id").and_then(|v| v.as_str()).unwrap_or("");
                if id.is_empty() {
                    continue;
                }
                // PB-89: browse is a parent→children map (legacy array = Objects kids).
                let by_parent = parse_browse_map(c.get("browse"));
                if !by_parent.is_empty() {
                    self.trees
                        .entry(id.to_string())
                        .or_default()
                        .sync_from_parent_map(&by_parent);
                }
                if let Some(err) = c.get("last_error").and_then(|v| v.as_str()) {
                    if !err.is_empty() {
                        self.last_error = format!("{id}: {err}");
                    }
                }
                if let Some(check) = c.get("checklist").and_then(|v| v.as_str()) {
                    if !check.is_empty() {
                        self.server_checklist = check.to_string();
                    }
                }
            }
        }
    }
}

/// Push client list + monitored binds from hub OPC UA Inputs into the pack.
pub fn apply_opcua_session(engine: &EngineHandle, state: &mut OpcUaPanelState, hubs: &HubModel) {
    state.ensure_clients();
    let clients: Vec<serde_json::Value> = state
        .clients
        .iter()
        .map(|c| client_configure_json(c, hubs))
        .collect();
    let json = serde_json::json!({ "clients": clients }).to_string();
    engine.submit(EngineId::OpcUa, EngineCommand::Configure { json });
}

fn client_configure_json(c: &OpcUaClient, hubs: &HubModel) -> serde_json::Value {
    // Collect monitored NodeIds from hub OPC UA Input binds for this client.
    let mut monitored = Vec::new();
    for hub in &hubs.hubs {
        for src in &hub.sources {
            if src.protocol != crate::hubs::HubProto::OpcUa {
                continue;
            }
            if !src.opcua_connection.is_empty() && src.opcua_connection != c.id {
                continue;
            }
            for bind in &src.opcua_inputs {
                if bind.node_id.trim().is_empty() {
                    continue;
                }
                monitored.push(serde_json::json!({
                    "node_id": bind.node_id,
                    "point": bind.point,
                    "mode": bind.mode,
                }));
            }
        }
    }
    serde_json::json!({
        "id": c.id,
        "label": c.label,
        "endpoint_url": c.endpoint_url,
        "security_policy": c.security_policy,
        "security_mode": c.security_mode,
        "auth": c.auth,
        "username": c.username,
        "password": c.password,
        "trust_server": c.trust_server,
        "pki_dir": c.pki_dir,
        "subscription_interval_ms": c.subscription_interval_ms,
        "monitored_nodes": monitored,
        "auto_connect": c.auto_connect,
    })
}

/// Build the address-space snapshot: hub catalog rows (public names), then — if any
/// OPC UA hub allows it — the remaining host points under their internal ids.
/// With no OPC UA hub at all, every host point is exposed (previous behaviour).
pub fn catalog_points(host: &HostSnapshot, hubs: &HubModel) -> Vec<serde_json::Value> {
    use crate::hubs::HubProto;
    let ua: Vec<_> = hubs.hubs.iter().filter(|h| h.protocol == HubProto::OpcUa).collect();
    let expose_unmapped = ua.is_empty() || ua.iter().any(|h| h.opcua_expose_unmapped);
    let mut out = Vec::new();
    let mut used = std::collections::HashSet::new();
    for hub in &ua {
        for e in &hub.opcua_catalog {
            if e.point.is_empty() || e.browse_name.trim().is_empty() || used.contains(&e.point) {
                continue;
            }
            let Some(pt) = host.points.iter().find(|p| p.name == e.point) else { continue };
            used.insert(e.point.clone());
            out.push(serde_json::json!({
                "id": pt.name,
                "value": pt.value,
                "browse_name": e.browse_name.trim(),
                "display_name": "",
                "folder": e.norm_path(),
            }));
        }
    }
    if expose_unmapped {
        for p in &host.points {
            if !used.contains(&p.name) {
                out.push(serde_json::json!({ "id": p.name, "value": p.value }));
            }
        }
    }
    out.truncate(2000);
    out
}

/// Push the catalog-aware snapshot so the embedded server can expose it.
pub fn sync_points_to_server(engine: &EngineHandle, host: &HostSnapshot, hubs: &HubModel) {
    let json = serde_json::json!({
        "op": "sync_points",
        "points": catalog_points(host, hubs),
    })
    .to_string();
    engine.submit(EngineId::OpcUa, EngineCommand::Configure { json });
}

pub fn maybe_auto_start_embedded(
    engine: &EngineHandle,
    state: &mut OpcUaPanelState,
    auto_run: bool,
) {
    if !auto_run || !state.embedded.enabled || state.server_running {
        return;
    }
    start_embedded(engine, state);
}

pub fn paint(
    ui: &mut egui::Ui,
    host: &HostSnapshot,
    p: &ThemePalette,
    engine: &EngineHandle,
    state: &mut OpcUaPanelState,
    hubs: &HubModel,
) {
    state.ensure_clients();
    state.sync_from_host(host);
    if state.server_running {
        sync_points_to_server(engine, host, hubs);
    }
    let full = ui.available_rect_before_wrap();
    let inner = form_inset(full);
    ui.allocate_new_ui(
        UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::top_down(egui::Align::Min)),
        |ui| paint_body(ui, host, p, engine, state, hubs),
    );
    ui.allocate_rect(full, Sense::hover());
}

fn paint_body(
    ui: &mut egui::Ui,
    host: &HostSnapshot,
    p: &ThemePalette,
    engine: &EngineHandle,
    state: &mut OpcUaPanelState,
    hubs: &HubModel,
) {
    // Dock splitters zero spacing — restore form rhythm for this pane (PB-88).
    apply_form_spacing(ui);
    let pack_loaded = host
        .slot(EngineId::OpcUa)
        .map(|s| s.pack_loaded)
        .unwrap_or(false);
    if !pack_loaded {
        ui.label(
            egui::RichText::new("OPC UA pack not loaded. Build pb-opcua into modules/.")
                .font(font_ui(12.5))
                .color(p.muted),
        );
        ui.add_space(space::AFTER_BLURB);
    }

    // When a client is selected, collapse the embedded hub so browse/config
    // can use the remaining dock height (PB-90).
    let client_selected = state.selected().is_some();
    if client_selected {
        let status = embedded_status_label(state);
        egui::CollapsingHeader::new(
            egui::RichText::new(format!("Embedded UA server hub · {status}"))
                .font(font_semi(15.0))
                .color(p.accent),
        )
        .id_salt("pb_opcua_hub_collapse")
        .default_open(false)
        .show(ui, |ui| {
            apply_form_spacing(ui);
            ui.label(
                egui::RichText::new(
                    "Protocol Bench listens as an OPC UA server and exposes the points named by the OPC UA hub catalog under Objects/Points.",
                )
                .font(font_ui(12.0))
                .color(p.muted),
            );
            ui.add_space(space::AFTER_BLURB);
            edit_embedded_fields(ui, state, engine, p);
        });
    } else {
        edit_embedded(ui, state, engine, p);
    }
    ui.add_space(space::SECTION);
    // Title + Add on one row (PB-105).
    if connection_list::heading_with_add(ui, "External UA clients", "+ Add", p.live, p) {
        add_opcua_client(state, engine, hubs);
    }
    ui.add_space(space::AFTER_HEADING);
    ui.label(
        egui::RichText::new(
            "Outbound sessions to plant UA servers. Hub Inputs bind NodeIds via the shared browse tree.",
        )
        .font(font_ui(12.0))
        .color(p.muted),
    );
    ui.add_space(space::AFTER_BLURB);
    clients_list(ui, state, p);
    if let Some(i) = connection_list::confirm_delete(ui.ctx(), &mut state.pending_delete, p) {
        remove_opcua_client(state, engine, hubs, i);
    }
    ui.add_space(space::AFTER_FIELD + 2.0);

    if state.selected().is_none() {
        ui.label(
            egui::RichText::new("Select a client to edit endpoint settings.")
                .font(font_ui(12.5))
                .color(p.muted),
        );
        return;
    }

    // Claim remaining dock height so client config + browse aren't a tiny scroll strip (PB-90).
    let rem = ui.available_rect_before_wrap();
    let rem_size = rem.size();
    let col_h = rem_size.y.max(CLIENT_AREA_MIN_H);
    let side_by_side = rem_size.x >= SIDE_BY_SIDE_MIN;

    if side_by_side {
        let gap = 16.0;
        let left_w = (rem_size.x * 0.48).clamp(300.0, rem_size.x - 280.0);
        let right_w = (rem_size.x - left_w - gap).max(240.0);
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
                    .id_salt("pb_opcua_cfg")
                    .max_height(col_h)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        apply_form_spacing(ui);
                        ui.set_min_width((left_w - 4.0).max(1.0));
                        selected_client_editor(ui, state, engine, hubs, p);
                        paint_connectivity_checklist(ui, state, p);
                    });
            },
        );
        ui.allocate_new_ui(
            UiBuilder::new()
                .max_rect(right)
                .layout(Layout::top_down(egui::Align::Min)),
            |ui| {
                apply_form_spacing(ui);
                browse_preview(ui, state, engine, host, p, col_h);
            },
        );
        ui.allocate_rect(rem, Sense::hover());
    } else {
        ui.allocate_new_ui(
            UiBuilder::new()
                .max_rect(rem)
                .layout(Layout::top_down(egui::Align::Min)),
            |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("pb_opcua_stack")
                    .max_height(col_h)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        apply_form_spacing(ui);
                        ui.set_min_width((rem_size.x - 4.0).max(1.0));
                        selected_client_editor(ui, state, engine, hubs, p);
                        ui.add_space(space::SECTION - 2.0);
                        // Tall tree even when stacked — avoids the old “peek then scroll” strip.
                        browse_preview(ui, state, engine, host, p, (col_h * 0.55).max(240.0));
                        paint_connectivity_checklist(ui, state, p);
                    });
            },
        );
        ui.allocate_rect(rem, Sense::hover());
    }
}

fn paint_connectivity_checklist(ui: &mut egui::Ui, state: &OpcUaPanelState, p: &ThemePalette) {
    if state.last_error.is_empty() && state.server_checklist.is_empty() {
        return;
    }
    ui.add_space(space::AFTER_FIELD + 2.0);
    panel_heading(ui, "Connectivity checklist", Color32::from_rgb(220, 140, 60), p);
    ui.add_space(space::AFTER_HEADING);
    if !state.last_error.is_empty() {
        ui.label(
            egui::RichText::new(&state.last_error)
                .font(font_ui(12.0))
                .color(Color32::from_rgb(220, 100, 80)),
        );
    }
    if !state.server_checklist.is_empty() {
        for line in state.server_checklist.lines() {
            ui.label(
                egui::RichText::new(line)
                    .font(font_ui(12.0))
                    .color(p.muted),
            );
        }
    }
}

fn embedded_status_label(state: &OpcUaPanelState) -> String {
    if !state.server_status.is_empty() {
        state.server_status.clone()
    } else if state.server_running {
        "Running".into()
    } else {
        "Stopped".into()
    }
}

fn edit_embedded(
    ui: &mut egui::Ui,
    state: &mut OpcUaPanelState,
    engine: &EngineHandle,
    p: &ThemePalette,
) {
    panel_heading(ui, "Embedded UA server hub", p.accent, p);
    ui.add_space(space::AFTER_HEADING);
    ui.label(
        egui::RichText::new(
            "Protocol Bench listens as an OPC UA server and exposes the points named by the OPC UA hub catalog under Objects/Points.",
        )
        .font(font_ui(12.0))
        .color(p.muted),
    );
    ui.add_space(space::AFTER_BLURB);
    edit_embedded_fields(ui, state, engine, p);
}

fn edit_embedded_fields(
    ui: &mut egui::Ui,
    state: &mut OpcUaPanelState,
    engine: &EngineHandle,
    p: &ThemePalette,
) {
    let mut dirty = false;
    dirty |= labeled_text(ui, "Name", &mut state.embedded.name, p);
    dirty |= labeled_text(ui, "Bind", &mut state.embedded.bind, p);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::ITEM_X;
        ui.add_sized(
            [LABEL_W, 20.0],
            egui::Label::new(egui::RichText::new("Port").font(font_ui(12.5)).color(p.muted)),
        );
        let mut port = state.embedded.port.to_string();
        if ui
            .add(egui::TextEdit::singleline(&mut port).desired_width(80.0))
            .changed()
        {
            if let Ok(v) = port.parse::<u16>() {
                state.embedded.port = v;
                dirty = true;
            }
        }
    });
    after_field(ui);
    dirty |= labeled_text(ui, "App URI", &mut state.embedded.application_uri, p);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(space::CHECK, space::ITEM_Y);
        if ui
            .checkbox(&mut state.embedded.allow_anonymous, "Anonymous")
            .changed()
        {
            dirty = true;
        }
        if ui
            .checkbox(
                &mut state.embedded.enable_sign_and_encrypt,
                "Sign&Encrypt endpoint",
            )
            .changed()
        {
            dirty = true;
        }
        if ui
            .checkbox(&mut state.embedded.auto_generate_cert, "Auto-cert")
            .changed()
        {
            dirty = true;
        }
        if ui
            .checkbox(&mut state.embedded.auto_trust_clients, "Auto-trust clients")
            .changed()
        {
            dirty = true;
        }
    });
    after_field(ui);
    if state.embedded.allow_anonymous
        && (state.embedded.bind.trim() == "0.0.0.0" || state.embedded.bind.trim() == "::")
    {
        ui.label(
            egui::RichText::new("Warning: anonymous + non-loopback bind exposes the LAN.")
                .font(font_ui(11.5))
                .color(Color32::from_rgb(220, 140, 60)),
        );
        after_field(ui);
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::BUTTON;
        let running = state.server_running;
        if secondary_btn(ui, if running { "Stop" } else { "Start" }, p).clicked() {
            if running {
                stop_embedded(engine, state);
            } else {
                state.embedded.enabled = true;
                start_embedded(engine, state);
            }
        }
        let status = embedded_status_label(state);
        ui.label(
            egui::RichText::new(status)
                .font(font_ui(12.0))
                .color(if running { p.live } else { p.muted }),
        );
    });
    let _ = dirty;
}

fn start_embedded(engine: &EngineHandle, state: &mut OpcUaPanelState) {
    let e = &state.embedded;
    let json = serde_json::json!({
        "op": "server_start",
        "bind": e.bind,
        "port": e.port,
        "application_uri": e.application_uri,
        "application_name": e.application_name,
        "allow_anonymous": e.allow_anonymous,
        "enable_sign_and_encrypt": e.enable_sign_and_encrypt,
        "auto_generate_cert": e.auto_generate_cert,
        "auto_trust_clients": e.auto_trust_clients,
        "pki_dir": e.pki_dir,
    })
    .to_string();
    engine.submit(EngineId::OpcUa, EngineCommand::Configure { json });
    state.server_status = "Starting…".into();
}

/// Project-panel Start / Stop for the embedded OPC UA server.
pub fn set_hub_running(engine: &EngineHandle, state: &mut OpcUaPanelState, run: bool) {
    if run {
        state.embedded.enabled = true;
        start_embedded(engine, state);
    } else {
        stop_embedded(engine, state);
    }
}

fn stop_embedded(engine: &EngineHandle, state: &mut OpcUaPanelState) {
    let json = serde_json::json!({ "op": "server_stop" }).to_string();
    engine.submit(EngineId::OpcUa, EngineCommand::Configure { json });
    state.embedded.enabled = false;
    state.server_running = false;
    state.server_status = "Stopped".into();
}

fn clients_list(ui: &mut egui::Ui, state: &mut OpcUaPanelState, p: &ThemePalette) {
    let mut select = None;
    let show_delete = state.clients.len() > 1;
    for (i, client) in state.clients.iter().enumerate() {
        let selected = state.selected == Some(i);
        let label = if client.label.is_empty() {
            client.id.clone()
        } else {
            client.label.clone()
        };
        let subtitle = client.endpoint_url.clone();
        match connection_list::selectable_row(ui, &label, &subtitle, selected, show_delete, p) {
            Some(RowAction::Select) => select = Some(i),
            Some(RowAction::Delete) => {
                state.pending_delete = Some((i, label));
            }
            None => {}
        }
        ui.add_space(2.0);
    }
    if let Some(i) = select {
        state.selected = if state.selected == Some(i) {
            None
        } else {
            Some(i)
        };
    }
}

fn add_opcua_client(state: &mut OpcUaPanelState, engine: &EngineHandle, hubs: &HubModel) {
    let id = state.unique_id("ua");
    let mut c = OpcUaClient::default_plant();
    c.id = id.clone();
    c.label = id;
    state.clients.push(c);
    state.selected = Some(state.clients.len() - 1);
    apply_opcua_session(engine, state, hubs);
}

fn remove_opcua_client(
    state: &mut OpcUaPanelState,
    engine: &EngineHandle,
    hubs: &HubModel,
    i: usize,
) {
    if i >= state.clients.len() {
        return;
    }
    let was = state.selected;
    let id = state.clients[i].id.clone();
    state.clients.remove(i);
    state.trees.remove(&id);
    state.selected = match was {
        Some(s) if s == i => None,
        Some(s) if s > i => Some(s - 1),
        other => other,
    };
    apply_opcua_session(engine, state, hubs);
}

fn selected_client_editor(
    ui: &mut egui::Ui,
    state: &mut OpcUaPanelState,
    engine: &EngineHandle,
    hubs: &HubModel,
    p: &ThemePalette,
) {
    let mut dirty = false;
    let mut do_connect = false;
    let mut do_disconnect = false;
    let mut do_browse = false;
    {
        let Some(c) = state.selected_mut() else {
            return;
        };
        dirty |= labeled_text(ui, "Id", &mut c.id, p);
        dirty |= labeled_text(ui, "Label", &mut c.label, p);
        dirty |= labeled_text(ui, "Endpoint", &mut c.endpoint_url, p);
        dirty |= labeled_text(ui, "Policy", &mut c.security_policy, p);
        dirty |= labeled_text(ui, "Mode", &mut c.security_mode, p);
        dirty |= labeled_text(ui, "Auth", &mut c.auth, p);
        if c.auth.eq_ignore_ascii_case("username") {
            dirty |= labeled_text(ui, "Username", &mut c.username, p);
            dirty |= labeled_password(ui, "Password", &mut c.password, p);
        }
        dirty |= labeled_text(ui, "Trust", &mut c.trust_server, p);
        after_field(ui);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(space::BUTTON, space::ITEM_Y);
            if secondary_btn(ui, "Connect", p).clicked() {
                c.auto_connect = true;
                do_connect = true;
            }
            if secondary_btn(ui, "Disconnect", p).clicked() {
                c.auto_connect = false;
                do_disconnect = true;
            }
            if secondary_btn(ui, "Browse Objects", p).clicked() {
                do_browse = true;
            }
        });
    }
    if dirty {
        apply_opcua_session(engine, state, hubs);
    }
    if do_connect {
        apply_opcua_session(engine, state, hubs);
        if let Some(c) = state.selected() {
            let json = serde_json::json!({ "op": "connect", "id": c.id }).to_string();
            engine.submit(EngineId::OpcUa, EngineCommand::Configure { json });
        }
    }
    if do_disconnect {
        if let Some(c) = state.selected() {
            let json = serde_json::json!({ "op": "disconnect", "id": c.id }).to_string();
            engine.submit(EngineId::OpcUa, EngineCommand::Configure { json });
        }
        apply_opcua_session(engine, state, hubs);
    }
    if do_browse {
        if let Some(c) = state.selected() {
            let json = serde_json::json!({
                "op": "browse",
                "id": c.id,
                "node_id": "Objects",
            })
            .to_string();
            engine.submit(EngineId::OpcUa, EngineCommand::Configure { json });
        }
    }
}

fn parse_browse_map(value: Option<&serde_json::Value>) -> BTreeMap<String, Vec<FlatBrowseRow>> {
    let Some(value) = value else {
        return BTreeMap::new();
    };
    let parse_row = |n: &serde_json::Value| -> Option<FlatBrowseRow> {
        Some(FlatBrowseRow {
            node_id: n.get("node_id")?.as_str()?.to_string(),
            browse_name: n
                .get("browse_name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            display_name: n
                .get("display_name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            is_variable: n
                .get("is_variable")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        })
    };
    let mut out = BTreeMap::new();
    if let Some(obj) = value.as_object() {
        for (parent, kids) in obj {
            let rows = kids
                .as_array()
                .map(|arr| arr.iter().filter_map(parse_row).collect())
                .unwrap_or_default();
            out.insert(parent.clone(), rows);
        }
        return out;
    }
    // Legacy flat array → Objects children.
    if let Some(arr) = value.as_array() {
        let rows: Vec<FlatBrowseRow> = arr.iter().filter_map(parse_row).collect();
        if !rows.is_empty() {
            out.insert("Objects".into(), rows);
        }
    }
    out
}

fn endpoint_looks_local(url: &str) -> bool {
    let u = url.to_ascii_lowercase();
    u.contains("127.0.0.1") || u.contains("localhost") || u.contains("[::1]")
}

/// Fallback Objects → Points → variables from host points when browsing the
/// local embedded hub and the client has no pack browse yet (PB-89 sample).
fn seed_embedded_points_preview(tree: &mut BrowseTree, host: &HostSnapshot) {
    if !tree.is_empty() {
        return;
    }
    let points_id = "ns=2;s=Points";
    tree.merge_children(
        "Objects",
        &[FlatBrowseRow {
            node_id: points_id.into(),
            browse_name: "Points".into(),
            display_name: "Points".into(),
            is_variable: false,
        }],
    );
    let rows: Vec<FlatBrowseRow> = host
        .points
        .iter()
        .take(48)
        .map(|pt| FlatBrowseRow {
            node_id: format!("ns=2;s={}", pt.name),
            browse_name: pt.name.clone(),
            display_name: pt.name.clone(),
            is_variable: true,
        })
        .collect();
    if rows.is_empty() {
        tree.merge_children(
            points_id,
            &[FlatBrowseRow {
                node_id: "ns=2;s=demo_temperature".into(),
                browse_name: "demo_temperature".into(),
                display_name: "demo_temperature".into(),
                is_variable: true,
            }],
        );
    } else {
        tree.merge_children(points_id, &rows);
    }
}

fn browse_preview(
    ui: &mut egui::Ui,
    state: &mut OpcUaPanelState,
    engine: &EngineHandle,
    host: &HostSnapshot,
    p: &ThemePalette,
    col_h: f32,
) {
    panel_heading(ui, "Browse", p.live, p);
    ui.add_space(space::AFTER_HEADING);
    ui.label(
        egui::RichText::new(
            "Objects → folders → variables. Expand folders to load children; click a Variable to select its NodeId.",
        )
        .font(font_ui(12.0))
        .color(p.muted),
    );
    ui.add_space(space::AFTER_BLURB);
    apply_form_spacing(ui);
    let id = state.selected().map(|c| c.id.clone()).unwrap_or_default();
    let local_preview = state
        .selected()
        .map(|c| endpoint_looks_local(&c.endpoint_url))
        .unwrap_or(false);
    {
        let tree = state.trees.entry(id.clone()).or_default();
        // Local hub only — real Connect/Browse replaces this via pack sync.
        if state.server_running && local_preview {
            seed_embedded_points_preview(tree, host);
        }
    }
    let tree = state.trees.entry(id.clone()).or_default().clone();
    // Header + blurb ~56px; keep the tree on the remaining flex height (PB-90).
    let tree_h = (col_h - 56.0).max(200.0);
    let action =
        opcua_browse::paint(ui, &tree, "pb_opcua_browse", BrowseMode::PickNode, p, tree_h);
    match action {
        Some(BrowseAction::Pick(pick)) => {
            if let Some(c) = state.selected_mut() {
                c.draft_node_id = pick.node_id.clone();
            }
            if let Some(c) = state.selected() {
                let json = serde_json::json!({
                    "op": "monitor",
                    "id": c.id,
                    "node_id": pick.node_id,
                    "point": "",
                    "mode": "subscribe",
                })
                .to_string();
                engine.submit(EngineId::OpcUa, EngineCommand::Configure { json });
            }
        }
        Some(BrowseAction::Expand(node_id)) => {
            if !id.is_empty() {
                let json = serde_json::json!({
                    "op": "browse",
                    "id": id,
                    "node_id": node_id,
                })
                .to_string();
                engine.submit(EngineId::OpcUa, EngineCommand::Configure { json });
            }
        }
        None => {}
    }
    // Show last selected NodeId for bind copy/paste into Hub Input.
    if let Some(c) = state.selected() {
        if !c.draft_node_id.is_empty() {
            ui.add_space(space::AFTER_FIELD);
            ui.label(
                egui::RichText::new(format!("Selected NodeId: {}", c.draft_node_id))
                    .font(font_ui(12.0))
                    .color(p.live),
            );
        }
    }
}

fn panel_heading(ui: &mut egui::Ui, title: &str, accent: Color32, p: &ThemePalette) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(4.0, 14.0), Sense::hover());
        ui.painter().rect_filled(rect, 1.0, accent);
        ui.label(
            egui::RichText::new(title)
                .font(font_semi(13.0))
                .color(p.text),
        );
    });
}

fn labeled_text(ui: &mut egui::Ui, label: &str, value: &mut String, p: &ThemePalette) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::ITEM_X;
        ui.add_sized(
            [LABEL_W, 20.0],
            egui::Label::new(egui::RichText::new(label).font(font_ui(12.5)).color(p.muted)),
        );
        let w = (ui.available_width() - 8.0).clamp(140.0, 360.0);
        changed = ui
            .add(
                egui::TextEdit::singleline(value)
                    .desired_width(w)
                    .margin(egui::Margin::symmetric(6.0, 3.0)),
            )
            .changed();
    });
    // Vertical rhythm from apply_form_spacing ITEM_Y (PB-91 — avoid double gap).
    changed
}

fn labeled_password(ui: &mut egui::Ui, label: &str, value: &mut String, p: &ThemePalette) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::ITEM_X;
        ui.add_sized(
            [LABEL_W, 20.0],
            egui::Label::new(egui::RichText::new(label).font(font_ui(12.5)).color(p.muted)),
        );
        let w = (ui.available_width() - 8.0).clamp(140.0, 360.0);
        changed = ui
            .add(
                egui::TextEdit::singleline(value)
                    .password(true)
                    .desired_width(w)
                    .margin(egui::Margin::symmetric(6.0, 3.0)),
            )
            .changed();
    });
    changed
}

fn secondary_btn(ui: &mut egui::Ui, label: &str, p: &ThemePalette) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(label).font(font_ui(12.0)).color(p.text))
            .fill(p.control)
            .stroke(egui::Stroke::new(1.0, p.line)),
    )
}

#[cfg(test)]
mod catalog_tests {
    use super::*;
    use crate::hubs::{HubModel, HubProto, OpcUaCatalogEntry};
    use pb_engine::{HostSnapshot, PointSnapshot};

    fn host() -> HostSnapshot {
        let points = [("mqtt_1", "5"), ("mqtt_2", "6")]
            .iter()
            .map(|(n, v)| PointSnapshot {
                name: (*n).into(),
                value: (*v).into(),
                quality: "good".into(),
                source: String::new(),
                updated_ms: 0,
            })
            .collect::<Vec<_>>();
        HostSnapshot {
            thread_name: String::new(),
            alive: true,
            ticks: 0,
            uptime_ms: 0,
            slots: vec![],
            modules_dir: String::new(),
            packs_loaded: vec![],
            point_count: points.len(),
            points,
        }
    }

    #[test]
    fn catalog_renames_and_optionally_dumps_rest() {
        let mut hubs = HubModel::default();
        hubs.add_hub();
        let id = hubs.hubs[0].id;
        hubs.set_hub_protocol(id, HubProto::OpcUa);
        hubs.hubs[0].opcua_catalog.push(OpcUaCatalogEntry {
            point: "mqtt_1".into(),
            path: "/WellA/Tank1/".into(),
            browse_name: "Level".into(),
            display_name: String::new(),
        });
        let pts = catalog_points(&host(), &hubs);
        assert_eq!(pts.len(), 2);
        assert_eq!(pts[0]["browse_name"], "Level");
        assert_eq!(pts[0]["folder"], "WellA/Tank1");
        assert_eq!(pts[1]["id"], "mqtt_2");
        hubs.hubs[0].opcua_expose_unmapped = false;
        assert_eq!(catalog_points(&host(), &hubs).len(), 1);
    }
}
