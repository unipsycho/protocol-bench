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
    /// Generator: synthetic signals, no device.
    Gen,
    /// Dashboard: a browser page fed by Output binds.
    Dashboard,
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
    blurb: "JSON over WebSocket. Dial a ws:// / wss:// feed as a client, or listen as a server so browsers and web dashboards get live points. A server also hosts a live viewer page at http://host:port/.",
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

/// Every model, in the order the catalog shows them (`gen_ui::MODELS` adds the words).
pub const GEN_MODELS: &[(&str, &str)] = &[
    ("constant", "Constant"),
    ("sine", "Sine"),
    ("square", "Square"),
    ("triangle", "Triangle"),
    ("sawtooth", "Sawtooth"),
    ("ramp", "Ramp"),
    ("noise", "Noise"),
    ("uniform", "Uniform random"),
    ("walk", "Random walk"),
    ("drift", "Drift (mean-reverting)"),
    ("lag", "Wandering setpoint"),
    ("sequence", "Sequence"),
    ("cycle", "On / off cycle"),
    ("toggle", "Toggle"),
    ("flip", "Random on / off"),
    ("counter", "Counter"),
    ("totaliser", "Totaliser"),
    ("formula", "Formula"),
];

const GEN_TRIGGERS: &[(&str, &str)] = &[("interval", "Regular interval"), ("random", "Random timer"), ("burst", "Bursts")];
const GEN_EDGES: &[(&str, &str)] = &[("hold", "Hold at the limit"), ("wrap", "Wrap around"), ("bounce", "Bounce back")];

const M_RANGE: &[&str] = &["sine", "square", "triangle", "sawtooth", "ramp", "uniform", "walk", "lag"];
const M_PERIODIC: &[&str] = &["sine", "square", "triangle", "sawtooth"];
const M_DUTY: &[&str] = &["square", "triangle"];
const M_NOISY: &[&str] = &["sine", "square", "triangle", "sawtooth", "ramp", "lag", "totaliser"];
const M_START: &[&str] = &["ramp", "walk", "totaliser", "counter"];
const M_RATE: &[&str] = &["ramp", "totaliser"];
const M_MEAN: &[&str] = &["noise", "drift"];
const M_FLOAT: &[&str] = &["constant", "sine", "square", "triangle", "sawtooth", "ramp", "noise", "uniform", "walk", "drift", "lag", "sequence", "totaliser", "formula"];

/// Index ranges of `GEN_FIELDS` by group (the inspector draws each group under its own heading).
pub const GEN_SHAPE: std::ops::Range<usize> = 2..25;
pub const GEN_TRIGGER: std::ops::Range<usize> = 25..33;
pub const GEN_OUT: std::ops::Range<usize> = 33..34;

const GEN_FIELDS: &[Field] = &[
    // identity (0..2)
    f("name", "Name", Kind::Text, Def::S(""), 120.0, "the point name — formulas refer to it, e.g. tank_level"),
    f("model", "Type", Kind::Choice(GEN_MODELS), Def::S("sine"), 170.0, ""),
    // shape (2..25)
    fs("value", "Value", num(-1e12, 1e12, 4), Def::N(0.0), 70.0, "", ("model", &["constant"])),
    fs("low", "Min", num(-1e12, 1e12, 4), Def::N(0.0), 70.0, "lowest value", ("model", M_RANGE)),
    fs("high", "Max", num(-1e12, 1e12, 4), Def::N(100.0), 70.0, "highest value", ("model", M_RANGE)),
    fs("period", "Period (s)", num(0.001, 1e7, 3), Def::N(60.0), 62.0, "one full cycle — model seconds", ("model", M_PERIODIC)),
    fs("phase", "Phase (°)", num(-360.0, 360.0, 0), Def::N(0.0), 62.0, "shift the wave: 90 starts at the peak", ("model", &["sine"])),
    fs("duty", "Duty (%)", num(1.0, 99.0, 0), Def::N(50.0), 62.0, "square: time spent high · triangle: share spent rising", ("model", M_DUTY)),
    fs("start", "Start at", num(-1e12, 1e12, 4), Def::N(0.0), 70.0, "first value", ("model", M_START)),
    fs("rate", "Rate /s", num(-1e9, 1e9, 4), Def::N(1.0), 70.0, "units per second (negative = falling)", ("model", M_RATE)),
    fs("edge", "At limits", Kind::Choice(GEN_EDGES), Def::S("hold"), 140.0, "what a ramp does at Min / Max", ("model", &["ramp"])),
    fs("mean", "Mean", num(-1e12, 1e12, 4), Def::N(50.0), 70.0, "centre value", ("model", M_MEAN)),
    fs("sigma", "Spread σ", num(0.0, 1e9, 4), Def::N(1.0), 62.0, "noise: standard deviation · drift: volatility per √s", ("model", M_MEAN)),
    fs("reversion", "Pull back", num(0.0, 1e3, 4), Def::N(0.1), 62.0, "per second; 0 = free random walk", ("model", &["drift"])),
    fs("step", "Step", num(-1e9, 1e9, 4), Def::N(1.0), 70.0, "walk: typical move per value · counter: added each value", ("model", &["walk", "counter"])),
    fs("wrap", "Roll over at", num(0.0, 1e12, 0), Def::N(0.0), 70.0, "counter wraps to 0 here (0 = never)", ("model", &["counter"])),
    fs("hold", "Retarget (s)", num(0.001, 1e7, 2), Def::N(30.0), 62.0, "how long each random setpoint lasts", ("model", &["lag"])),
    fs("tau", "Response (s)", num(0.001, 1e6, 2), Def::N(5.0), 62.0, "time constant — larger = slower to follow", ("model", &["lag"])),
    fs("noise", "Noise σ", num(0.0, 1e9, 4), Def::N(0.0), 62.0, "random jitter added on top", ("model", M_NOISY)),
    fs("on_secs", "On (s)", num(0.001, 1e7, 2), Def::N(30.0), 70.0, "", ("model", &["cycle"])),
    fs("off_secs", "Off (s)", num(0.001, 1e7, 2), Def::N(30.0), 70.0, "", ("model", &["cycle"])),
    fs("prob", "Chance %", num(0.0, 100.0, 1), Def::N(50.0), 62.0, "probability each value is true", ("model", &["flip"])),
    fs("values", "Values", Kind::Text, Def::S("10, 20, 30"), 150.0, "comma separated; steps through them", ("model", &["sequence"])),
    fs("loop", "Repeat", Kind::Bool, Def::B(true), 60.0, "off = stay on the last value", ("model", &["sequence"])),
    fs("expr", "Formula", Kind::Text, Def::S("50 + 10 * sin(t / 20) + randn()"), 150.0, "t, dt, prev, n, other signal names; sin cos abs min max clamp rand() randn() noise(x) if(c,a,b) …", ("model", &["formula"])),
    // trigger (25..33)
    f("trigger", "Generate", Kind::Choice(GEN_TRIGGERS), Def::S("interval"), 140.0, "when a new value is produced"),
    fs("every", "Every (s)", num(0.001, 86400.0, 3), Def::N(1.0), 62.0, "", ("trigger", &["interval"])),
    fs("every_min", "Min (s)", num(0.001, 86400.0, 3), Def::N(0.5), 62.0, "shortest wait before the next value", ("trigger", &["random"])),
    fs("every_max", "Max (s)", num(0.001, 86400.0, 3), Def::N(5.0), 62.0, "longest wait before the next value", ("trigger", &["random"])),
    fs("burst_count", "Per burst", INT(1.0, 10000.0), Def::N(5.0), 62.0, "values in each burst", ("trigger", &["burst"])),
    fs("burst_gap", "Gap (s)", num(0.001, 3600.0, 3), Def::N(0.1), 62.0, "time between values inside a burst", ("trigger", &["burst"])),
    fs("idle_min", "Idle min (s)", num(0.001, 86400.0, 2), Def::N(5.0), 62.0, "quiet time between bursts — shortest", ("trigger", &["burst"])),
    fs("idle_max", "Idle max (s)", num(0.001, 86400.0, 2), Def::N(15.0), 62.0, "quiet time between bursts — longest", ("trigger", &["burst"])),
    // output (33..34)
    fs("decimals", "Decimals", INT(0.0, 9.0), Def::N(3.0), 50.0, "round to this many places", ("model", M_FLOAT)),
];

const GEN: UiSpec = UiSpec {
    blurb: "Invents data instead of reading a device — waveforms, random walks, process shapes, counters and your own formulas — on a regular, random or burst timer. Wire its points to a hub, Aggregator, Calculation or Output like any other source.",
    conn_fields: &[
        f("seed", "Random seed", INT(0.0, 2_000_000_000.0), Def::N(0.0), 100.0, "0 = different every run · any other number repeats the same data"),
        f("speed", "Time scale ×", num(0.01, 100000.0, 2), Def::N(1.0), 90.0, "model seconds per real second — 60 makes a minute of waveform pass each second"),
    ],
    in_fields: GEN_FIELDS,
    out_fields: &[],
    in_heading: "SIGNALS → gen_N",
    out_heading: "",
    interval_in: None,
    interval_out: None,
    note: "Each signal below is one point. A signal's formula can read any other signal on the same board by name. Nothing can be wired into a generator.",
};

const DASHBOARD: UiSpec = UiSpec {
    blurb: "A live web dashboard for everything Protocol Bench converts. The service hosts the page on its own port; anyone on the network can open it in a browser. Wire points to it with a hub Output node.",
    conn_fields: &[
        f("title", "Title", Kind::Text, Def::S("Dashboard"), 200.0, "shown at the top of the page"),
        f("host", "Listen host", Kind::Text, Def::S("0.0.0.0"), 140.0, "0.0.0.0 = reachable from the network"),
        f("port", "Port", INT(1.0, 65535.0), Def::N(8780.0), 70.0, ""),
    ],
    in_fields: &[],
    out_fields: &[],
    in_heading: "",
    out_heading: "",
    interval_in: None,
    interval_out: Some(("Update every (s)", 0.25)),
    note: "Anyone who can reach the port can view the dashboard. Everything wired into this node is available to the dashboard; choose what to show on the page itself.",
};

impl ExtProto {
    pub const ALL: [ExtProto; 7] = [ExtProto::Wits, ExtProto::WebSocket, ExtProto::Sparkplug, ExtProto::Dnp3, ExtProto::Can, ExtProto::Gen, ExtProto::Dashboard];

    pub fn id(self) -> &'static str {
        match self {
            Self::Wits => "wits",
            Self::WebSocket => "websocket",
            Self::Sparkplug => "sparkplug",
            Self::Dnp3 => "dnp3",
            Self::Can => "can",
            Self::Gen => "gen",
            Self::Dashboard => "dashboard",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Wits => "WITS",
            Self::WebSocket => "WebSocket",
            Self::Sparkplug => "Sparkplug B",
            Self::Dnp3 => "DNP3",
            Self::Can => "CAN bus",
            Self::Gen => "Generator",
            Self::Dashboard => "Dashboard",
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
            Self::Gen => &GEN,
            Self::Dashboard => &DASHBOARD,
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
            Self::Gen => Color32::from_rgb(0x7d, 0xe0, 0x9a),
            Self::Dashboard => Color32::from_rgb(0xb3, 0x9d, 0xff),
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
            Self::Gen => Color32::from_rgb(0x1d, 0x76, 0x3f),
            Self::Dashboard => Color32::from_rgb(0x4a, 0x3a, 0xb0),
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

/// Whether `fd` shows for the current values of `o` (its `show` rule).
pub fn is_visible(o: &Obj, fd: &Field, fields: &[Field]) -> bool {
    visible(o, fd, fields)
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

/// `v` with at most `dec` decimals and no trailing zeros.
fn trim_number(v: f64, dec: usize) -> String {
    let s = format!("{v:.dec$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
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
            let mut dv = egui::DragValue::new(&mut x).range(min..=max).speed(speed).max_decimals(dec);
            if dec > 0 {
                // "60" not "60.000": show only the decimals a value needs (keeps narrow forms narrow).
                dv = dv.custom_formatter(move |v, _| trim_number(v, dec)).custom_parser(|s| s.trim().replace(',', ".").parse::<f64>().ok());
            }
            let r = ui.add_sized([w, h], dv);
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

pub(crate) fn status_dot(link: Option<&LiveLink>, pack_loaded: bool, enabled: bool, p: &ThemePalette) -> (Color32, String) {
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
    #[serde(default)]
    pub gen: ExtSection,
    #[serde(default)]
    pub dashboard: ExtSection,
}

impl ExtSections {
    pub fn get(&self, p: ExtProto) -> &ExtSection {
        match p {
            ExtProto::Wits => &self.wits,
            ExtProto::WebSocket => &self.websocket,
            ExtProto::Sparkplug => &self.sparkplug,
            ExtProto::Dnp3 => &self.dnp3,
            ExtProto::Can => &self.can,
            ExtProto::Gen => &self.gen,
            ExtProto::Dashboard => &self.dashboard,
        }
    }
    pub fn get_mut(&mut self, p: ExtProto) -> &mut ExtSection {
        match p {
            ExtProto::Wits => &mut self.wits,
            ExtProto::WebSocket => &mut self.websocket,
            ExtProto::Sparkplug => &mut self.sparkplug,
            ExtProto::Dnp3 => &mut self.dnp3,
            ExtProto::Can => &mut self.can,
            ExtProto::Gen => &mut self.gen,
            ExtProto::Dashboard => &mut self.dashboard,
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
        ExtProto::Dashboard => format!("http://{}:{}/", s("host"), s("port")),
        ExtProto::Gen => {
            let seed = s("seed");
            let speed = s("speed");
            let mut out = if seed.is_empty() || seed == "0" { "random seed".to_string() } else { format!("seed {seed}") };
            if !speed.is_empty() && speed != "1" {
                out.push_str(&format!(" · ×{speed}"));
            }
            out
        }
    }
}

pub(crate) fn section(ui: &mut Ui, title: &str, p: &ThemePalette) {
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
    if proto == ExtProto::Dashboard || (proto == ExtProto::WebSocket && text_of(c, "mode") == "server") {
        ui.add_space(4.0);
        let host = text_of(c, "host");
        let host = if matches!(host.as_str(), "" | "0.0.0.0" | "::") { "localhost".to_string() } else { host };
        let url = format!("http://{host}:{}/", text_of(c, "port"));
        ui.horizontal(|ui| {
            let (label, tip) = if proto == ExtProto::Dashboard {
                ("Open dashboard in browser", "The live dashboard page served by this connection")
            } else {
                ("Open viewer in browser", "Live page served by this server — shows the points it sends")
            };
            if ui.button(label).on_hover_text(tip).clicked() {
                crate::app::open_in_browser(&url);
            }
            ui.label(RichText::new(&url).font(font_mono(11.0)).color(p.muted));
        });
    }
    ui.add_space(space::AFTER_FIELD);
    ui.label(RichText::new(spec.note).font(font_ui(11.0)).color(p.muted));

    // Seen on the wire.
    if let Some(l) = link.filter(|l| !l.seen.is_empty()) {
        ui.add_space(space::SECTION);
        let heading = if proto == ExtProto::Gen { "LIVE SIGNALS" } else { "SEEN ON THE WIRE" };
        section(ui, &format!("{heading}  ·  {}", l.seen.len()), p);
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
            let vr = ui.painter().text(r.right_center() - vec2(6.0, 0.0), Align2::RIGHT_CENTER, s.value.chars().take(32).collect::<String>(), font_mono(11.5), ink);
            let label = crate::register_map::clip_text(ui, &s.label, font_ui(12.0), (vr.min.x - r.min.x - 18.0).max(40.0));
            ui.painter().text(r.left_center() + vec2(6.0, 0.0), Align2::LEFT_CENTER, label, font_ui(12.0), p.text);
        }
    }
    if proto == ExtProto::Gen {
        ui.add_space(space::SECTION);
        crate::gen_ui::types_reference(ui, p);
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
    if proto == ExtProto::Gen {
        let model = text_of(b, "model");
        let name = text_of(b, "name");
        return if model.is_empty() { name } else { format!("{name} · {model}") };
    }
    if proto == ExtProto::Dashboard {
        return "dashboard".into();
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

pub(crate) fn live_value<'a>(host: &'a HostSnapshot, point: &str, source: Option<&str>) -> Option<&'a pb_engine::PointSnapshot> {
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
    /// Signal names already used by the board's other Generator nodes (they share one namespace).
    pub other_names: Vec<String>,
    pub conn: &'a mut String,
    pub interval: &'a mut f32,
    /// Dashboard node only: (page title, listen port).
    pub dash: Option<(&'a mut String, &'a mut u16)>,
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
    if proto == ExtProto::Gen {
        // Generators have their own catalog-style inspector (and no Output side).
        return incoming && crate::gen_ui::edit_port(ui, port_key, port, conns, host, p);
    }
    apply_form_spacing(ui);
    let spec = proto.spec();
    let ink = if p.is_light { proto.ink_deep() } else { proto.ink_core() };
    let mut dirty = false;
    if let (ExtProto::Dashboard, Some((title, listen))) = (proto, port.dash) {
        // The node is the connection: just a title and a port.
        section(ui, "DASHBOARD", p);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Title").font(font_ui(12.0)).color(p.muted));
            dirty |= ui.add(egui::TextEdit::singleline(title).desired_width(180.0).hint_text("Dashboard")).changed();
            ui.label(RichText::new("Port").font(font_ui(12.0)).color(p.muted));
            let mut v = (*listen).max(1) as u32;
            if ui.add(egui::DragValue::new(&mut v).range(1..=65535).speed(1.0)).changed() {
                *listen = v as u16;
                dirty = true;
            }
            if let Some((label, _)) = spec.interval_out {
                ui.label(RichText::new(label).font(font_ui(12.0)).color(p.muted));
                dirty |= ui.add(egui::DragValue::new(port.interval).range(0.02..=3600.0).speed(0.05).max_decimals(2)).changed();
            }
        });
    } else {
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
    }
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
    if proto == ExtProto::Dashboard && !port.conn.is_empty() {
        // The pack reports its real address once it is serving.
        match link.filter(|l| l.endpoint.starts_with("http")).map(|l| viewer_url(&l.endpoint)) {
            Some(url) => {
                ui.horizontal(|ui| {
                    if ui.button("Open dashboard in browser").on_hover_text("The live dashboard page served by this connection").clicked() {
                        crate::app::open_in_browser(&url);
                    }
                    ui.label(RichText::new(&url).font(font_mono(11.0)).color(p.muted));
                });
            }
            None => {
                ui.label(RichText::new("Start the service and this connection to open the dashboard.").font(font_ui(11.0)).color(p.muted));
            }
        }
    }
    if conns.is_empty() && proto != ExtProto::Dashboard {
        ui.label(RichText::new(format!("Add a connection in the {} panel.", proto.title())).font(font_ui(11.0)).color(p.muted));
    }
    if proto == ExtProto::Dashboard {
        // No bind list: everything wired into the node is exposed, and the dashboard page is
        // where the user picks what to show.
        ui.add_space(space::SECTION - 2.0);
        let n = port.outputs.len();
        let msg = if n == 0 {
            "Nothing wired yet — draw an arrow from an Input into this node and its points become available on the dashboard.".to_string()
        } else {
            format!("{n} point{} wired in — all available on the dashboard.", if n == 1 { "" } else { "s" })
        };
        ui.label(RichText::new(msg).font(font_ui(11.5)).color(p.muted));
        return dirty;
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

/// Binds for a Dashboard node: every wired point gets one (marked `auto`), in wire order, keeping
/// the units already set; manual binds (no `auto` mark) are left alone; auto binds whose point is
/// no longer wired go away.
pub fn dashboard_auto_binds(current: &[Obj], wired: &[String]) -> Vec<Obj> {
    let mut next: Vec<Obj> = current.iter().filter(|b| !matches!(b.get("auto"), Some(Value::Bool(true)))).cloned().collect();
    for point in wired {
        if next.iter().any(|b| text_of(b, "point") == *point) {
            continue;
        }
        let mut b = current.iter().find(|b| text_of(b, "point") == *point).cloned().unwrap_or_else(|| obj_with_defaults(DASHBOARD.out_fields));
        b.insert("point".into(), Value::from(point.clone()));
        b.insert("auto".into(), Value::Bool(true));
        next.push(b);
    }
    next
}

/// Connection id for a Dashboard node.
pub fn dashboard_conn_id(hub_id: u64, port_id: u64) -> String {
    format!("dash_{hub_id}_{port_id}")
}

/// A Dashboard node *is* its connection. Keep the `dashboard` connection list in step with the
/// Dashboard Output nodes: one connection per node (id derived from the node, title / port / label
/// from the node), none left over. A node from an older project (a hand-made connection picked in
/// the old dropdown) adopts that connection's title and port; a fresh node gets the first free
/// port from 8780. Idempotent; returns true when anything changed.
pub fn sync_dashboard_conns(hubs: &mut crate::hubs::HubModel, conns: &mut Vec<Obj>) -> bool {
    use crate::hubs::HubProto;
    let old = conns.clone();
    let mut want: Vec<Obj> = Vec::new();
    let mut used: Vec<u16> = Vec::new();
    // Ports already chosen, so a new node can't collide with them.
    for h in &hubs.hubs {
        for o in h.outputs.iter().filter(|o| o.protocol == HubProto::Dashboard) {
            if o.dash_port != 0 {
                used.push(o.dash_port);
            }
        }
    }
    for h in &mut hubs.hubs {
        let hub_id = h.id;
        for o in h.outputs.iter_mut().filter(|o| o.protocol == HubProto::Dashboard) {
            if o.dash_port == 0 || o.dash_title.is_empty() {
                // Adopt the old hand-made connection this node pointed at, if any.
                if let Some(c) = old.iter().find(|c| text_of(c, "id") == o.ext_connection) {
                    if o.dash_port == 0 {
                        o.dash_port = text_of(c, "port").parse().unwrap_or(0);
                    }
                    if o.dash_title.is_empty() {
                        o.dash_title = text_of(c, "title");
                    }
                }
            }
            if o.dash_port == 0 {
                let free = (8780u16..=65535).find(|p| !used.contains(p)).unwrap_or(8780);
                o.dash_port = free;
                used.push(free);
            }
            let id = dashboard_conn_id(hub_id, o.id);
            o.ext_connection = id.clone();
            let mut c = old.iter().find(|c| text_of(c, "id") == id).cloned().unwrap_or_default();
            c.insert("id".into(), Value::from(id));
            c.insert("label".into(), Value::from(o.label.clone()));
            c.insert("title".into(), Value::from(if o.dash_title.trim().is_empty() { o.label.clone() } else { o.dash_title.clone() }));
            c.insert("host".into(), Value::from("0.0.0.0"));
            c.insert("port".into(), Value::from(o.dash_port));
            c.entry("enabled").or_insert(Value::Bool(true));
            want.push(c);
        }
    }
    if want == old {
        return false;
    }
    *conns = want;
    true
}

/// Address to open in this machine's browser: a wildcard listen host becomes `localhost`.
fn viewer_url(endpoint: &str) -> String {
    endpoint.replacen("//0.0.0.0", "//localhost", 1).replacen("//[::]", "//localhost", 1)
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
        ExtProto::Dashboard => {
            // Dashboard: a window with a gauge arc and a bar.
            painter.rect_stroke(Rect::from_min_max(n(r, 0.08, 0.16), n(r, 0.92, 0.84)), Rounding::same(3.0), stroke);
            painter.circle_stroke(n(r, 0.34, 0.56), r.width() * 0.16, stroke);
            painter.line_segment([n(r, 0.34, 0.56), n(r, 0.42, 0.44)], Stroke::new(stroke.width, ink));
            painter.rect_filled(Rect::from_min_max(n(r, 0.62, 0.56), n(r, 0.72, 0.74)), Rounding::same(1.0), ink);
            painter.rect_filled(Rect::from_min_max(n(r, 0.76, 0.40), n(r, 0.86, 0.74)), Rounding::same(1.0), ink);
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
        ExtProto::Gen => {
            // A wave leaving a small source box: sine through a dashed baseline.
            painter.rect_stroke(Rect::from_min_max(n(r, 0.06, 0.30), n(r, 0.30, 0.70)), Rounding::same(2.0), stroke);
            let mut prev = None;
            for i in 0..=20 {
                let x = 0.38 + 0.56 * i as f32 / 20.0;
                let y = 0.50 - 0.26 * (i as f32 / 20.0 * std::f32::consts::TAU * 1.5).sin();
                let pt = n(r, x, y);
                if let Some(q) = prev {
                    painter.line_segment([q, pt], Stroke::new(stroke.width, ink));
                }
                prev = Some(pt);
            }
            painter.line_segment([n(r, 0.30, 0.50), n(r, 0.38, 0.50)], stroke);
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
            ExtProto::Gen => &["sine", "walk", "formula"],
            ExtProto::Dashboard => &["gauges", "charts", "live"],
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

    fn dash_model() -> crate::hubs::HubModel {
        let mut m = crate::hubs::HubModel::sample();
        let hub = m.hubs[1].id;
        m.hub_mut(hub).unwrap().outputs[0].protocol = crate::hubs::HubProto::Dashboard;
        m
    }

    #[test]
    fn dashboard_node_owns_its_connection() {
        let mut m = dash_model();
        let mut conns: Vec<Obj> = Vec::new();
        assert!(sync_dashboard_conns(&mut m, &mut conns));
        assert_eq!(conns.len(), 1);
        let port = m.hubs[1].outputs[0].dash_port;
        assert_eq!(port, 8780, "first free port");
        assert_eq!(m.hubs[1].outputs[0].ext_connection, text_of(&conns[0], "id"));
        assert_eq!(text_of(&conns[0], "port"), "8780");
        assert!(!sync_dashboard_conns(&mut m, &mut conns), "idempotent");
        // Edits on the node flow to the connection; a second node gets the next port.
        m.hubs[1].outputs[0].dash_title = "Plant".into();
        m.hubs[1].outputs[0].dash_port = 9000;
        let hub = m.hubs[1].id;
        m.add_port(hub, false);
        let n = m.hubs[1].outputs.len() - 1;
        m.hubs[1].outputs[n].protocol = crate::hubs::HubProto::Dashboard;
        assert!(sync_dashboard_conns(&mut m, &mut conns));
        assert_eq!(conns.len(), 2);
        assert_eq!((text_of(&conns[0], "title"), text_of(&conns[0], "port")), ("Plant".into(), "9000".into()));
        assert_eq!(m.hubs[1].outputs[n].dash_port, 8780);
        // Removing the node removes its connection.
        m.hubs[1].outputs[n].protocol = crate::hubs::HubProto::Mqtt;
        assert!(sync_dashboard_conns(&mut m, &mut conns));
        assert_eq!(conns.len(), 1);
    }

    #[test]
    fn old_hand_made_connection_is_adopted() {
        let mut m = dash_model();
        m.hubs[1].outputs[0].ext_connection = "web".into();
        let mut conns = vec![serde_json::json!({"id":"web","title":"Old","port":8800}).as_object().unwrap().clone()];
        assert!(sync_dashboard_conns(&mut m, &mut conns));
        assert_eq!(m.hubs[1].outputs[0].dash_port, 8800);
        assert_eq!(m.hubs[1].outputs[0].dash_title, "Old");
        assert_eq!(conns.len(), 1);
        assert_ne!(text_of(&conns[0], "id"), "web");
    }

    #[test]
    fn wired_points_become_dashboard_binds() {
        let wired = vec!["a".to_string(), "b".to_string()];
        let mut binds = dashboard_auto_binds(&[], &wired);
        assert_eq!(binds.iter().map(|b| text_of(b, "point")).collect::<Vec<_>>(), ["a", "b"]);
        assert!(binds.iter().all(|b| b.get("auto") == Some(&Value::Bool(true))));
        // Re-sync keeps existing binds; unwiring drops the auto bind; a manual bind survives.
        let mut manual = obj_with_defaults(DASHBOARD.out_fields);
        manual.insert("point".into(), Value::from("m"));
        binds.push(manual);
        let next = dashboard_auto_binds(&binds, &["a".to_string()]);
        assert_eq!(next.iter().map(|b| text_of(b, "point")).collect::<Vec<_>>(), ["m", "a"]);
    }

    #[test]
    fn dashboard_labels_and_viewer_url() {
        let b = obj_with_defaults(ExtProto::Dashboard.spec().out_fields);
        assert_eq!(bind_label(ExtProto::Dashboard, &b), "dashboard");
        assert_eq!(viewer_url("http://0.0.0.0:8780/"), "http://localhost:8780/");
        assert_eq!(viewer_url("http://10.0.0.5:8780/"), "http://10.0.0.5:8780/");
    }

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
            assert!(ext_leaf(p.core(), |k| text_of(&d, k)).is_none() || matches!(p, ExtProto::Dnp3 | ExtProto::WebSocket | ExtProto::Dashboard), "{:?}", p);
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
