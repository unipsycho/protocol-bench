//! Streams / TCP panel — serial + TCP client + TCP listener endpoints (locked plan).
//! Framing / parsers / emit live on hub Input / Output nodes.

use egui::{Layout, RichText, Sense, UiBuilder};

use crate::connection_list::{self, RowAction};
use crate::theme::{apply_form_spacing, font_ui, form_inset, space, ThemePalette};

const LABEL_W: f32 = 110.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamKind {
    Serial,
    TcpClient,
    TcpListener,
}

impl StreamKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Serial => "Serial",
            Self::TcpClient => "TCP client",
            Self::TcpListener => "TCP listener",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "tcp_client" | "tcp-client" | "client" => Self::TcpClient,
            "tcp_listener" | "tcp-listener" | "listener" => Self::TcpListener,
            _ => Self::Serial,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Serial => "serial",
            Self::TcpClient => "tcp_client",
            Self::TcpListener => "tcp_listener",
        }
    }
}

#[derive(Debug, Clone)]
pub struct StreamEndpoint {
    pub id: String,
    pub label: String,
    pub kind: StreamKind,
    // Serial
    pub serial_port: String,
    pub baud: String,
    pub data_bits: String,
    pub parity: String,
    pub stop_bits: String,
    pub flow: String,
    // TCP client
    pub host: String,
    pub tcp_port: String,
    pub reconnect: bool,
    // TCP listener
    pub bind: String,
    pub listen_port: String,
}

impl StreamEndpoint {
    pub fn default_serial() -> Self {
        Self {
            id: "com1".into(),
            label: "Serial".into(),
            kind: StreamKind::Serial,
            serial_port: default_serial_hint(),
            baud: "9600".into(),
            data_bits: "8".into(),
            parity: "none".into(),
            stop_bits: "1".into(),
            flow: "none".into(),
            host: "127.0.0.1".into(),
            tcp_port: "4001".into(),
            reconnect: true,
            bind: "0.0.0.0".into(),
            listen_port: "5001".into(),
        }
    }

    pub fn summary(&self) -> String {
        match self.kind {
            StreamKind::Serial => format!(
                "{}  {} {}{}{}",
                self.serial_port,
                self.baud,
                self.data_bits,
                parity_short(&self.parity),
                self.stop_bits
            ),
            StreamKind::TcpClient => format!("{}:{}", self.host, self.tcp_port),
            StreamKind::TcpListener => format!("{}:{}", self.bind, self.listen_port),
        }
    }
}

fn parity_short(p: &str) -> &'static str {
    match p.trim().to_ascii_lowercase().as_str() {
        "even" => "E",
        "odd" => "O",
        "mark" => "M",
        "space" => "S",
        _ => "N",
    }
}

fn default_serial_hint() -> String {
    if cfg!(windows) {
        "COM1".into()
    } else {
        "/dev/ttyUSB0".into()
    }
}

/// Refreshable serial device names (Linux/macOS paths; Windows COM stubs).
pub fn enumerate_serial_ports() -> Vec<String> {
    let mut out = Vec::new();
    if cfg!(windows) {
        for n in 1..=32 {
            out.push(format!("COM{n}"));
        }
        return out;
    }
    let prefixes = [
        "/dev/ttyUSB",
        "/dev/ttyACM",
        "/dev/ttyS",
        "/dev/tty.usb",
        "/dev/cu.usb",
    ];
    if let Ok(rd) = std::fs::read_dir("/dev") {
        for ent in rd.flatten() {
            let path = ent.path();
            let Some(name) = path.to_str() else { continue };
            if prefixes.iter().any(|p| name.starts_with(p)) {
                out.push(name.to_string());
            }
        }
    }
    out.sort();
    out.dedup();
    if out.is_empty() {
        out.push(default_serial_hint());
    }
    out
}

#[derive(Debug, Clone)]
pub struct StreamsPanelState {
    pub endpoints: Vec<StreamEndpoint>,
    pub selected: Option<usize>,
    pub pending_delete: Option<(usize, String)>,
    pub serial_ports: Vec<String>,
}

impl Default for StreamsPanelState {
    fn default() -> Self {
        Self {
            endpoints: vec![StreamEndpoint::default_serial()],
            selected: None,
            pending_delete: None,
            serial_ports: enumerate_serial_ports(),
        }
    }
}

impl StreamsPanelState {
    pub fn selected(&self) -> Option<&StreamEndpoint> {
        self.selected.and_then(|i| self.endpoints.get(i))
    }

    pub fn selected_mut(&mut self) -> Option<&mut StreamEndpoint> {
        self.selected.and_then(|i| self.endpoints.get_mut(i))
    }

    pub fn connection_choices(&self) -> Vec<(String, String)> {
        self.endpoints
            .iter()
            .map(|e| {
                let label = if e.label.trim().is_empty() {
                    e.id.clone()
                } else {
                    e.label.clone()
                };
                (e.id.clone(), format!("{} · {}", label, e.kind.label()))
            })
            .collect()
    }

    fn ensure_endpoints(&mut self) {
        if self.endpoints.is_empty() {
            self.endpoints.push(StreamEndpoint::default_serial());
            self.selected = None;
        } else if let Some(i) = self.selected {
            if i >= self.endpoints.len() {
                self.selected = None;
            }
        }
    }

    fn unique_id(&self, base: &str) -> String {
        let mut id = base.to_string();
        let mut n = 2;
        while self.endpoints.iter().any(|e| e.id == id) {
            id = format!("{base}-{n}");
            n += 1;
        }
        id
    }

    pub fn refresh_serial_ports(&mut self) {
        self.serial_ports = enumerate_serial_ports();
    }
}

pub fn paint(ui: &mut egui::Ui, p: &ThemePalette, state: &mut StreamsPanelState) -> bool {
    state.ensure_endpoints();
    let mut dirty = false;
    let full = ui.available_rect_before_wrap();
    let inner = form_inset(full);
    ui.allocate_new_ui(
        UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::top_down(egui::Align::Min)),
        |ui| {
            dirty = paint_body(ui, p, state);
        },
    );
    ui.allocate_rect(full, Sense::hover());
    dirty
}

fn paint_body(ui: &mut egui::Ui, p: &ThemePalette, state: &mut StreamsPanelState) -> bool {
    apply_form_spacing(ui);
    let mut dirty = false;
    ui.label(
        RichText::new(
            "Serial and TCP text streams for hub Inputs (scan) and Outputs (emit). Listeners broadcast to all peers.",
        )
        .font(font_ui(12.5))
        .color(p.muted),
    );
    ui.add_space(space::AFTER_BLURB);

    if connection_list::heading_with_add(ui, "Endpoints", "+ Add", p.live, p) {
        let id = state.unique_id("stream");
        let mut e = StreamEndpoint::default_serial();
        e.id = id.clone();
        e.label = id;
        state.endpoints.push(e);
        state.selected = Some(state.endpoints.len() - 1);
        dirty = true;
    }
    ui.add_space(space::AFTER_HEADING);
    ui.label(
        RichText::new("Transport only — framing, parsers, and point binds live on hub nodes.")
            .font(font_ui(12.0))
            .color(p.muted),
    );
    ui.add_space(space::AFTER_BLURB);

    let mut select = None;
    let show_delete = state.endpoints.len() > 1;
    for (i, ep) in state.endpoints.iter().enumerate() {
        let selected = state.selected == Some(i);
        let label = if ep.label.is_empty() {
            ep.id.clone()
        } else {
            ep.label.clone()
        };
        let subtitle = format!("{} · {}", ep.kind.label(), ep.summary());
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
    if let Some(i) = connection_list::confirm_delete(ui.ctx(), &mut state.pending_delete, p) {
        if i < state.endpoints.len() {
            let was = state.selected;
            state.endpoints.remove(i);
            state.selected = match was {
                Some(s) if s == i => None,
                Some(s) if s > i => Some(s - 1),
                other => other,
            };
            state.ensure_endpoints();
            dirty = true;
        }
    }

    ui.add_space(space::AFTER_FIELD + 2.0);
    if state.selected().is_none() {
        ui.label(
            RichText::new("Select an endpoint to edit.")
                .font(font_ui(12.5))
                .color(p.muted),
        );
        return dirty;
    }

    egui::ScrollArea::vertical()
        .id_salt("pb_streams_cfg")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            dirty |= edit_selected(ui, state, p);
        });
    dirty
}

fn edit_selected(ui: &mut egui::Ui, state: &mut StreamsPanelState, p: &ThemePalette) -> bool {
    let mut dirty = false;
    ui.label(
        RichText::new("ENDPOINT")
            .font(font_ui(10.5))
            .color(p.muted),
    );
    ui.add_space(space::AFTER_HEADING);

    // Kind first — drives which transport fields show.
    {
        let e = state.selected_mut().unwrap();
        dirty |= field(ui, "Id", &mut e.id, p);
        dirty |= field(ui, "Label", &mut e.label, p);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = space::ITEM_X;
            ui.add_sized(
                [LABEL_W, 20.0],
                egui::Label::new(RichText::new("Kind").font(font_ui(12.0)).color(p.muted)),
            );
            let mut kind = e.kind;
            egui::ComboBox::from_id_salt("pb_stream_kind")
                .selected_text(kind.label())
                .width(140.0)
                .show_ui(ui, |ui| {
                    for k in [
                        StreamKind::Serial,
                        StreamKind::TcpClient,
                        StreamKind::TcpListener,
                    ] {
                        ui.selectable_value(&mut kind, k, k.label());
                    }
                });
            if kind != e.kind {
                e.kind = kind;
                dirty = true;
            }
        });
    }

    ui.add_space(space::SECTION - 2.0);
    let kind = state.selected().unwrap().kind;
    match kind {
        StreamKind::Serial => dirty |= edit_serial(ui, state, p),
        StreamKind::TcpClient => dirty |= edit_tcp_client(ui, state, p),
        StreamKind::TcpListener => dirty |= edit_tcp_listener(ui, state, p),
    }
    dirty
}

fn edit_serial(ui: &mut egui::Ui, state: &mut StreamsPanelState, p: &ThemePalette) -> bool {
    let mut dirty = false;
    ui.label(
        RichText::new("SERIAL")
            .font(font_ui(10.5))
            .color(p.muted),
    );
    ui.add_space(space::AFTER_HEADING);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::ITEM_X;
        ui.add_sized(
            [LABEL_W, 20.0],
            egui::Label::new(RichText::new("Port").font(font_ui(12.0)).color(p.muted)),
        );
        let ports = state.serial_ports.clone();
        let e = state.selected_mut().unwrap();
        let mut port = e.serial_port.clone();
        egui::ComboBox::from_id_salt("pb_stream_serial_port")
            .selected_text(&port)
            .width(200.0)
            .show_ui(ui, |ui| {
                for pth in &ports {
                    ui.selectable_value(&mut port, pth.clone(), pth);
                }
            });
        if port != e.serial_port {
            e.serial_port = port;
            dirty = true;
        }
        if text_btn(ui, "Refresh", false, p).clicked() {
            // Defer refresh until after borrow ends.
            dirty = true;
            ui.ctx()
                .data_mut(|d| d.insert_temp(egui::Id::new("pb_streams_refresh_serial"), true));
        }
    });
    if ui
        .ctx()
        .data_mut(|d| d.remove_temp::<bool>(egui::Id::new("pb_streams_refresh_serial")))
        .unwrap_or(false)
    {
        state.refresh_serial_ports();
    }

    {
        let e = state.selected_mut().unwrap();
        // Always allow typing a path not in the list.
        dirty |= field(ui, "Port path", &mut e.serial_port, p);
        dirty |= field(ui, "Baud", &mut e.baud, p);
        dirty |= combo_str(
            ui,
            "Data bits",
            &mut e.data_bits,
            &["5", "6", "7", "8"],
            "pb_stream_databits",
            p,
        );
        dirty |= combo_str(
            ui,
            "Parity",
            &mut e.parity,
            &["none", "even", "odd", "mark", "space"],
            "pb_stream_parity",
            p,
        );
        dirty |= combo_str(
            ui,
            "Stop bits",
            &mut e.stop_bits,
            &["1", "2"],
            "pb_stream_stop",
            p,
        );
        dirty |= combo_str(
            ui,
            "Flow",
            &mut e.flow,
            &["none", "hardware", "software"],
            "pb_stream_flow",
            p,
        );
    }
    dirty
}

fn edit_tcp_client(ui: &mut egui::Ui, state: &mut StreamsPanelState, p: &ThemePalette) -> bool {
    let mut dirty = false;
    ui.label(
        RichText::new("TCP CLIENT")
            .font(font_ui(10.5))
            .color(p.muted),
    );
    ui.add_space(space::AFTER_HEADING);
    let e = state.selected_mut().unwrap();
    dirty |= field(ui, "Host", &mut e.host, p);
    dirty |= field(ui, "Port", &mut e.tcp_port, p);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::ITEM_X;
        ui.add_sized(
            [LABEL_W, 20.0],
            egui::Label::new(RichText::new("Reconnect").font(font_ui(12.0)).color(p.muted)),
        );
        if ui.checkbox(&mut e.reconnect, "").changed() {
            dirty = true;
        }
    });
    dirty
}

fn edit_tcp_listener(ui: &mut egui::Ui, state: &mut StreamsPanelState, p: &ThemePalette) -> bool {
    let mut dirty = false;
    ui.label(
        RichText::new("TCP LISTENER")
            .font(font_ui(10.5))
            .color(p.muted),
    );
    ui.add_space(space::AFTER_HEADING);
    ui.label(
        RichText::new("Output writes broadcast to every connected peer.")
            .font(font_ui(11.0))
            .color(p.muted),
    );
    ui.add_space(space::AFTER_BLURB);
    let e = state.selected_mut().unwrap();
    dirty |= field(ui, "Bind", &mut e.bind, p);
    dirty |= field(ui, "Port", &mut e.listen_port, p);
    dirty
}

fn field(ui: &mut egui::Ui, label: &str, value: &mut String, p: &ThemePalette) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::ITEM_X;
        ui.add_sized(
            [LABEL_W, 20.0],
            egui::Label::new(RichText::new(label).font(font_ui(12.0)).color(p.muted)),
        );
        let w = (ui.available_width() - 8.0).clamp(140.0, 420.0);
        if ui
            .add(
                egui::TextEdit::singleline(value)
                    .desired_width(w)
                    .margin(egui::Margin::symmetric(6.0, 3.0)),
            )
            .changed()
        {
            changed = true;
        }
    });
    changed
}

fn combo_str(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut String,
    options: &[&str],
    id: &str,
    p: &ThemePalette,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::ITEM_X;
        ui.add_sized(
            [LABEL_W, 20.0],
            egui::Label::new(RichText::new(label).font(font_ui(12.0)).color(p.muted)),
        );
        let mut next = value.clone();
        egui::ComboBox::from_id_salt(id)
            .selected_text(&next)
            .width(120.0)
            .show_ui(ui, |ui| {
                for opt in options {
                    ui.selectable_value(&mut next, (*opt).to_string(), *opt);
                }
            });
        if next != *value {
            *value = next;
            changed = true;
        }
    });
    changed
}

fn text_btn(ui: &mut egui::Ui, label: &str, selected: bool, p: &ThemePalette) -> egui::Response {
    let fill = if selected { p.accent } else { p.control };
    let text = if selected {
        egui::Color32::from_rgb(0x12, 0x14, 0x18)
    } else {
        p.text
    };
    ui.add(
        egui::Button::new(RichText::new(label).font(font_ui(12.0)).color(text))
            .fill(fill)
            .stroke(egui::Stroke::new(1.0, p.line)),
    )
}
