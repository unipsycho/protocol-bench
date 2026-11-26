//! Modbus panel — connections only (client + server, TCP + RTU).
//!
//! Same split as MQTT / HTTP: this panel owns **endpoints**; hub Input / Output nodes own the
//! register binds (area + address + type → `modbus_N`). A **Server** connection is what a
//! Modbus hub listens on; **Client** connections are polled / written by Input / Output nodes.

use egui::{Layout, RichText, Sense, UiBuilder};
use pb_engine::{EngineId, HostSnapshot, PointSnapshot};
use pb_modbus::codec::{Area, ByteOrder, DataType};
use serde::{Deserialize, Serialize};

use crate::connection_list::{self, RowAction};
use crate::theme::{apply_form_spacing, font_ui, form_inset, space, ThemePalette};

// ---------------------------------------------------------------------------
// Live state (from the pack's `detail_json`)
// ---------------------------------------------------------------------------

/// One connection as the Modbus pack reports it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LiveConn {
    pub id: String,
    pub role: String,
    pub endpoint: String,
    pub state: String,
    pub connected: bool,
    pub error: String,
    pub polls: u64,
    pub errors: u64,
    /// Connected masters (servers only).
    pub clients: Option<u64>,
}

pub fn parse_live(detail_json: &str) -> Vec<LiveConn> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(detail_json) else {
        return Vec::new();
    };
    let str_of = |c: &serde_json::Value, k: &str| c.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    v.get("connections")
        .and_then(|c| c.as_array())
        .into_iter()
        .flatten()
        .map(|c| LiveConn {
            id: str_of(c, "id"),
            role: str_of(c, "role"),
            endpoint: str_of(c, "endpoint"),
            state: str_of(c, "state"),
            connected: c.get("connected").and_then(|x| x.as_bool()).unwrap_or(false),
            error: str_of(c, "error"),
            polls: c.get("polls").and_then(|x| x.as_u64()).unwrap_or(0),
            errors: c.get("errors").and_then(|x| x.as_u64()).unwrap_or(0),
            clients: c.get("clients").and_then(|x| x.as_u64()),
        })
        .collect()
}

/// Live connections from the host snapshot, plus whether the pack is loaded at all.
pub fn live_from_host(host: &HostSnapshot) -> (Vec<LiveConn>, bool) {
    match host.slot(EngineId::Modbus) {
        Some(slot) => (parse_live(&slot.detail_json), slot.pack_loaded),
        None => (Vec::new(), false),
    }
}

pub fn point_by_name<'a>(host: &'a HostSnapshot, name: &str) -> Option<&'a PointSnapshot> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    host.points.iter().find(|p| p.name == name)
}

fn quality_color(q: &str, p: &ThemePalette) -> egui::Color32 {
    match q.to_ascii_lowercase().as_str() {
        "good" => p.live,
        "bad" | "error" => p.err,
        _ => p.warn,
    }
}

/// `●` + value for a bound point, or `—` when it has not been seen yet.
pub fn live_value_label(ui: &mut egui::Ui, host: &HostSnapshot, point: &str, p: &ThemePalette) {
    match point_by_name(host, point) {
        Some(pt) => {
            ui.label(RichText::new("●").font(font_ui(11.0)).color(quality_color(&pt.quality, p)));
            let v: String = pt.value.chars().take(24).collect();
            ui.label(RichText::new(v).font(font_ui(12.0)).color(p.text))
                .on_hover_text(format!("quality: {}", pt.quality));
        }
        None => {
            ui.label(RichText::new("—").font(font_ui(12.0)).color(p.muted));
        }
    }
}

/// One status line: coloured dot, state, and the last error underneath.
pub fn status_line(ui: &mut egui::Ui, live: Option<&LiveConn>, pack_loaded: bool, role: ModbusRole, p: &ThemePalette) {
    let (color, text) = if !pack_loaded {
        (p.warn, "Modbus pack not loaded — copy libpb_modbus into the service modules/ folder".to_string())
    } else {
        match live {
            None => (
                p.muted,
                match role {
                    ModbusRole::Server => "Not running — pick this connection on a Modbus hub".to_string(),
                    ModbusRole::Client => "Waiting for the service to apply this connection".to_string(),
                },
            ),
            Some(l) if l.connected => (p.live, l.state.clone()),
            Some(l) if l.state == "error" => (p.err, "Error".to_string()),
            Some(l) => (p.warn, l.state.clone()),
        }
    };
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = space::ITEM_X;
        ui.label(RichText::new("●").font(font_ui(12.0)).color(color));
        ui.label(RichText::new(text).font(font_ui(12.0)).color(p.text));
    });
    if let Some(l) = live {
        if !l.error.is_empty() {
            ui.label(RichText::new(&l.error).font(font_ui(11.5)).color(p.warn));
        }
    }
}

const LABEL_W: f32 = 110.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModbusRole {
    /// Master — polls / writes a remote device.
    Client,
    /// Slave — listens; the hub's register image is served to remote masters.
    Server,
}

impl ModbusRole {
    pub fn label(self) -> &'static str {
        match self {
            Self::Client => "Client (master)",
            Self::Server => "Server (slave)",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModbusTransport {
    Tcp,
    Rtu,
}

impl ModbusTransport {
    pub fn label(self) -> &'static str {
        match self {
            Self::Tcp => "TCP",
            Self::Rtu => "RTU (serial)",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModbusParity {
    None,
    Even,
    Odd,
}

impl ModbusParity {
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Even => "Even",
            Self::Odd => "Odd",
        }
    }
}


/// One register bind. Client Input: register → host point (`modbus_N`). Client Output: host
/// point → register. Server map entry: host point served at this address.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModbusBind {
    /// Tag label shown in the register map (`Discharge pressure`). Optional.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// `coil` | `discrete` | `input` | `holding`.
    #[serde(default = "default_modbus_area")]
    pub area: String,
    #[serde(default)]
    pub address: u16,
    /// `bool` | `u16` | `i16` | `u32` | `i32` | `u64` | `i64` | `f32` | `f64` | `bit0..15` | `strN`.
    #[serde(default = "default_modbus_type")]
    pub dtype: String,
    /// Byte / word order for multi-byte types (`abcd` | `cdab` | `badc` | `dcba`).
    #[serde(default = "default_modbus_order")]
    pub order: String,
    /// Unit / slave id override; 0 = connection default. Unused on server map entries.
    #[serde(default)]
    pub unit: u8,
    /// Input: empty on new binds — host assigns `modbus_N`. Output / server map: the host point.
    #[serde(default)]
    pub point: String,
    /// Engineering scaling: `value = raw × scale + offset` (writes invert it).
    #[serde(default = "default_scale", skip_serializing_if = "is_one")]
    pub scale: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub offset: f64,
    /// Engineering units label (`psi`, `°C`, `bbl/min`). Display only.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub units: String,
}

pub fn default_scale() -> f64 {
    1.0
}
fn is_one(v: &f64) -> bool {
    *v == 1.0
}
fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

pub fn default_modbus_area() -> String {
    "holding".into()
}
pub fn default_modbus_type() -> String {
    "u16".into()
}
pub fn default_modbus_order() -> String {
    "abcd".into()
}

impl Default for ModbusBind {
    fn default() -> Self {
        Self {
            area: default_modbus_area(),
            address: 0,
            dtype: default_modbus_type(),
            order: default_modbus_order(),
            unit: 0,
            point: String::new(),
            name: String::new(),
            scale: 1.0,
            offset: 0.0,
            units: String::new(),
        }
    }
}

impl ModbusBind {
    pub const AREAS: [(&'static str, &'static str); 4] = [
        ("coil", "Coil (0x)"),
        ("discrete", "Discrete input (1x)"),
        ("input", "Input register (3x)"),
        ("holding", "Holding register (4x)"),
    ];

    pub fn data_type(&self) -> Option<DataType> {
        DataType::parse(&self.dtype)
    }

    pub fn byte_order(&self) -> ByteOrder {
        ByteOrder::parse(&self.order).unwrap_or(ByteOrder::Abcd)
    }

    /// Registers occupied (1 for bit areas).
    pub fn width(&self) -> u16 {
        self.data_type().map(|t| t.width()).unwrap_or(1)
    }

    /// Short human form: `holding 100 f32 cdab`.
    pub fn describe(&self) -> String {
        let shows_order = self.data_type().is_some_and(|t| t.uses_order())
            && self.byte_order() != ByteOrder::Abcd;
        let order = if shows_order { format!(" {}", self.order) } else { String::new() };
        format!("{} {} {}{order}", self.area, self.address, self.dtype)
    }

    /// First problem with this bind, if any. `need_point` for Output / server-map entries.
    pub fn problem(&self, need_point: bool) -> Option<String> {
        let Some(area) = Area::parse(&self.area) else {
            return Some(format!("unknown area \"{}\"", self.area));
        };
        let Some(dt) = self.data_type() else {
            return Some(format!("unknown type \"{}\"", self.dtype));
        };
        if ByteOrder::parse(&self.order).is_none() {
            return Some(format!("unknown byte order \"{}\"", self.order));
        }
        if area.is_bit() != (dt == DataType::Bool) {
            return Some(if area.is_bit() {
                "coil / discrete areas hold bool only".into()
            } else {
                "bool needs a coil or discrete area".into()
            });
        }
        if self.address as u32 + dt.width() as u32 > 65_536 {
            return Some("runs past address 65535".into());
        }
        if need_point && self.point.trim().is_empty() {
            return Some("no host point".into());
        }
        None
    }

    fn span(&self) -> (u32, u32) {
        (self.address as u32, self.address as u32 + self.width() as u32)
    }
}

/// Conflicts in a server register map: overlapping ranges within one area, or bad entries.
pub fn map_problems(map: &[ModbusBind]) -> Vec<String> {
    let mut out = Vec::new();
    for (i, a) in map.iter().enumerate() {
        if let Some(p) = a.problem(true) {
            out.push(format!("{}: {p}", a.describe()));
        }
        for b in &map[i + 1..] {
            if a.area == b.area {
                let (a0, a1) = a.span();
                let (b0, b1) = b.span();
                if a0 < b1 && b0 < a1 {
                    out.push(format!("{} overlaps {}", a.describe(), b.describe()));
                }
            }
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModbusConn {
    pub id: String,
    #[serde(default)]
    pub label: String,
    pub role: ModbusRole,
    pub transport: ModbusTransport,
    /// TCP client: remote host. TCP server: bind address.
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_tcp_port")]
    pub port: u16,
    /// RTU: serial device (`/dev/ttyUSB0`, `COM3`).
    #[serde(default)]
    pub serial_port: String,
    #[serde(default = "default_baud")]
    pub baud: u32,
    #[serde(default = "default_data_bits")]
    pub data_bits: u8,
    #[serde(default = "default_parity")]
    pub parity: ModbusParity,
    #[serde(default = "default_stop_bits")]
    pub stop_bits: u8,
    /// Client: default slave / unit id. Server: the unit id this slave answers to.
    #[serde(default = "default_unit")]
    pub unit_id: u8,
    /// Client: response timeout (ms).
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u32,
    /// Client: retries before the link is marked bad (quality `bad`, then reconnect).
    #[serde(default = "default_retries")]
    pub retries: u8,
    /// Off = the service keeps the connection configured but does not connect / listen.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Server only: register image — which host point is served at which address.
    #[serde(default)]
    pub map: Vec<ModbusBind>,
}

fn default_enabled() -> bool {
    true
}
fn default_host() -> String {
    "127.0.0.1".into()
}
fn default_tcp_port() -> u16 {
    502
}
fn default_baud() -> u32 {
    9600
}
fn default_data_bits() -> u8 {
    8
}
fn default_parity() -> ModbusParity {
    ModbusParity::None
}
fn default_stop_bits() -> u8 {
    1
}
fn default_unit() -> u8 {
    1
}
fn default_timeout_ms() -> u32 {
    1000
}
fn default_retries() -> u8 {
    3
}

impl ModbusConn {
    pub fn default_client() -> Self {
        Self {
            id: "plc".into(),
            label: "Plant PLC".into(),
            role: ModbusRole::Client,
            transport: ModbusTransport::Tcp,
            host: default_host(),
            port: default_tcp_port(),
            serial_port: String::new(),
            baud: default_baud(),
            data_bits: default_data_bits(),
            parity: default_parity(),
            stop_bits: default_stop_bits(),
            unit_id: default_unit(),
            timeout_ms: default_timeout_ms(),
            retries: default_retries(),
            enabled: true,
            map: Vec::new(),
        }
    }

    pub fn default_server() -> Self {
        Self {
            id: "hub".into(),
            label: "Hub server".into(),
            role: ModbusRole::Server,
            host: "0.0.0.0".into(),
            ..Self::default_client()
        }
    }

    pub fn display_name(&self) -> &str {
        if self.label.trim().is_empty() {
            &self.id
        } else {
            &self.label
        }
    }

    /// One-line endpoint summary for list rows and Points sources.
    pub fn summary(&self) -> String {
        let role = match self.role {
            ModbusRole::Client => "client",
            ModbusRole::Server => "server",
        };
        match self.transport {
            ModbusTransport::Tcp => {
                let host = if self.host.trim().is_empty() {
                    "(no host)"
                } else {
                    self.host.trim()
                };
                format!("TCP {role}  ·  {host}:{}  ·  unit {}", self.port, self.unit_id)
            }
            ModbusTransport::Rtu => {
                let dev = if self.serial_port.trim().is_empty() {
                    "(no port)"
                } else {
                    self.serial_port.trim()
                };
                format!(
                    "RTU {role}  ·  {dev} {}{}{}  ·  unit {}",
                    self.baud,
                    self.parity.label().chars().next().unwrap_or('N'),
                    self.stop_bits,
                    self.unit_id
                )
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct ModbusPanelState {
    pub conns: Vec<ModbusConn>,
    pub selected: Option<usize>,
    pub pending_delete: Option<(usize, String)>,
}

impl Default for ModbusPanelState {
    fn default() -> Self {
        Self {
            conns: vec![ModbusConn::default_client(), ModbusConn::default_server()],
            selected: None,
            pending_delete: None,
        }
    }
}

impl ModbusPanelState {
    pub fn selected_mut(&mut self) -> Option<&mut ModbusConn> {
        self.selected.and_then(|i| self.conns.get_mut(i))
    }

    /// `(id, label)` for connections of one role (Input/Output pick clients, hubs pick servers).
    pub fn choices(&self, role: ModbusRole) -> Vec<(String, String)> {
        self.conns
            .iter()
            .filter(|c| c.role == role)
            .map(|c| (c.id.clone(), c.display_name().to_string()))
            .collect()
    }

    fn ensure_conns(&mut self) {
        if self.conns.is_empty() {
            self.conns.push(ModbusConn::default_client());
            self.selected = None;
        } else if self.selected.is_some_and(|i| i >= self.conns.len()) {
            self.selected = None;
        }
    }

    fn unique_id(&self, base: &str) -> String {
        connection_list::unique_id(base, |id| self.conns.iter().any(|c| c.id == id))
    }
}

pub fn paint(ui: &mut egui::Ui, p: &ThemePalette, state: &mut ModbusPanelState, host: &HostSnapshot) -> bool {
    state.ensure_conns();
    let mut dirty = false;
    let full = ui.available_rect_before_wrap();
    let inner = form_inset(full);
    ui.allocate_new_ui(
        UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::top_down(egui::Align::Min)),
        |ui| {
            dirty = paint_body(ui, p, state, host);
        },
    );
    ui.allocate_rect(full, Sense::hover());
    dirty
}

fn paint_body(ui: &mut egui::Ui, p: &ThemePalette, state: &mut ModbusPanelState, host: &HostSnapshot) -> bool {
    apply_form_spacing(ui);
    let (live, pack_loaded) = live_from_host(host);
    let mut dirty = false;
    ui.label(
        RichText::new(
            "Modbus TCP / RTU links. Clients feed hub Inputs (poll) and Outputs (write); \
             a Server is what a Modbus hub listens on.",
        )
        .font(font_ui(12.5))
        .color(p.muted),
    );
    ui.add_space(space::AFTER_BLURB);

    if connection_list::heading_with_add(ui, "Connections", "+ Add", p.live, p) {
        let mut c = ModbusConn::default_client();
        c.id = state.unique_id("modbus");
        c.label = c.id.clone();
        state.conns.push(c);
        state.selected = Some(state.conns.len() - 1);
        dirty = true;
    }
    ui.add_space(space::AFTER_HEADING);
    ui.label(
        RichText::new("Endpoints only — bind registers on hub Input / Output nodes.")
            .font(font_ui(12.0))
            .color(p.muted),
    );
    ui.add_space(space::AFTER_BLURB);

    let mut select = None;
    let show_delete = state.conns.len() > 1;
    for (i, c) in state.conns.iter().enumerate() {
        let selected = state.selected == Some(i);
        let label = c.display_name().to_string();
        let status = match live.iter().find(|l| l.id == c.id) {
            Some(l) if l.connected => l.state.clone(),
            Some(l) if l.state == "error" => "error".to_string(),
            Some(l) => l.state.clone(),
            None if !pack_loaded => "pack not loaded".to_string(),
            None => "not running".to_string(),
        };
        let subtitle = format!("{}  ·  {status}", c.summary());
        match connection_list::selectable_row(ui, &label, &subtitle, selected, show_delete, p) {
            Some(RowAction::Select) => select = Some(i),
            Some(RowAction::Delete) => state.pending_delete = Some((i, label)),
            None => {}
        }
        ui.add_space(2.0);
    }
    if let Some(i) = select {
        state.selected = if state.selected == Some(i) { None } else { Some(i) };
    }
    if let Some(i) = connection_list::confirm_delete(ui.ctx(), &mut state.pending_delete, p) {
        if i < state.conns.len() {
            let was = state.selected;
            state.conns.remove(i);
            state.selected = match was {
                Some(s) if s == i => None,
                Some(s) if s > i => Some(s - 1),
                other => other,
            };
            state.ensure_conns();
            dirty = true;
        }
    }

    ui.add_space(space::AFTER_FIELD + 2.0);
    if state.selected.is_none() {
        ui.label(
            RichText::new("Select a connection to edit.")
                .font(font_ui(12.5))
                .color(p.muted),
        );
        return dirty;
    }
    egui::ScrollArea::vertical()
        .id_salt("pb_modbus_cfg")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            dirty |= edit_selected(ui, state, p, host, &live, pack_loaded);
        });
    dirty
}

fn edit_selected(
    ui: &mut egui::Ui,
    state: &mut ModbusPanelState,
    p: &ThemePalette,
    host: &HostSnapshot,
    live: &[LiveConn],
    pack_loaded: bool,
) -> bool {
    let mut dirty = false;
    // Ids must stay unique — compare against the others before accepting an edit.
    let others: Vec<String> = state
        .conns
        .iter()
        .enumerate()
        .filter(|(i, _)| Some(*i) != state.selected)
        .map(|(_, c)| c.id.clone())
        .collect();
    let Some(c) = state.selected_mut() else {
        return false;
    };
    section(ui, "STATUS", p);
    let mine = live.iter().find(|l| l.id == c.id);
    status_line(ui, mine, pack_loaded, c.role, p);
    if let Some(l) = mine {
        let mut bits: Vec<String> = Vec::new();
        if c.role == ModbusRole::Client {
            bits.push(format!("{} polls", l.polls));
        }
        if let Some(n) = l.clients {
            bits.push(format!("{n} master{}", if n == 1 { "" } else { "s" }));
        }
        bits.push(format!("{} errors", l.errors));
        ui.label(RichText::new(bits.join("  ·  ")).font(font_ui(11.0)).color(p.muted));
    }
    ui.add_space(space::SECTION);
    section(ui, "CONNECTION", p);
    let before_id = c.id.clone();
    dirty |= text_field(ui, "Id", &mut c.id, p);
    if c.id.trim().is_empty() || others.contains(&c.id) {
        c.id = before_id;
    }
    dirty |= text_field(ui, "Label", &mut c.label, p);
    combo_row(ui, "Enabled", p, |ui| {
        dirty |= ui
            .checkbox(&mut c.enabled, if c.role == ModbusRole::Server { "Listen when a hub uses it" } else { "Connect and poll" })
            .changed();
    });

    combo_row(ui, "Role", p, |ui| {
        let mut role = c.role;
        egui::ComboBox::from_id_salt("pb_modbus_role")
            .selected_text(role.label())
            .width(160.0)
            .show_ui(ui, |ui| {
                for r in [ModbusRole::Client, ModbusRole::Server] {
                    ui.selectable_value(&mut role, r, r.label());
                }
            });
        if role != c.role {
            c.role = role;
            // Server defaults to the wildcard bind; flip back to loopback for clients.
            if role == ModbusRole::Server && c.host == "127.0.0.1" {
                c.host = "0.0.0.0".into();
            } else if role == ModbusRole::Client && c.host == "0.0.0.0" {
                c.host = "127.0.0.1".into();
            }
            dirty = true;
        }
    });
    combo_row(ui, "Transport", p, |ui| {
        let mut t = c.transport;
        egui::ComboBox::from_id_salt("pb_modbus_transport")
            .selected_text(t.label())
            .width(160.0)
            .show_ui(ui, |ui| {
                for v in [ModbusTransport::Tcp, ModbusTransport::Rtu] {
                    ui.selectable_value(&mut t, v, v.label());
                }
            });
        if t != c.transport {
            c.transport = t;
            dirty = true;
        }
    });

    ui.add_space(space::SECTION);
    match c.transport {
        ModbusTransport::Tcp => {
            section(
                ui,
                if c.role == ModbusRole::Server {
                    "LISTEN"
                } else {
                    "REMOTE DEVICE"
                },
                p,
            );
            dirty |= text_field(
                ui,
                if c.role == ModbusRole::Server {
                    "Bind address"
                } else {
                    "Host"
                },
                &mut c.host,
                p,
            );
            dirty |= drag_row(ui, "Port", p, |ui| {
                ui.add(egui::DragValue::new(&mut c.port).range(1..=65535))
            });
        }
        ModbusTransport::Rtu => {
            section(ui, "SERIAL LINE", p);
            dirty |= text_field(ui, "Serial port", &mut c.serial_port, p);
            dirty |= drag_row(ui, "Baud", p, |ui| {
                ui.add(egui::DragValue::new(&mut c.baud).range(300..=921_600).speed(100.0))
            });
            dirty |= drag_row(ui, "Data bits", p, |ui| {
                ui.add(egui::DragValue::new(&mut c.data_bits).range(5..=8))
            });
            combo_row(ui, "Parity", p, |ui| {
                let mut v = c.parity;
                egui::ComboBox::from_id_salt("pb_modbus_parity")
                    .selected_text(v.label())
                    .width(100.0)
                    .show_ui(ui, |ui| {
                        for x in [ModbusParity::None, ModbusParity::Even, ModbusParity::Odd] {
                            ui.selectable_value(&mut v, x, x.label());
                        }
                    });
                if v != c.parity {
                    c.parity = v;
                    dirty = true;
                }
            });
            dirty |= drag_row(ui, "Stop bits", p, |ui| {
                ui.add(egui::DragValue::new(&mut c.stop_bits).range(1..=2))
            });
        }
    }

    ui.add_space(space::SECTION);
    section(ui, "PROTOCOL", p);
    dirty |= drag_row(
        ui,
        if c.role == ModbusRole::Server {
            "Unit id"
        } else {
            "Default unit"
        },
        p,
        |ui| ui.add(egui::DragValue::new(&mut c.unit_id).range(0..=247)),
    );
    if c.role == ModbusRole::Client {
        dirty |= drag_row(ui, "Timeout (ms)", p, |ui| {
            ui.add(egui::DragValue::new(&mut c.timeout_ms).range(50..=60_000).speed(10.0))
        });
        dirty |= drag_row(ui, "Retries", p, |ui| {
            ui.add(egui::DragValue::new(&mut c.retries).range(0..=10))
        });
    }

    if c.role == ModbusRole::Server {
        ui.add_space(space::SECTION);
        section(ui, "REGISTER MAP  ·  host point served at each address", p);
        let points: Vec<String> = host.points.iter().map(|pt| pt.name.clone()).collect();
        let ctx = crate::register_map::MapCtx {
            role: crate::register_map::MapRole::Server,
            conn: "",
            host,
            points: &points,
            p,
        };
        let id = c.id.clone();
        if crate::register_map::register_map(ui, ("server_map", id), &mut c.map, &ctx) {
            for e in c.map.iter_mut() {
                e.unit = 0;
                e.point = e.point.trim().to_string();
            }
            dirty = true;
        }
    }
    let Some(c) = state.selected_mut() else {
        return dirty;
    };

    ui.add_space(space::AFTER_FIELD);
    let note = match c.role {
        ModbusRole::Client => {
            "Dead link → points go bad and the pack retries. Per-bind unit id overrides the default."
        }
        ModbusRole::Server => {
            "Masters read these points and writes land on them. Set up independently of the \
             client connections that poll external devices."
        }
    };
    ui.label(RichText::new(note).font(font_ui(11.0)).color(p.muted));
    dirty
}

pub(crate) fn section(ui: &mut egui::Ui, title: &str, p: &ThemePalette) {
    ui.label(RichText::new(title).font(font_ui(10.5)).color(p.muted));
    ui.add_space(space::AFTER_HEADING);
}

fn label_cell(ui: &mut egui::Ui, label: &str, p: &ThemePalette) {
    ui.add_sized(
        [LABEL_W, 20.0],
        egui::Label::new(RichText::new(label).font(font_ui(12.0)).color(p.muted)),
    );
}

pub(crate) fn combo_row(ui: &mut egui::Ui, label: &str, p: &ThemePalette, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::ITEM_X;
        label_cell(ui, label, p);
        add(ui);
    });
}

pub(crate) fn drag_row(
    ui: &mut egui::Ui,
    label: &str,
    p: &ThemePalette,
    add: impl FnOnce(&mut egui::Ui) -> egui::Response,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::ITEM_X;
        label_cell(ui, label, p);
        changed = add(ui).changed();
    });
    changed
}

pub(crate) fn text_field(ui: &mut egui::Ui, label: &str, value: &mut String, p: &ThemePalette) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::ITEM_X;
        label_cell(ui, label, p);
        let w = (ui.available_width() - 8.0).clamp(140.0, 420.0);
        changed = ui
            .add(
                egui::TextEdit::singleline(value)
                    .desired_width(w)
                    .margin(egui::Margin::symmetric(6.0, 3.0)),
            )
            .changed();
    });
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_filter_by_role() {
        let mut s = ModbusPanelState::default();
        s.conns.push(ModbusConn::default_server());
        assert_eq!(s.choices(ModbusRole::Client).len(), 1);
        assert_eq!(s.choices(ModbusRole::Server)[0].0, "hub");
    }

    #[test]
    fn summary_names_transport() {
        let mut c = ModbusConn::default_client();
        assert!(c.summary().starts_with("TCP client"));
        c.transport = ModbusTransport::Rtu;
        c.serial_port = "/dev/ttyUSB0".into();
        assert!(c.summary().contains("9600N1"));
    }

    #[test]
    fn legacy_bind_json_loads_with_big_endian_default() {
        let b: ModbusBind =
            serde_json::from_str(r#"{"area":"holding","address":5,"dtype":"f32"}"#).unwrap();
        assert_eq!(b.order, "abcd");
        assert_eq!(b.width(), 2);
    }

    #[test]
    fn bind_problems_cover_area_type_and_range() {
        let mut b = ModbusBind::default();
        assert!(b.problem(false).is_none());
        assert!(b.problem(true).is_some()); // needs a point
        b.dtype = "bool".into();
        assert!(b.problem(false).unwrap().contains("bool"));
        b.area = "coil".into();
        assert!(b.problem(false).is_none());
        b.dtype = "u16".into();
        assert!(b.problem(false).is_some());
        let big = ModbusBind { address: 65535, dtype: "f32".into(), ..Default::default() };
        assert!(big.problem(false).unwrap().contains("65535"));
        let order = ModbusBind { order: "zz".into(), ..Default::default() };
        assert!(order.problem(false).is_some());
    }

    #[test]
    fn server_map_detects_overlap_per_area_only() {
        let mk = |area: &str, addr: u16, dt: &str| ModbusBind {
            area: area.into(),
            address: addr,
            dtype: dt.into(),
            point: "p".into(),
            ..Default::default()
        };
        let ok = [mk("holding", 0, "f32"), mk("holding", 2, "u16"), mk("input", 0, "f32")];
        assert!(map_problems(&ok).is_empty());
        let bad = [mk("holding", 0, "f32"), mk("holding", 1, "u16")];
        assert_eq!(map_problems(&bad).len(), 1);
    }

    #[test]
    fn describe_shows_order_only_when_it_matters() {
        let mut b = ModbusBind { dtype: "f32".into(), address: 100, ..Default::default() };
        assert_eq!(b.describe(), "holding 100 f32");
        b.order = "cdab".into();
        assert_eq!(b.describe(), "holding 100 f32 cdab");
        b.area = "coil".into();
        b.dtype = "bool".into();
        assert_eq!(b.describe(), "coil 100 bool");
    }

    #[test]
    fn live_state_parses_pack_detail_json() {
        let json = r#"{"connections":[
            {"id":"plc","endpoint":"10.0.0.5:502","role":"client","state":"connected","connected":true,"error":"","polls":12,"errors":1,"clients":null},
            {"id":"hub","endpoint":"0.0.0.0:1502","role":"server","state":"listening 0.0.0.0:1502","connected":true,"error":"","polls":0,"errors":0,"clients":2}]}"#;
        let live = parse_live(json);
        assert_eq!(live.len(), 2);
        assert_eq!(live[0].polls, 12);
        assert_eq!(live[0].clients, None);
        assert_eq!(live[1].clients, Some(2));
        assert_eq!(live[1].role, "server");
        assert!(parse_live("").is_empty());
        assert!(parse_live("[]").is_empty());
    }
}
