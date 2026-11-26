//! Ext protocols in the UI: WITS, WebSocket, Sparkplug B, DNP3 and CAN.
//!
//! These protocols share one schema-driven implementation instead of a hand-written panel each:
//! a [`UiSpec`] lists the connection fields and the Input / Output bind fields, and this module
//! renders
//!
//! * the protocol **panel** (connection list, settings form, live status and a "seen on the
//!   wire" table fed by the pack),
//! * the hub **Input / Output inspector** (connection pick, interval, bind table with live
//!   values, add form, and one-click chips for things the link has already seen),
//! * project persistence (`wits`, `websocket`, … sections of `project.json`).
//!
//! The service side is generic too (`build_ext_configure` in `pb-service`), and the wire
//! address of a bind comes from `pb_points::addr::ext_leaf`, so UI and service always agree.

use egui::{vec2, Align2, Color32, Painter, Rect, RichText, Rounding, Sense, Stroke, Ui};
use pb_engine::{EngineId, HostSnapshot};
use pb_points::addr::{ext_bind_source, ext_leaf, ExtSpec};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::connection_list::{self, RowAction};
use crate::theme::{apply_form_spacing, font_mono, font_semi, font_ui, space, tint, ThemePalette};

pub type Obj = Map<String, Value>;

// ---------------------------------------------------------------------------
// Protocols and their schemas
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExtProto {
    Wits,
    WebSocket,
    Sparkplug,
    Dnp3,
    Can,
}

#[derive(Debug, Clone, Copy)]
pub enum Def {
    S(&'static str),
    N(f64),
    B(bool),
}

#[derive(Debug, Clone, Copy)]
pub enum Kind {
    Text,
    Secret,
    /// Number with range and decimals (0 = stored as an integer).
    Num { min: f64, max: f64, dec: usize },
    Bool,
    Choice(&'static [(&'static str, &'static str)]),
}

#[derive(Debug, Clone, Copy)]
pub struct Field {
    pub key: &'static str,
    pub label: &'static str,
    pub kind: Kind,
    pub def: Def,
    pub width: f32,
    pub hint: &'static str,
    /// Only shown when another field has one of these values.
    pub show: Option<(&'static str, &'static [&'static str])>,
}

const fn f(key: &'static str, label: &'static str, kind: Kind, def: Def, width: f32, hint: &'static str) -> Field {
    Field { key, label, kind, def, width, hint, show: None }
}

const fn fs(key: &'static str, label: &'static str, kind: Kind, def: Def, width: f32, hint: &'static str, show: (&'static str, &'static [&'static str])) -> Field {
    Field { key, label, kind, def, width, hint, show: Some(show) }
}

#[allow(non_snake_case)]
const fn INT(min: f64, max: f64) -> Kind {
    Kind::Num { min, max, dec: 0 }
}
const fn num(min: f64, max: f64, dec: usize) -> Kind {
    Kind::Num { min, max, dec }
}

const TRANSPORTS: &[(&str, &str)] = &[("tcp_client", "TCP client"), ("tcp_listen", "TCP listen"), ("serial", "Serial")];
const PARITY: &[(&str, &str)] = &[("none", "None"), ("even", "Even"), ("odd", "Odd")];
const SCALE: Field = f("scale", "Scale", num(-1e9, 1e9, 6), Def::N(1.0), 70.0, "value = raw × scale + offset");
const OFFSET: Field = f("offset", "Offset", num(-1e9, 1e9, 4), Def::N(0.0), 64.0, "");

pub struct UiSpec {
    pub blurb: &'static str,
    pub conn_fields: &'static [Field],
    pub in_fields: &'static [Field],
    pub out_fields: &'static [Field],
    pub in_heading: &'static str,
    pub out_heading: &'static str,
    pub interval_in: Option<(&'static str, f64)>,
    pub interval_out: Option<(&'static str, f64)>,
    pub note: &'static str,
}

const WITS: UiSpec = UiSpec {
    blurb: "WITS level 0 record streams — the oilfield / frac data van feed. One link carries both directions over TCP or serial.",
    conn_fields: &[
        f("transport", "Transport", Kind::Choice(TRANSPORTS), Def::S("tcp_listen"), 130.0, ""),
        fs("host", "Host", Kind::Text, Def::S("0.0.0.0"), 140.0, "listen address or remote host", ("transport", &["tcp_client", "tcp_listen"])),
        fs("port", "Port", INT(1.0, 65535.0), Def::N(5100.0), 70.0, "", ("transport", &["tcp_client", "tcp_listen"])),
        fs("serial_port", "Serial port", Kind::Text, Def::S(""), 140.0, "/dev/ttyUSB0 or COM3", ("transport", &["serial"])),
        fs("baud", "Baud", INT(300.0, 921600.0), Def::N(9600.0), 80.0, "", ("transport", &["serial"])),
        fs("parity", "Parity", Kind::Choice(PARITY), Def::S("none"), 80.0, "", ("transport", &["serial"])),
        f("stale_secs", "Stale after (s)", num(0.5, 3600.0, 1), Def::N(15.0), 64.0, "no data this long → Input points go bad"),
        f("null_values", "Null values", Kind::Text, Def::S("-9999,-999.25"), 120.0, "values that mean 'absent'"),
    ],
    in_fields: &[f("item", "Item", Kind::Text, Def::S(""), 64.0, "0108 = record 01 item 08"), SCALE, OFFSET],
    out_fields: &[f("item", "Item", Kind::Text, Def::S(""), 64.0, "4-digit record + item"), f("decimals", "Decimals", INT(0.0, 9.0), Def::N(2.0), 50.0, "")],
    in_heading: "WITS ITEMS → wits_N",
    out_heading: "POINTS → WITS ITEMS (one block per interval)",
    interval_in: None,
    interval_out: Some(("Send every (s)", 1.0)),
    note: "Record 01 item names are shown for reference; frac vans often agree their own item list — any 4-digit code works.",
};

const WEBSOCKET: UiSpec = UiSpec {
    blurb: "JSON over WebSocket. Dial a ws:// / wss:// feed as a client, or listen as a server so browsers and web dashboards get live points.",
    conn_fields: &[
        f("mode", "Mode", Kind::Choice(&[("client", "Client (dial a URL)"), ("server", "Server (listen)")]), Def::S("client"), 170.0, ""),
        fs("url", "URL", Kind::Text, Def::S("ws://127.0.0.1:8765/"), 240.0, "ws:// or wss://", ("mode", &["client"])),
        fs("subscribe_message", "Subscribe message", Kind::Text, Def::S(""), 240.0, "optional text sent after connecting", ("mode", &["client"])),
        fs("host", "Listen host", Kind::Text, Def::S("0.0.0.0"), 140.0, "", ("mode", &["server"])),
        fs("port", "Port", INT(1.0, 65535.0), Def::N(8765.0), 70.0, "", ("mode", &["server"])),
        f("out_format", "Output format", Kind::Choice(&[("object", "One object, all keys"), ("change", "One message per change")]), Def::S("object"), 180.0, ""),
    ],
    in_fields: &[f("path", "JSON path", Kind::Text, Def::S(""), 160.0, "data.temp, items[0].v — empty = whole message"), SCALE, OFFSET],
    out_fields: &[f("key", "JSON key", Kind::Text, Def::S(""), 140.0, "empty = point id")],
    in_heading: "MESSAGE FIELDS → ws_N",
    out_heading: "POINTS → JSON MESSAGES",
    interval_in: None,
    interval_out: Some(("Max rate (s)", 0.5)),
    note: "A server sends every new viewer the current values straight away, then updates as points change.",
};

const SPB_TYPES: &[(&str, &str)] = &[("auto", "Auto"), ("double", "Double"), ("float", "Float"), ("int32", "Int32"), ("int64", "Int64"), ("boolean", "Boolean"), ("string", "String")];

const SPARKPLUG: UiSpec = UiSpec {
    blurb: "Sparkplug B over MQTT. A host consumes edge node births / data and can send commands; an edge node publishes points as metrics.",
    conn_fields: &[
        f("role", "Role", Kind::Choice(&[("host", "Host (SCADA / consumer)"), ("edge", "Edge node (publisher)")]), Def::S("host"), 200.0, ""),
        f("host", "Broker host", Kind::Text, Def::S("127.0.0.1"), 140.0, ""),
        f("port", "Port", INT(1.0, 65535.0), Def::N(1883.0), 70.0, ""),
        f("group_id", "Group id", Kind::Text, Def::S("Plant"), 110.0, "host: + for every group"),
        f("edge_node_id", "Edge node id", Kind::Text, Def::S(""), 120.0, "edge: this node; host: filter (empty = all)"),
        f("client_id", "Client id", Kind::Text, Def::S(""), 140.0, "empty = generated"),
        f("username", "Username", Kind::Text, Def::S(""), 110.0, ""),
        f("password", "Password", Kind::Secret, Def::S(""), 110.0, ""),
    ],
    in_fields: &[f("device", "Device", Kind::Text, Def::S(""), 90.0, "empty = node metric"), f("metric", "Metric", Kind::Text, Def::S(""), 140.0, "metric name"), SCALE, OFFSET],
    out_fields: &[
        f("edge", "Edge node", Kind::Text, Def::S(""), 90.0, "host commands only"),
        f("device", "Device", Kind::Text, Def::S(""), 90.0, "empty = node metric"),
        f("metric", "Metric", Kind::Text, Def::S(""), 140.0, ""),
        f("datatype", "Type", Kind::Choice(SPB_TYPES), Def::S("auto"), 90.0, "birth datatype (edge)"),
    ],
    in_heading: "METRICS → spb_N",
    out_heading: "POINTS → METRICS (edge: NDATA / DDATA · host: NCMD / DCMD)",
    interval_in: None,
    interval_out: Some(("Publish every (s)", 0.5)),
    note: "Aliases from BIRTH certificates are resolved automatically. A DEATH certificate turns that node's points bad.",
};

const DNP_KINDS: &[(&str, &str)] = &[
    ("analog", "Analog input"),
    ("binary", "Binary input"),
    ("counter", "Counter"),
    ("binary_output", "Binary output status"),
    ("analog_output", "Analog output status"),
];

const DNP3: UiSpec = UiSpec {
    blurb: "DNP3 master over TCP for RTUs and outstations — class 0 / 1 / 2 / 3 polls, point flags as quality, controls by direct operate.",
    conn_fields: &[
        f("host", "Outstation host", Kind::Text, Def::S("127.0.0.1"), 140.0, ""),
        f("port", "Port", INT(1.0, 65535.0), Def::N(20000.0), 70.0, ""),
        f("master_address", "Master address", INT(0.0, 65519.0), Def::N(1.0), 64.0, ""),
        f("outstation_address", "Outstation address", INT(0.0, 65519.0), Def::N(10.0), 64.0, ""),
        f("timeout_ms", "Timeout (ms)", INT(100.0, 60000.0), Def::N(3000.0), 70.0, ""),
    ],
    in_fields: &[f("kind", "Type", Kind::Choice(DNP_KINDS), Def::S("analog"), 160.0, ""), f("index", "Index", INT(0.0, 65535.0), Def::N(0.0), 60.0, ""), SCALE, OFFSET],
    out_fields: &[
        f("kind", "Control", Kind::Choice(&[("binary_output", "CROB (latch on / off)"), ("analog_output", "Analog output")]), Def::S("binary_output"), 170.0, ""),
        f("index", "Index", INT(0.0, 65535.0), Def::N(0.0), 60.0, ""),
        f("float", "Float (g41v3)", Kind::Bool, Def::B(true), 110.0, "off = 32-bit integer (g41v1)"),
        SCALE,
        OFFSET,
    ],
    in_heading: "DNP3 POINTS → dnp3_N",
    out_heading: "POINTS → CONTROLS (direct operate)",
    interval_in: Some(("Poll (s)", 2.0)),
    interval_out: None,
    note: "Each poll reads class 0 (static) plus class 1 / 2 / 3 events. DEVICE RESTART is cleared automatically.",
};

const CAN_ORDER: &[(&str, &str)] = &[("intel", "Intel (little)"), ("motorola", "Motorola (big)")];
const CAN_TYPE: &[(&str, &str)] = &[("unsigned", "Unsigned"), ("signed", "Signed"), ("float", "Float32")];

const CAN: UiSpec = UiSpec {
    blurb: "CAN bus over SocketCAN (Linux): decode signals from frames — including J1939 parameter groups — and transmit points as frames.",
    conn_fields: &[
        f("interface", "Interface", Kind::Text, Def::S("vcan0"), 100.0, "can0, vcan0"),
        f("stale_secs", "Stale after (s)", num(0.2, 3600.0, 1), Def::N(5.0), 64.0, "no frame this long → point bad"),
    ],
    in_fields: &[
        f("id", "Frame id", Kind::Text, Def::S(""), 110.0, "0x123, 0x18FEF100 or pgn:61444"),
        f("start", "Start bit", INT(0.0, 63.0), Def::N(0.0), 54.0, ""),
        f("len", "Bits", INT(1.0, 64.0), Def::N(8.0), 50.0, ""),
        f("order", "Order", Kind::Choice(CAN_ORDER), Def::S("intel"), 120.0, ""),
        f("type", "Type", Kind::Choice(CAN_TYPE), Def::S("unsigned"), 96.0, ""),
        SCALE,
        OFFSET,
    ],
    out_fields: &[
        f("id", "Frame id", Kind::Text, Def::S(""), 110.0, "0x123 or 0x18FF0001"),
        f("start", "Start bit", INT(0.0, 63.0), Def::N(0.0), 54.0, ""),
        f("len", "Bits", INT(1.0, 64.0), Def::N(8.0), 50.0, ""),
        f("order", "Order", Kind::Choice(CAN_ORDER), Def::S("intel"), 120.0, ""),
        f("type", "Type", Kind::Choice(CAN_TYPE), Def::S("unsigned"), 96.0, ""),
        SCALE,
        OFFSET,
        f("dlc", "DLC", INT(0.0, 8.0), Def::N(8.0), 44.0, ""),
    ],
    in_heading: "SIGNALS → can_N",
    out_heading: "POINTS → FRAMES",
    interval_in: None,
    interval_out: Some(("Transmit every (s)", 0.1)),
    note: "Test without hardware: sudo ip link add dev vcan0 type vcan && sudo ip link set up vcan0.",
};

impl ExtProto {
    pub const ALL: [ExtProto; 5] = [ExtProto::Wits, ExtProto::WebSocket, ExtProto::Sparkplug, ExtProto::Dnp3, ExtProto::Can];

    pub fn id(self) -> &'static str {
        match self {
            Self::Wits => "wits",
            Self::WebSocket => "websocket",
            Self::Sparkplug => "sparkplug",
            Self::Dnp3 => "dnp3",
            Self::Can => "can",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Wits => "WITS",
            Self::WebSocket => "WebSocket",
            Self::Sparkplug => "Sparkplug B",
            Self::Dnp3 => "DNP3",
            Self::Can => "CAN bus",
        }
    }

    pub fn core(self) -> &'static ExtSpec {
        pb_points::addr::ext_spec(self.id()).expect("ext spec registered in pb-points")
    }

    pub fn engine(self) -> EngineId {
        EngineId::from_ext(self.id()).expect("ext engine id")
    }

    pub fn spec(self) -> &'static UiSpec {
        match self {
            Self::Wits => &WITS,
            Self::WebSocket => &WEBSOCKET,
            Self::Sparkplug => &SPARKPLUG,
            Self::Dnp3 => &DNP3,
            Self::Can => &CAN,
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|p| *p == self).unwrap_or(0)
    }

    /// Bright ink for dark displays.
    pub fn ink_core(self) -> Color32 {
        match self {
            Self::Wits => Color32::from_rgb(0xff, 0x8f, 0x5a),
            Self::WebSocket => Color32::from_rgb(0x6c, 0xd4, 0xff),
            Self::Sparkplug => Color32::from_rgb(0xf2, 0x72, 0xb6),
            Self::Dnp3 => Color32::from_rgb(0xb8, 0xe0, 0x5c),
            Self::Can => Color32::from_rgb(0xff, 0xd1, 0x66),
        }
    }

    /// Ink that reads on light panes.
    pub fn ink_deep(self) -> Color32 {
        match self {
            Self::Wits => Color32::from_rgb(0xa3, 0x40, 0x12),
            Self::WebSocket => Color32::from_rgb(0x0b, 0x6a, 0x96),
            Self::Sparkplug => Color32::from_rgb(0x9a, 0x1f, 0x60),
            Self::Dnp3 => Color32::from_rgb(0x4a, 0x6e, 0x10),
            Self::Can => Color32::from_rgb(0x8a, 0x62, 0x00),
        }
    }

}

// ---------------------------------------------------------------------------
// Field values on JSON objects
// ---------------------------------------------------------------------------

pub fn def_value(fd: &Field) -> Value {
    match (fd.def, fd.kind) {
        (Def::S(s), _) => Value::from(s),
        (Def::N(n), Kind::Num { dec: 0, .. }) => Value::from(n as i64),
        (Def::N(n), _) => Value::from(n),
        (Def::B(b), _) => Value::from(b),
    }
}

pub fn text_of(o: &Obj, key: &str) -> String {
    match o.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

fn num_of(o: &Obj, fd: &Field) -> f64 {
    match o.get(fd.key) {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(Value::String(s)) => s.trim().parse().unwrap_or(match fd.def {
            Def::N(n) => n,
            _ => 0.0,
        }),
        _ => match fd.def {
            Def::N(n) => n,
            _ => 0.0,
        },
    }
}

fn visible(o: &Obj, fd: &Field, fields: &[Field]) -> bool {
    match fd.show {
        None => true,
        Some((k, vals)) => {
            let cur = match o.get(k) {
                Some(_) => text_of(o, k),
                None => fields.iter().find(|x| x.key == k).map(|x| text_of(&obj_with_defaults(&[*x]), k)).unwrap_or_default(),
            };
            vals.contains(&cur.as_str())
        }
    }
}

/// A fresh object with every field's default.
pub fn obj_with_defaults(fields: &[Field]) -> Obj {
    let mut o = Obj::new();
    for fd in fields {
        o.insert(fd.key.into(), def_value(fd));
    }
    o
}

/// One field editor of width `w`. Returns true when the value changed.
fn field_widget(ui: &mut Ui, salt: egui::Id, fd: &Field, o: &mut Obj, w: f32) -> bool {
    let h = crate::form::ROW_H - 2.0;
    let resp_changed = match fd.kind {
        Kind::Text | Kind::Secret => {
            let mut s = text_of(o, fd.key);
            let r = ui.add_sized([w, h], egui::TextEdit::singleline(&mut s).password(matches!(fd.kind, Kind::Secret)).hint_text(fd.hint));
            if r.changed() {
                o.insert(fd.key.into(), Value::from(s));
                true
            } else {
                false
            }
        }
        Kind::Num { min, max, dec } => {
            let mut x = num_of(o, fd);
            let speed = if dec == 0 { 1.0 } else { 10f64.powi(-(dec.min(3) as i32)) };
            let r = ui.add_sized([w, h], egui::DragValue::new(&mut x).range(min..=max).speed(speed).max_decimals(dec));
            let r = if fd.hint.is_empty() { r } else { r.on_hover_text(fd.hint) };
            if r.changed() {
                o.insert(fd.key.into(), if dec == 0 { Value::from(x.round() as i64) } else { Value::from(x) });
                true
            } else {
                false
            }
        }
        Kind::Bool => {
            let mut b = match o.get(fd.key) {
                Some(Value::Bool(b)) => *b,
                _ => matches!(fd.def, Def::B(true)),
            };
            let r = ui.checkbox(&mut b, "");
            if r.changed() {
                o.insert(fd.key.into(), Value::from(b));
                true
            } else {
                false
            }
        }
        Kind::Choice(opts) => {
            let mut cur = text_of(o, fd.key);
            if cur.is_empty() {
                if let Def::S(d) = fd.def {
                    cur = d.to_string();
                }
            }
            crate::form::combo(ui, salt.with(fd.key), w, &mut cur, opts) && {
                o.insert(fd.key.into(), Value::from(cur));
                true
            }
        }
    };
    resp_changed
}

/// Wrapping form over `fields` (hidden ones skipped).
pub fn fields_form(ui: &mut Ui, salt: egui::Id, fields: &'static [Field], o: &mut Obj, p: &ThemePalette) -> bool {
    let shown: Vec<&'static Field> = fields.iter().filter(|fd| visible(o, fd, fields)).collect();
    let cells: Vec<crate::form::Cell<Obj>> = shown
        .into_iter()
        .map(|fd| {
            let cell = move |ui: &mut Ui, w: f32, o: &mut Obj| field_widget(ui, salt, fd, o, w);
            if matches!(fd.kind, Kind::Text | Kind::Secret) && fd.width >= 120.0 {
                crate::form::Cell::grow(fd.label, fd.width, cell)
            } else {
                crate::form::Cell::fixed(fd.label, fd.width, cell)
            }
        })
        .collect();
    crate::form::flow(ui, o, p, cells)
}

// ---------------------------------------------------------------------------
// Live state from the pack
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SeenItem {
    pub leaf: Obj,
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LiveLink {
    pub id: String,
    pub endpoint: String,
    pub state: String,
    pub connected: bool,
    pub error: String,
    pub rx: u64,
    pub tx: u64,
    pub errors: u64,
    pub peers: Option<u64>,
    pub seen: Vec<SeenItem>,
}

pub fn parse_live(detail: &str) -> Vec<LiveLink> {
    let Ok(v) = serde_json::from_str::<Value>(detail) else { return Vec::new() };
    v.get("connections")
        .and_then(|c| c.as_array())
        .into_iter()
        .flatten()
        .map(|c| {
            let s = |k: &str| c.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
            let n = |k: &str| c.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
            LiveLink {
                id: s("id"),
                endpoint: s("endpoint"),
                state: s("state"),
                connected: c.get("connected").and_then(|x| x.as_bool()).unwrap_or(false),
                error: s("error"),
                rx: n("rx"),
                tx: n("tx"),
                errors: n("errors"),
                peers: c.get("peers").and_then(|x| x.as_u64()),
                seen: c
                    .get("seen")
                    .and_then(|x| x.as_array())
                    .into_iter()
                    .flatten()
                    .map(|x| SeenItem {
                        leaf: x.get("leaf").and_then(|l| l.as_object()).cloned().unwrap_or_default(),
                        label: x.get("label").and_then(|l| l.as_str()).unwrap_or("").to_string(),
                        value: x.get("value").and_then(|l| l.as_str()).unwrap_or("").to_string(),
                    })
                    .collect(),
            }
        })
        .collect()
}

pub fn live_for(host: &HostSnapshot, proto: ExtProto) -> (Vec<LiveLink>, bool) {
    match host.slot(proto.engine()) {
        Some(slot) => (parse_live(&slot.detail_json), slot.pack_loaded),
        None => (Vec::new(), false),
    }
}

fn status_dot(link: Option<&LiveLink>, pack_loaded: bool, enabled: bool, p: &ThemePalette) -> (Color32, String) {
    if !pack_loaded {
        return (p.warn, "pack not loaded".into());
    }
    if !enabled {
        return (p.muted, "disabled".into());
    }
    match link {
        None => (p.muted, "waiting for the service".into()),
        Some(l) if l.connected => (p.live, l.state.clone()),
        Some(l) if l.state == "error" => (p.err, "error".into()),
        Some(l) => (p.warn, l.state.clone()),
    }
}

// ---------------------------------------------------------------------------
// Panel state + persistence
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExtSection {
    #[serde(default)]
    pub connections: Vec<Obj>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub selected_id: String,
}

/// The ext sections of `project.json` (flattened into the project file).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExtSections {
    #[serde(default)]
    pub wits: ExtSection,
    #[serde(default)]
    pub websocket: ExtSection,
    #[serde(default)]
    pub sparkplug: ExtSection,
    #[serde(default)]
    pub dnp3: ExtSection,
    #[serde(default)]
    pub can: ExtSection,
}

impl ExtSections {
    pub fn get(&self, p: ExtProto) -> &ExtSection {
        match p {
            ExtProto::Wits => &self.wits,
            ExtProto::WebSocket => &self.websocket,
            ExtProto::Sparkplug => &self.sparkplug,
            ExtProto::Dnp3 => &self.dnp3,
            ExtProto::Can => &self.can,
        }
    }
    pub fn get_mut(&mut self, p: ExtProto) -> &mut ExtSection {
        match p {
            ExtProto::Wits => &mut self.wits,
            ExtProto::WebSocket => &mut self.websocket,
            ExtProto::Sparkplug => &mut self.sparkplug,
            ExtProto::Dnp3 => &mut self.dnp3,
            ExtProto::Can => &mut self.can,
        }
    }
    pub fn from_panels(panels: &ExtPanels) -> Self {
        let mut s = Self::default();
        for st in &panels.states {
            let sec = s.get_mut(st.proto);
            sec.connections = st.conns.clone();
            sec.selected_id = st.selected.and_then(|i| st.conns.get(i)).map(|c| text_of(c, "id")).unwrap_or_default();
        }
        s
    }
    pub fn apply_to_panels(&self, panels: &mut ExtPanels) {
        for st in &mut panels.states {
            let sec = self.get(st.proto);
            st.conns = sec.connections.clone();
            st.selected = st.conns.iter().position(|c| text_of(c, "id") == sec.selected_id);
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExtPanelState {
    pub proto: ExtProto,
    pub conns: Vec<Obj>,
    pub selected: Option<usize>,
    pub pending_delete: Option<(usize, String)>,
    pub seen_filter: String,
}

impl ExtPanelState {
    pub fn new(proto: ExtProto) -> Self {
        Self { proto, conns: Vec::new(), selected: None, pending_delete: None, seen_filter: String::new() }
    }

    /// `(id, label)` choices for hub inspectors.
    pub fn choices(&self) -> Vec<(String, String)> {
        self.conns
            .iter()
            .map(|c| {
                let id = text_of(c, "id");
                let label = text_of(c, "label");
                (id.clone(), if label.trim().is_empty() { id } else { label })
            })
            .collect()
    }

    pub fn new_conn(&self) -> Obj {
        let mut o = obj_with_defaults(self.proto.spec().conn_fields);
        let id = connection_list::unique_id(self.proto.id(), |x| self.conns.iter().any(|c| text_of(c, "id") == x));
        o.insert("id".into(), Value::from(id.clone()));
        o.insert("label".into(), Value::from(format!("{} {}", self.proto.title(), self.conns.len() + 1)));
        o.insert("enabled".into(), Value::from(true));
        o
    }
}

#[derive(Debug, Clone)]
pub struct ExtPanels {
    pub states: Vec<ExtPanelState>,
}

impl Default for ExtPanels {
    fn default() -> Self {
        Self { states: ExtProto::ALL.iter().map(|p| ExtPanelState::new(*p)).collect() }
    }
}

impl ExtPanels {
    #[allow(dead_code)]
    pub fn get(&self, p: ExtProto) -> &ExtPanelState {
        &self.states[p.index()]
    }
    pub fn get_mut(&mut self, p: ExtProto) -> &mut ExtPanelState {
        &mut self.states[p.index()]
    }
}

// ---------------------------------------------------------------------------
// Panel
// ---------------------------------------------------------------------------

fn conn_summary(proto: ExtProto, c: &Obj) -> String {
    let s = |k: &str| text_of(c, k);
    match proto {
        ExtProto::Wits => match s("transport").as_str() {
            "serial" => format!("serial {} @ {}", s("serial_port"), s("baud")),
            "tcp_client" => format!("TCP → {}:{}", s("host"), s("port")),
            _ => format!("listen {}:{}", s("host"), s("port")),
        },
        ExtProto::WebSocket => {
            if s("mode") == "server" {
                format!("server ws://{}:{}", s("host"), s("port"))
            } else {
                s("url")
            }
        }
        ExtProto::Sparkplug => format!("{} · {}:{} · {}{}", s("role"), s("host"), s("port"), s("group_id"), if s("edge_node_id").is_empty() { String::new() } else { format!("/{}", s("edge_node_id")) }),
        ExtProto::Dnp3 => format!("{}:{} · {} → {}", s("host"), s("port"), s("master_address"), s("outstation_address")),
        ExtProto::Can => format!("SocketCAN {}", s("interface")),
    }
}

fn section(ui: &mut Ui, title: &str, p: &ThemePalette) {
    ui.label(RichText::new(title).font(font_ui(10.5)).color(p.muted));
    ui.add_space(space::AFTER_HEADING);
}

/// The protocol panel. Returns true when connections changed.
pub fn paint_panel(ui: &mut Ui, p: &ThemePalette, st: &mut ExtPanelState, host: &HostSnapshot) -> bool {
    let full = ui.available_rect_before_wrap();
    let inner = crate::theme::form_inset(full);
    let mut dirty = false;
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(inner), |ui| {
        egui::ScrollArea::vertical().id_salt(("pb_ext_panel", st.proto.id())).auto_shrink([false, false]).show(ui, |ui| {
            dirty = paint_panel_body(ui, p, st, host);
        });
    });
    dirty
}

fn paint_panel_body(ui: &mut Ui, p: &ThemePalette, st: &mut ExtPanelState, host: &HostSnapshot) -> bool {
    apply_form_spacing(ui);
    let proto = st.proto;
    let spec = proto.spec();
    let light = p.is_light;
    let ink = if light { proto.ink_deep() } else { proto.ink_core() };
    let (live, pack_loaded) = live_for(host, proto);
    let mut dirty = false;
    // Header banner.
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, 54.0), Sense::hover());
    ui.painter().rect_filled(rect, Rounding::same(8.0), tint(ink, 22));
    ui.painter().rect_stroke(rect, Rounding::same(8.0), Stroke::new(1.0, tint(ink, 90)));
    let icon = Rect::from_center_size(egui::pos2(rect.min.x + 28.0, rect.center().y), vec2(30.0, 30.0));
    paint_icon(ui.painter(), proto, icon, Stroke::new(1.6, ink), ink);
    ui.painter().text(egui::pos2(rect.min.x + 54.0, rect.min.y + 10.0), Align2::LEFT_TOP, proto.title(), font_semi(15.0), p.text);
    let up = live.iter().filter(|l| l.connected).count();
    let summary = if !pack_loaded {
        format!("pack not loaded — add libpb_{} to the service modules/ folder", proto.id())
    } else {
        format!("{} connection{} · {up} up", st.conns.len(), if st.conns.len() == 1 { "" } else { "s" })
    };
    ui.painter().text(egui::pos2(rect.min.x + 54.0, rect.min.y + 31.0), Align2::LEFT_TOP, summary, font_ui(11.5), if pack_loaded { p.muted } else { p.warn });
    ui.add_space(6.0);
    ui.label(RichText::new(spec.blurb).font(font_ui(12.5)).color(p.muted));
    ui.add_space(space::AFTER_BLURB);

    if connection_list::heading_with_add(ui, "Connections", "+ Add", p.live, p) {
        let c = st.new_conn();
        st.conns.push(c);
        st.selected = Some(st.conns.len() - 1);
        dirty = true;
    }
    ui.add_space(space::AFTER_HEADING);
    if st.conns.is_empty() {
        ui.label(RichText::new("No connections yet — + Add one, then bind it on a hub Input or Output node.").font(font_ui(12.0)).color(p.muted));
    }
    let mut select = None;
    for (i, c) in st.conns.iter().enumerate() {
        let id = text_of(c, "id");
        let label = text_of(c, "label");
        let label = if label.trim().is_empty() { id.clone() } else { label };
        let enabled = !matches!(c.get("enabled"), Some(Value::Bool(false)));
        let (_, state) = status_dot(live.iter().find(|l| l.id == id), pack_loaded, enabled, p);
        let summary = conn_summary(proto, c);
        // Pack states often start with the endpoint already; don't say it twice.
        let ep_tail = summary.split_whitespace().last().unwrap_or("").to_string();
        let subtitle = if !ep_tail.is_empty() && state.contains(&ep_tail) { state.clone() } else { format!("{summary}  ·  {state}") };
        match connection_list::selectable_row(ui, &label, &subtitle, st.selected == Some(i), true, p) {
            Some(RowAction::Select) => select = Some(i),
            Some(RowAction::Delete) => st.pending_delete = Some((i, label)),
            None => {}
        }
        ui.add_space(2.0);
    }
    if let Some(i) = select {
        st.selected = if st.selected == Some(i) { None } else { Some(i) };
    }
    if let Some(i) = connection_list::confirm_delete(ui.ctx(), &mut st.pending_delete, p) {
        if i < st.conns.len() {
            st.conns.remove(i);
            st.selected = None;
            dirty = true;
        }
    }
    let Some(i) = st.selected.filter(|i| *i < st.conns.len()) else {
        if !st.conns.is_empty() {
            ui.add_space(space::AFTER_FIELD);
            ui.label(RichText::new("Select a connection to edit.").font(font_ui(12.5)).color(p.muted));
        }
        return dirty;
    };
    let others: Vec<String> = st.conns.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, c)| text_of(c, "id")).collect();
    let c = &mut st.conns[i];
    let id = text_of(c, "id");
    let link = live.iter().find(|l| l.id == id).cloned();
    ui.add_space(space::SECTION);
    section(ui, "STATUS", p);
    let enabled = !matches!(c.get("enabled"), Some(Value::Bool(false)));
    let (dot, state) = status_dot(link.as_ref(), pack_loaded, enabled, p);
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("●").font(font_ui(12.0)).color(dot));
        ui.label(RichText::new(state).font(font_ui(12.0)).color(p.text));
    });
    if let Some(l) = &link {
        if !l.error.is_empty() {
            ui.label(RichText::new(&l.error).font(font_ui(11.5)).color(p.warn));
        }
        let mut bits = vec![format!("{} received", l.rx), format!("{} sent", l.tx), format!("{} errors", l.errors)];
        if let Some(n) = l.peers {
            bits.push(format!("{n} peer{}", if n == 1 { "" } else { "s" }));
        }
        ui.label(RichText::new(bits.join("  ·  ")).font(font_ui(11.0)).color(p.muted));
    }
    ui.add_space(space::SECTION);
    section(ui, "CONNECTION", p);
    let salt = egui::Id::new(("pb_ext_conn", proto.id(), i));
    let mut idv = text_of(c, "id");
    let mut label = text_of(c, "label");
    let mut on = enabled;
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("Id").font(font_ui(12.0)).color(p.muted));
        changed |= ui.add(egui::TextEdit::singleline(&mut idv).desired_width(110.0)).changed();
        ui.label(RichText::new("Label").font(font_ui(12.0)).color(p.muted));
        changed |= ui.add(egui::TextEdit::singleline(&mut label).desired_width(150.0)).changed();
        changed |= ui.checkbox(&mut on, "Enabled").changed();
    });
    if changed {
        let idv = idv.trim().to_string();
        if !idv.is_empty() && !others.contains(&idv) && !idv.contains(['/', ':']) {
            c.insert("id".into(), Value::from(idv));
        }
        c.insert("label".into(), Value::from(label));
        c.insert("enabled".into(), Value::from(on));
        dirty = true;
    }
    ui.add_space(4.0);
    dirty |= fields_form(ui, salt, spec.conn_fields, c, p);
    ui.add_space(space::AFTER_FIELD);
    ui.label(RichText::new(spec.note).font(font_ui(11.0)).color(p.muted));

    // Seen on the wire.
    if let Some(l) = link.filter(|l| !l.seen.is_empty()) {
        ui.add_space(space::SECTION);
        section(ui, &format!("SEEN ON THE WIRE  ·  {}", l.seen.len()), p);
        if l.seen.len() > 10 {
            ui.add(egui::TextEdit::singleline(&mut st.seen_filter).hint_text("filter").desired_width(180.0));
        }
        let flt = st.seen_filter.to_ascii_lowercase();
        let w = ui.available_width();
        for (k, s) in l.seen.iter().filter(|s| flt.is_empty() || s.label.to_ascii_lowercase().contains(&flt)).take(200).enumerate() {
            let (r, _) = ui.allocate_exact_size(vec2(w, 20.0), Sense::hover());
            if k % 2 == 1 {
                ui.painter().rect_filled(r, Rounding::same(3.0), tint(p.line, 40));
            }
            ui.painter().text(r.left_center() + vec2(6.0, 0.0), Align2::LEFT_CENTER, &s.label, font_ui(12.0), p.text);
            ui.painter().text(r.right_center() - vec2(6.0, 0.0), Align2::RIGHT_CENTER, s.value.chars().take(32).collect::<String>(), font_mono(11.5), ink);
        }
    }
    dirty
}

// ---------------------------------------------------------------------------
// Hub port inspector
// ---------------------------------------------------------------------------

/// Bind summary: the leaf fields (`0108`, `Robot1 · Torque`, `analog 3`).
pub fn bind_label(proto: ExtProto, b: &Obj) -> String {
    let parts: Vec<String> = proto.core().leaf_keys.iter().map(|k| text_of(b, k)).filter(|s| !s.trim().is_empty()).collect();
    let mut s = parts.join(" · ");
    if proto == ExtProto::Wits {
        let code = text_of(b, "item");
        let name = pb_proto_item_name(&code);
        if !name.is_empty() {
            s = format!("{code} {name}");
        }
    }
    if s.is_empty() {
        "(whole message)".into()
    } else {
        s
    }
}

/// WITS record-1 names (copy of `pb_proto::wits::item_name` keys so the UI needs no pack crate).
fn pb_proto_item_name(code: &str) -> &'static str {
    match format!("{:0>4}", code.trim()).as_str() {
        "0108" => "Depth bit",
        "0110" => "Depth hole",
        "0112" => "Block position",
        "0113" => "ROP",
        "0114" => "Hookload",
        "0116" => "WOB",
        "0118" => "Torque",
        "0120" => "RPM",
        "0121" => "Standpipe pressure",
        "0122" => "Casing pressure",
        "0123" => "SPM 1",
        "0126" => "Tank volume",
        "0130" => "Flow in",
        _ => "",
    }
}

/// Wire address for an Input bind on `conn` (same as the service builds).
pub fn bind_source(proto: ExtProto, conn: &str, b: &Obj) -> Option<String> {
    let spec = proto.core();
    ext_leaf(spec, |k| text_of(b, k)).map(|leaf| ext_bind_source(spec, conn, &leaf))
}

fn live_value<'a>(host: &'a HostSnapshot, point: &str, source: Option<&str>) -> Option<&'a pb_engine::PointSnapshot> {
    let pt = point.trim();
    if !pt.is_empty() {
        if let Some(x) = host.points.iter().find(|p| p.name == pt) {
            return Some(x);
        }
    }
    source.and_then(|s| host.points.iter().find(|p| p.source == s))
}

#[derive(Clone, Default)]
struct PortDraft {
    obj: Option<Obj>,
    point: String,
    editing: Option<usize>,
}

/// Fields of a hub port that the ext protocols use.
pub struct PortFields<'a> {
    pub conn: &'a mut String,
    pub interval: &'a mut f32,
    pub inputs: &'a mut Vec<Obj>,
    pub outputs: &'a mut Vec<Obj>,
}

/// Hub Input / Output inspector for an ext protocol port. Returns true when it changed.
#[allow(clippy::too_many_arguments)]
pub fn edit_port(
    ui: &mut Ui,
    proto: ExtProto,
    port_key: (u64, u64),
    incoming: bool,
    port: PortFields<'_>,
    conns: &[(String, String)],
    host: &HostSnapshot,
    points: &[String],
    p: &ThemePalette,
) -> bool {
    apply_form_spacing(ui);
    let spec = proto.spec();
    let ink = if p.is_light { proto.ink_deep() } else { proto.ink_core() };
    let mut dirty = false;
    section(ui, &format!("{} CONNECTION", proto.title().to_ascii_uppercase()), p);
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("Connection").font(font_ui(12.0)).color(p.muted));
        let shown = conns.iter().find(|(id, _)| id == port.conn).map(|(_, l)| l.clone()).unwrap_or_else(|| if port.conn.is_empty() { "(pick connection)".into() } else { port.conn.clone() });
        let mut next = port.conn.clone();
        egui::ComboBox::from_id_salt(("pb_ext_port_conn", proto.id(), port_key)).selected_text(shown).width(180.0).show_ui(ui, |ui| {
            ui.selectable_value(&mut next, String::new(), "(none)");
            for (id, l) in conns {
                ui.selectable_value(&mut next, id.clone(), l);
            }
        });
        if next != *port.conn {
            *port.conn = next;
            dirty = true;
        }
        let interval = if incoming { spec.interval_in } else { spec.interval_out };
        if let Some((label, _)) = interval {
            ui.label(RichText::new(label).font(font_ui(12.0)).color(p.muted));
            dirty |= ui.add(egui::DragValue::new(port.interval).range(0.02..=3600.0).speed(0.05).max_decimals(2)).changed();
        }
    });
    let (live, pack_loaded) = live_for(host, proto);
    let link = live.iter().find(|l| l.id == *port.conn);
    if !port.conn.is_empty() {
        let (dot, state) = status_dot(link, pack_loaded, true, p);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("●").font(font_ui(12.0)).color(dot));
            ui.label(RichText::new(state).font(font_ui(12.0)).color(p.text));
        });
        if let Some(l) = link.filter(|l| !l.error.is_empty()) {
            ui.label(RichText::new(&l.error).font(font_ui(11.0)).color(p.warn));
        }
    }
    if conns.is_empty() {
        ui.label(RichText::new(format!("Add a connection in the {} panel.", proto.title())).font(font_ui(11.0)).color(p.muted));
    }
    ui.add_space(space::SECTION - 2.0);
    section(ui, if incoming { spec.in_heading } else { spec.out_heading }, p);

    let fields = if incoming { spec.in_fields } else { spec.out_fields };
    let draft_id = egui::Id::new(("pb_ext_draft", proto.id(), port_key, incoming));
    let mut draft: PortDraft = ui.data_mut(|d| d.get_temp(draft_id)).unwrap_or_default();
    let list = if incoming { port.inputs } else { port.outputs };
    // Bind rows.
    let w = ui.available_width();
    let mut remove = None;
    let mut edit = None;
    for (i, b) in list.iter().enumerate() {
        let point = text_of(b, "point");
        let source = if incoming { bind_source(proto, port.conn, b) } else { None };
        let lv = live_value(host, &point, source.as_deref());
        let (r, resp) = ui.allocate_exact_size(vec2(w, 26.0), Sense::click());
        let sel = draft.editing == Some(i);
        let fill = if sel { tint(ink, 40) } else if resp.hovered() { tint(ink, 18) } else if i % 2 == 1 { tint(p.line, 40) } else { Color32::TRANSPARENT };
        ui.painter().rect_filled(r, Rounding::same(4.0), fill);
        let q = lv.map(|x| match x.quality.as_str() {
            "good" => p.live,
            "bad" => p.err,
            _ => p.warn,
        });
        ui.painter().circle_filled(egui::pos2(r.min.x + 9.0, r.center().y), 3.5, q.unwrap_or(tint(p.muted, 120)));
        let shown_point = if point.trim().is_empty() {
            lv.map(|x| x.name.clone()).unwrap_or_else(|| if incoming { format!("{}_N", proto.core().prefix) } else { "pick point".into() })
        } else {
            point.clone()
        };
        let left = if incoming { format!("{}  →  {shown_point}", bind_label(proto, b)) } else { format!("{shown_point}  →  {}", bind_label(proto, b)) };
        let val = lv.map(|x| x.value.chars().take(18).collect::<String>()).unwrap_or_else(|| "—".into());
        let vr = ui.painter().text(egui::pos2(r.max.x - 30.0, r.center().y), Align2::RIGHT_CENTER, val, font_mono(11.5), if lv.is_some() { ink } else { p.muted });
        let room = vr.min.x - (r.min.x + 18.0) - 10.0;
        let left = crate::register_map::clip_text(ui, &left, font_ui(12.5), room);
        ui.painter().text(egui::pos2(r.min.x + 18.0, r.center().y), Align2::LEFT_CENTER, left, font_ui(12.5), p.text);
        let x = Rect::from_center_size(egui::pos2(r.max.x - 12.0, r.center().y), vec2(20.0, 20.0));
        let xr = ui.interact(x, draft_id.with(("x", i)), Sense::click()).on_hover_text("Remove");
        ui.painter().text(x.center(), Align2::CENTER_CENTER, "×", font_ui(14.0), if xr.hovered() { p.err } else { p.muted });
        if xr.clicked() {
            remove = Some(i);
        } else if resp.clicked() {
            edit = Some(i);
        }
    }
    if let Some(i) = remove {
        list.remove(i);
        draft.editing = None;
        dirty = true;
    }
    if let Some(i) = edit {
        if draft.editing == Some(i) {
            draft.editing = None;
            draft.obj = None;
        } else {
            draft.editing = Some(i);
            draft.obj = Some(list[i].clone());
            draft.point = text_of(&list[i], "point");
        }
    }
    if list.is_empty() {
        ui.label(RichText::new(if incoming { "No binds yet — add one below or click something the link has seen." } else { "No binds yet — pick a point and where it goes." }).font(font_ui(11.5)).color(p.muted));
    }

    // Seen chips.
    if let Some(l) = link.filter(|l| !l.seen.is_empty()) {
        ui.add_space(6.0);
        ui.label(RichText::new(if incoming { "SEEN ON THE WIRE — click to bind" } else { "SEEN ON THE WIRE — click to target" }).font(font_ui(10.0)).color(p.muted));
        let taken = |s: &SeenItem| list.iter().any(|b| s.leaf.iter().all(|(k, v)| text_of(b, k) == match v {
            Value::String(x) => x.clone(),
            other => other.to_string(),
        }));
        let mut pick: Option<Obj> = None;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(5.0, 4.0);
            for s in l.seen.iter().filter(|s| !s.leaf.is_empty()).take(60) {
                let used = taken(s);
                let txt = format!("{}  {}", s.label, s.value.chars().take(12).collect::<String>());
                let btn = egui::Button::new(RichText::new(txt).font(font_ui(11.5)).color(if used { p.muted } else { p.text }))
                    .fill(if used { tint(p.line, 60) } else { tint(ink, 30) })
                    .stroke(Stroke::new(1.0, tint(ink, if used { 40 } else { 120 })))
                    .rounding(Rounding::same(10.0));
                if ui.add_enabled(!used || !incoming, btn).clicked() {
                    pick = Some(s.leaf.clone());
                }
            }
        });
        if let Some(leaf) = pick {
            let mut o = obj_with_defaults(fields);
            for (k, v) in leaf {
                o.insert(k, v);
            }
            if incoming {
                o.insert("point".into(), Value::from(""));
                list.push(o);
                dirty = true;
            } else {
                draft.editing = None;
                draft.obj = Some(o);
            }
        }
    }

    // Add / edit form.
    ui.add_space(8.0);
    let editing = draft.editing.filter(|i| *i < list.len());
    egui::Frame::none()
        .fill(tint(ink, 10))
        .stroke(Stroke::new(1.0, tint(ink, 70)))
        .rounding(Rounding::same(6.0))
        .inner_margin(egui::Margin::same(8.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(if editing.is_some() { "EDIT BIND" } else { "ADD BIND" }).font(font_ui(10.5)).color(p.muted));
            let mut o = draft.obj.take().unwrap_or_else(|| obj_with_defaults(fields));
            fields_form(ui, draft_id.with("form"), fields, &mut o, p);
            if !incoming {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Point").font(font_ui(12.0)).color(p.muted));
                    crate::register_map::point_picker(ui, draft_id.with("pt"), 200.0, &mut draft.point, points);
                });
            }
            let leaf_ok = ext_leaf(proto.core(), |k| text_of(&o, k)).is_some();
            let ready = leaf_ok && (incoming || !draft.point.trim().is_empty());
            ui.horizontal(|ui| {
                let label = if editing.is_some() { "Save" } else { "Add bind" };
                if ui.add_enabled(ready, egui::Button::new(label)).clicked() {
                    let mut b = o.clone();
                    if !incoming {
                        b.insert("point".into(), Value::from(draft.point.trim()));
                    }
                    match editing {
                        Some(i) => {
                            if incoming {
                                // Changing the address of an Input re-allocates its point.
                                let old_src = bind_source(proto, port.conn, &list[i]);
                                let new_src = bind_source(proto, port.conn, &b);
                                let keep = if old_src == new_src { text_of(&list[i], "point") } else { String::new() };
                                b.insert("point".into(), Value::from(keep));
                            }
                            list[i] = b;
                        }
                        None => {
                            if incoming {
                                b.insert("point".into(), Value::from(""));
                            }
                            list.push(b);
                        }
                    }
                    draft.editing = None;
                    draft.point.clear();
                    o = obj_with_defaults(fields);
                    dirty = true;
                }
                if editing.is_some() && ui.button("Cancel").clicked() {
                    draft.editing = None;
                    o = obj_with_defaults(fields);
                }
            });
            if !leaf_ok {
                let need: Vec<&str> = proto.core().leaf_keys.iter().filter_map(|k| fields.iter().find(|f| f.key == *k).map(|f| f.label)).collect();
                ui.label(RichText::new(format!("Needs: {}", need.join(", "))).font(font_ui(11.0)).color(p.muted));
            }
            draft.obj = Some(o);
        });
    ui.data_mut(|d| d.insert_temp(draft_id, draft));
    dirty
}

// ---------------------------------------------------------------------------
// Art
// ---------------------------------------------------------------------------

fn n(r: Rect, x: f32, y: f32) -> egui::Pos2 {
    egui::pos2(r.min.x + r.width() * x, r.min.y + r.height() * y)
}

/// Rail / banner icon for an ext protocol.
pub fn paint_icon(painter: &Painter, proto: ExtProto, r: Rect, stroke: Stroke, ink: Color32) {
    match proto {
        ExtProto::Wits => {
            // Derrick over a record stream.
            painter.line_segment([n(r, 0.30, 0.86), n(r, 0.50, 0.12)], stroke);
            painter.line_segment([n(r, 0.70, 0.86), n(r, 0.50, 0.12)], stroke);
            painter.line_segment([n(r, 0.36, 0.62), n(r, 0.64, 0.62)], stroke);
            painter.line_segment([n(r, 0.42, 0.38), n(r, 0.58, 0.38)], stroke);
            painter.line_segment([n(r, 0.14, 0.86), n(r, 0.86, 0.86)], stroke);
            painter.circle_filled(n(r, 0.50, 0.12), r.width() * 0.06, ink);
        }
        ExtProto::WebSocket => {
            // Two-way arrows between a browser and a socket.
            let s2 = Stroke::new(stroke.width, ink);
            painter.rect_stroke(Rect::from_min_max(n(r, 0.10, 0.20), n(r, 0.42, 0.80)), Rounding::same(2.0), stroke);
            painter.line_segment([n(r, 0.10, 0.34), n(r, 0.42, 0.34)], stroke);
            painter.circle_stroke(n(r, 0.76, 0.50), r.width() * 0.14, stroke);
            painter.line_segment([n(r, 0.46, 0.42), n(r, 0.60, 0.42)], s2);
            painter.line_segment([n(r, 0.54, 0.36), n(r, 0.60, 0.42)], s2);
            painter.line_segment([n(r, 0.46, 0.58), n(r, 0.60, 0.58)], s2);
            painter.line_segment([n(r, 0.46, 0.58), n(r, 0.52, 0.64)], s2);
        }
        ExtProto::Sparkplug => {
            // Spark: edge node with a flash.
            painter.circle_stroke(n(r, 0.50, 0.52), r.width() * 0.32, stroke);
            let pts = [n(r, 0.56, 0.20), n(r, 0.38, 0.54), n(r, 0.52, 0.54), n(r, 0.44, 0.84), n(r, 0.64, 0.46), n(r, 0.50, 0.46), n(r, 0.56, 0.20)];
            for w in pts.windows(2) {
                painter.line_segment([w[0], w[1]], Stroke::new(stroke.width, ink));
            }
        }
        ExtProto::Dnp3 => {
            // Master → outstation with polling dots.
            painter.rect_stroke(Rect::from_min_max(n(r, 0.08, 0.30), n(r, 0.34, 0.70)), Rounding::same(2.0), stroke);
            painter.rect_stroke(Rect::from_min_max(n(r, 0.66, 0.22), n(r, 0.92, 0.78)), Rounding::same(2.0), stroke);
            for (k, x) in [0.42f32, 0.50, 0.58].iter().enumerate() {
                painter.circle_filled(n(r, *x, 0.50), r.width() * (0.035 + k as f32 * 0.01), ink);
            }
            painter.line_segment([n(r, 0.72, 0.36), n(r, 0.86, 0.36)], stroke);
            painter.line_segment([n(r, 0.72, 0.50), n(r, 0.86, 0.50)], stroke);
            painter.line_segment([n(r, 0.72, 0.64), n(r, 0.86, 0.64)], stroke);
        }
        ExtProto::Can => {
            // Twisted pair bus with three nodes.
            painter.line_segment([n(r, 0.06, 0.40), n(r, 0.94, 0.40)], stroke);
            painter.line_segment([n(r, 0.06, 0.60), n(r, 0.94, 0.60)], Stroke::new(stroke.width, ink));
            for x in [0.24f32, 0.50, 0.76] {
                painter.line_segment([n(r, x, 0.40), n(r, x, 0.18)], stroke);
                painter.circle_filled(n(r, x, 0.16), r.width() * 0.06, ink);
                painter.line_segment([n(r, x, 0.60), n(r, x, 0.80)], stroke);
            }
        }
    }
}

/// Hero graphic on the dark scope display.
pub fn paint_hero(painter: &Painter, proto: ExtProto, r: Rect) {
    let ink = proto.ink_core();
    let icon = Rect::from_center_size(n(r, 0.36, 0.50), vec2(r.height() * 0.62, r.height() * 0.62));
    paint_icon(painter, proto, icon, Stroke::new(2.0, ink), ink);
    if r.width() > 240.0 {
        let tags: &[&str] = match proto {
            ExtProto::Wits => &["&&", "0108", "!!"],
            ExtProto::WebSocket => &["ws://", "JSON", "live"],
            ExtProto::Sparkplug => &["BIRTH", "DATA", "DEATH"],
            ExtProto::Dnp3 => &["class 0", "events", "CROB"],
            ExtProto::Can => &["SocketCAN", "J1939", "DBC"],
        };
        for (i, t) in tags.iter().enumerate() {
            let c = n(r, 0.76, 0.28 + i as f32 * 0.22);
            let g = painter.layout_no_wrap(t.to_string(), font_ui(11.0), ink);
            let pill = Rect::from_center_size(c, g.size() + vec2(14.0, 6.0));
            painter.rect_filled(pill, Rounding::same(8.0), tint(ink, 30));
            painter.rect_stroke(pill, Rounding::same(8.0), Stroke::new(1.0, tint(ink, 140)));
            painter.galley(pill.center() - g.size() * 0.5, g, ink);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_proto_has_a_spec_engine_and_leaf_fields() {
        for p in ExtProto::ALL {
            let spec = p.spec();
            assert_eq!(p.engine().pack_id(), Some(p.id()));
            for k in p.core().leaf_keys {
                assert!(spec.in_fields.iter().any(|f| f.key == *k), "{:?} input lacks leaf field {k}", p);
            }
            // Defaults never contain a leaf value, so an untouched draft is not addable.
            let d = obj_with_defaults(spec.in_fields);
            assert!(ext_leaf(p.core(), |k| text_of(&d, k)).is_none() || matches!(p, ExtProto::Dnp3 | ExtProto::WebSocket), "{:?}", p);
            let mut keys = std::collections::HashSet::new();
            for f in spec.conn_fields {
                assert!(keys.insert(f.key), "{:?} duplicate conn field {}", p, f.key);
            }
        }
    }

    #[test]
    fn integer_defaults_stay_integers_for_stable_addresses() {
        let d = obj_with_defaults(DNP3.in_fields);
        assert_eq!(d["index"], serde_json::json!(0));
        assert_eq!(bind_source(ExtProto::Dnp3, "rtu", &d).unwrap(), "dnp3/rtu::analog:0");
        let mut c = obj_with_defaults(CAN.in_fields);
        c.insert("id".into(), Value::from("pgn:61444"));
        c.insert("start".into(), Value::from(24));
        c.insert("len".into(), Value::from(16));
        assert_eq!(bind_source(ExtProto::Can, "bus", &c).unwrap(), "can/bus::pgn:61444:24:16");
    }

    #[test]
    fn sections_round_trip_through_panels() {
        let mut panels = ExtPanels::default();
        let st = panels.get_mut(ExtProto::Wits);
        let c = st.new_conn();
        st.conns.push(c);
        st.selected = Some(0);
        let sec = ExtSections::from_panels(&panels);
        assert_eq!(sec.wits.connections.len(), 1);
        assert_eq!(sec.wits.selected_id, "wits");
        let json = serde_json::to_value(&sec).unwrap();
        assert!(json.get("dnp3").is_some());
        let back: ExtSections = serde_json::from_value(json).unwrap();
        let mut p2 = ExtPanels::default();
        back.apply_to_panels(&mut p2);
        assert_eq!(p2.get(ExtProto::Wits).selected, Some(0));
        assert_eq!(text_of(&p2.get(ExtProto::Wits).conns[0], "transport"), "tcp_listen");
        // Unique ids on add.
        let st = p2.get_mut(ExtProto::Wits);
        let c2 = st.new_conn();
        assert_eq!(text_of(&c2, "id"), "wits-2");
    }

    #[test]
    fn show_rules_follow_choices() {
        let mut o = obj_with_defaults(WITS.conn_fields);
        let serial = WITS.conn_fields.iter().find(|f| f.key == "serial_port").unwrap();
        assert!(!visible(&o, serial, WITS.conn_fields));
        o.insert("transport".into(), Value::from("serial"));
        assert!(visible(&o, serial, WITS.conn_fields));
    }

    #[test]
    fn live_detail_parses() {
        let d = r#"{"connections":[{"id":"van","endpoint":"x","state":"up","connected":true,"error":"","rx":3,"tx":1,"errors":0,"peers":2,
            "seen":[{"leaf":{"item":"0108"},"label":"0108 Depth","value":"1.5"}]}]}"#;
        let l = parse_live(d);
        assert_eq!(l[0].peers, Some(2));
        assert_eq!(l[0].seen[0].leaf["item"], "0108");
        assert!(parse_live("nope").is_empty());
    }
}
