//! Visual Modbus register map editor.
//!
//! One widget for every place a list of registers is set up — hub **Modbus Input** (registers
//! polled into `modbus_N` points), hub **Modbus Output** (host points written to coils / holding
//! registers) and a **Server** connection's register image (host points served to masters).
//!
//! Layout, top to bottom:
//! * toolbar — `+ Register`, `+ Block`, device templates, CSV import / export, address style
//!   (0-based protocol address or Modicon `40001` reference) and a request summary;
//! * memory strip — one bar per area showing which addresses are used (overlaps in red);
//! * register table — tag name, area badge, address, type / order, scaling, point, live value;
//! * edit card — the selected register's full settings, edited in place.
//!
//! The pure helpers (Modicon references, CSV, blocks, templates) are unit-tested below.

use std::hash::Hash;

use egui::{vec2, Align2, Color32, Rect, RichText, Rounding, Sense, Stroke, Ui};
use pb_engine::{HostSnapshot, PointSnapshot};
use pb_modbus::codec::{self, Area, ByteOrder, DataType};

use crate::form::{self, Cell};
use crate::modbus_panel::{default_modbus_type, map_problems, ModbusBind};
use crate::theme::{font_mono, font_semi, font_ui, space, tint, ThemePalette};

/// Which list this editor is showing; decides allowed areas and whether a point is required.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapRole {
    /// Client poll: register → auto `modbus_N` point.
    Input,
    /// Client write: host point → coil / holding register.
    Output,
    /// Server image: host point served at an address (masters read and write it).
    Server,
}

impl MapRole {
    fn needs_point(self) -> bool {
        !matches!(self, Self::Input)
    }
    fn writable_only(self) -> bool {
        matches!(self, Self::Output)
    }
    fn noun(self) -> &'static str {
        match self {
            Self::Input => "registers polled into points",
            Self::Output => "points written to the device",
            Self::Server => "points served to masters",
        }
    }
}

/// Everything the editor needs besides the list itself.
pub struct MapCtx<'a> {
    pub role: MapRole,
    /// Client connection id (Input / Output) — used to find live values by wire address.
    pub conn: &'a str,
    pub host: &'a HostSnapshot,
    /// Host point names offered by the point picker (Output / Server).
    pub points: &'a [String],
    pub p: &'a ThemePalette,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Drawer {
    #[default]
    None,
    Block,
    Templates,
    Import,
}

/// Per-editor session state (egui temp memory; not saved in the project).
#[derive(Debug, Clone)]
struct EdState {
    selected: Option<usize>,
    modicon: bool,
    drawer: Drawer,
    block: BlockSpec,
    import_text: String,
    message: String,
    filter: String,
}

impl Default for EdState {
    fn default() -> Self {
        Self {
            selected: None,
            modicon: false,
            drawer: Drawer::None,
            block: BlockSpec::default(),
            import_text: String::new(),
            message: String::new(),
            filter: String::new(),
        }
    }
}

/// `+ Block` settings: N consecutive registers of one type.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockSpec {
    pub area: String,
    pub start: u16,
    pub count: u16,
    pub dtype: String,
    pub order: String,
    pub prefix: String,
}

impl Default for BlockSpec {
    fn default() -> Self {
        Self {
            area: "holding".into(),
            start: 0,
            count: 8,
            dtype: "u16".into(),
            order: "abcd".into(),
            prefix: "Reg".into(),
        }
    }
}

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

/// Modicon reference for a 0-based protocol address: coil `00001`, discrete `10001`,
/// input `30001`, holding `40001`. Addresses past 9998 use the 6-digit form (`400001`).
pub fn modicon_ref(area: &str, address: u16) -> String {
    let lead = match Area::parse(area) {
        Some(Area::Coil) => 0,
        Some(Area::Discrete) => 1,
        Some(Area::Input) => 3,
        Some(Area::Holding) => 4,
        None => return address.to_string(),
    };
    let n = address as u32 + 1;
    if n <= 9999 {
        format!("{lead}{n:04}")
    } else {
        format!("{lead}{n:05}")
    }
}

/// Parse a Modicon reference (`40101`, `400101`, `30001`, `1`) → (area, 0-based address).
/// Five digits use the classic form, six the extended one; shorter numbers are coils.
pub fn parse_modicon(text: &str) -> Option<(&'static str, u16)> {
    let t = text.trim();
    if t.is_empty() || !t.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (lead, rest) = match t.len() {
        5 | 6 => (t.as_bytes()[0] - b'0', &t[1..]),
        1..=4 => (0, t),
        _ => return None,
    };
    let area = match lead {
        0 => "coil",
        1 => "discrete",
        3 => "input",
        4 => "holding",
        _ => return None,
    };
    let n: u32 = rest.parse().ok()?;
    if n == 0 || n > 65_536 {
        return None;
    }
    Some((area, (n - 1) as u16))
}

/// First free address after the last register of `area` (0 for an empty area).
pub fn next_address(binds: &[ModbusBind], area: &str) -> u16 {
    binds
        .iter()
        .filter(|b| b.area == area)
        .map(|b| b.address as u32 + b.width() as u32)
        .max()
        .unwrap_or(0)
        .min(65_535) as u16
}

/// A new register that continues the list: same area / type / order as the last one, at the
/// next free address.
pub fn continue_list(binds: &[ModbusBind], role: MapRole) -> ModbusBind {
    let mut b = binds.last().cloned().unwrap_or_default();
    if role.writable_only() && !matches!(b.area.as_str(), "coil" | "holding") {
        b.area = "holding".into();
        b.dtype = default_modbus_type();
    }
    b.address = next_address(binds, &b.area);
    b.name.clear();
    b.point.clear();
    b
}

/// `count` consecutive registers named `prefix 1..count`.
pub fn make_block(spec: &BlockSpec) -> Vec<ModbusBind> {
    let bit_area = Area::parse(&spec.area).is_some_and(|a| a.is_bit());
    let dtype = if bit_area { "bool".to_string() } else { spec.dtype.clone() };
    let width = DataType::parse(&dtype).map(|t| t.width()).unwrap_or(1).max(1) as u32;
    (0..spec.count as u32)
        .map_while(|i| {
            let addr = spec.start as u32 + i * width;
            (addr + width <= 65_536).then(|| ModbusBind {
                name: if spec.prefix.trim().is_empty() {
                    String::new()
                } else {
                    format!("{} {}", spec.prefix.trim(), i + 1)
                },
                area: spec.area.clone(),
                address: addr as u16,
                dtype: dtype.clone(),
                order: spec.order.clone(),
                ..Default::default()
            })
        })
        .collect()
}

const CSV_HEADER: &str = "name,area,address,type,order,unit,scale,offset,units,point";

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn fmt_num(x: f64) -> String {
    if x.fract() == 0.0 && x.abs() < 1e15 {
        format!("{}", x as i64)
    } else {
        format!("{x}")
    }
}

/// Register list → CSV (header + one row per register).
pub fn to_csv(binds: &[ModbusBind]) -> String {
    let mut out = String::from(CSV_HEADER);
    out.push('\n');
    for b in binds {
        let row = [
            csv_field(&b.name),
            b.area.clone(),
            b.address.to_string(),
            b.dtype.clone(),
            b.order.clone(),
            b.unit.to_string(),
            fmt_num(b.scale),
            fmt_num(b.offset),
            csv_field(&b.units),
            csv_field(&b.point),
        ];
        out.push_str(&row.join(","));
        out.push('\n');
    }
    out
}

/// Split one CSV line (quoted fields, `""` escapes).
fn split_csv(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' | '\t' | ';' if !quoted => out.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    out.push(cur);
    out.into_iter().map(|s| s.trim().to_string()).collect()
}

/// CSV → registers. The header row names the columns (any order; `type` / `dtype`,
/// `address` / `addr` / `ref`); without a header the export column order is assumed. An
/// address with a blank area may be a Modicon reference (`40101`). Returns the registers and
/// one message per skipped line.
pub fn from_csv(text: &str) -> (Vec<ModbusBind>, Vec<String>) {
    let mut lines = text.lines().enumerate().filter(|(_, l)| !l.trim().is_empty() && !l.trim_start().starts_with('#'));
    let default_cols: Vec<String> = CSV_HEADER.split(',').map(str::to_string).collect();
    let mut cols = default_cols.clone();
    let mut pending = None;
    if let Some((n, first)) = lines.next() {
        let cells: Vec<String> = split_csv(first).into_iter().map(|c| c.to_ascii_lowercase()).collect();
        if cells.iter().any(|c| matches!(c.as_str(), "area" | "address" | "addr" | "type" | "dtype" | "ref")) {
            cols = cells;
        } else {
            pending = Some((n, first));
        }
    }
    let idx = |names: &[&str]| cols.iter().position(|c| names.contains(&c.as_str()));
    let (i_name, i_area, i_addr, i_type, i_order, i_unit, i_scale, i_off, i_units, i_point) = (
        idx(&["name", "tag", "label", "description"]),
        idx(&["area", "table"]),
        idx(&["address", "addr", "ref", "register"]),
        idx(&["type", "dtype", "datatype"]),
        idx(&["order", "byteorder", "byte_order"]),
        idx(&["unit", "slave", "unit_id"]),
        idx(&["scale", "gain"]),
        idx(&["offset"]),
        idx(&["units", "eng", "uom"]),
        idx(&["point", "point_id"]),
    );
    let mut out = Vec::new();
    let mut errs = Vec::new();
    for (n, line) in pending.into_iter().chain(lines) {
        let cells = split_csv(line);
        let get = |i: Option<usize>| i.and_then(|i| cells.get(i)).map(String::as_str).unwrap_or("");
        let mut b = ModbusBind {
            name: get(i_name).to_string(),
            units: get(i_units).to_string(),
            point: get(i_point).to_string(),
            ..Default::default()
        };
        let area = get(i_area).to_ascii_lowercase();
        let addr = get(i_addr);
        if area.is_empty() {
            match parse_modicon(addr) {
                Some((a, x)) if addr.len() >= 5 => {
                    b.area = a.into();
                    b.address = x;
                }
                _ => {
                    errs.push(format!("line {}: no area and \"{addr}\" is not a Modicon reference", n + 1));
                    continue;
                }
            }
        } else {
            let area = match area.as_str() {
                "0x" | "coils" => "coil".to_string(),
                "1x" | "discrete input" | "discretes" => "discrete".to_string(),
                "3x" | "input register" | "inputs" => "input".to_string(),
                "4x" | "holding register" | "holdings" => "holding".to_string(),
                a => a.to_string(),
            };
            b.area = area;
            match addr.parse::<u16>() {
                Ok(x) => b.address = x,
                Err(_) => {
                    errs.push(format!("line {}: bad address \"{addr}\"", n + 1));
                    continue;
                }
            }
        }
        let dt = get(i_type).to_ascii_lowercase();
        b.dtype = if Area::parse(&b.area).is_some_and(|a| a.is_bit()) {
            "bool".into()
        } else if dt.is_empty() {
            default_modbus_type()
        } else {
            match dt.as_str() {
                "float" | "real" | "float32" => "f32".into(),
                "double" | "float64" => "f64".into(),
                "int" | "int16" | "short" => "i16".into(),
                "uint" | "uint16" | "word" => "u16".into(),
                "int32" | "dint" | "long" => "i32".into(),
                "uint32" | "udint" | "dword" => "u32".into(),
                d => d.to_string(),
            }
        };
        let order = get(i_order).to_ascii_lowercase();
        b.order = if order.is_empty() { "abcd".into() } else { order };
        b.unit = get(i_unit).parse().unwrap_or(0);
        b.scale = get(i_scale).parse().ok().filter(|x: &f64| *x != 0.0).unwrap_or(1.0);
        b.offset = get(i_off).parse().unwrap_or(0.0);
        if let Some(p) = b.problem(false) {
            errs.push(format!("line {}: {p}", n + 1));
            continue;
        }
        out.push(b);
    }
    (out, errs)
}

/// Rows whose (unit, area, address) repeat an earlier row — the poll would read them twice.
pub fn duplicate_rows(binds: &[ModbusBind]) -> Vec<usize> {
    let mut seen = std::collections::HashSet::new();
    binds
        .iter()
        .enumerate()
        .filter(|(_, b)| !seen.insert((b.unit, b.area.clone(), b.address, b.dtype.clone())))
        .map(|(i, _)| i)
        .collect()
}

/// Read requests one poll of this list needs (same planner as the pack).
pub fn request_count(binds: &[ModbusBind]) -> usize {
    let items: Vec<codec::ReadItem> = binds
        .iter()
        .enumerate()
        .filter_map(|(key, b)| {
            let area = Area::parse(&b.area)?;
            Some(codec::ReadItem { key, unit: b.unit, area, address: b.address, width: b.width() })
        })
        .collect();
    codec::plan_reads(&items, 4).len()
}

// ---------------------------------------------------------------------------
// Device templates
// ---------------------------------------------------------------------------

/// A ready-made register list for a common device.
pub struct Template {
    #[allow(dead_code)]
    pub id: &'static str,
    pub title: &'static str,
    pub blurb: &'static str,
    /// True when every register is writable (offered on Output nodes).
    pub writable: bool,
    /// `(name, area, address, type, order, scale, units)`.
    pub rows: &'static [(&'static str, &'static str, u16, &'static str, &'static str, f64, &'static str)],
}

pub const TEMPLATES: &[Template] = &[
    Template {
        id: "power-meter",
        title: "Power meter (3-phase)",
        blurb: "Volts, amps, kW, kVAR, PF, Hz, kWh as float32 input registers",
        writable: false,
        rows: &[
            ("Voltage L1-N", "input", 0, "f32", "abcd", 1.0, "V"),
            ("Voltage L2-N", "input", 2, "f32", "abcd", 1.0, "V"),
            ("Voltage L3-N", "input", 4, "f32", "abcd", 1.0, "V"),
            ("Current L1", "input", 6, "f32", "abcd", 1.0, "A"),
            ("Current L2", "input", 8, "f32", "abcd", 1.0, "A"),
            ("Current L3", "input", 10, "f32", "abcd", 1.0, "A"),
            ("Active power", "input", 12, "f32", "abcd", 1.0, "kW"),
            ("Reactive power", "input", 14, "f32", "abcd", 1.0, "kVAR"),
            ("Power factor", "input", 16, "f32", "abcd", 1.0, ""),
            ("Frequency", "input", 18, "f32", "abcd", 1.0, "Hz"),
            ("Energy import", "input", 20, "f32", "abcd", 1.0, "kWh"),
        ],
    },
    Template {
        id: "vfd-status",
        title: "VFD drive — status",
        blurb: "Status word, speed, current, torque, DC bus, fault code (scaled u16)",
        writable: false,
        rows: &[
            ("Status word", "holding", 0, "u16", "abcd", 1.0, ""),
            ("Running", "holding", 0, "bit2", "abcd", 1.0, ""),
            ("Faulted", "holding", 0, "bit3", "abcd", 1.0, ""),
            ("Output frequency", "holding", 1, "u16", "abcd", 0.01, "Hz"),
            ("Motor speed", "holding", 2, "u16", "abcd", 1.0, "rpm"),
            ("Motor current", "holding", 3, "u16", "abcd", 0.1, "A"),
            ("Motor torque", "holding", 4, "i16", "abcd", 0.1, "%"),
            ("DC bus voltage", "holding", 5, "u16", "abcd", 1.0, "V"),
            ("Fault code", "holding", 6, "u16", "abcd", 1.0, ""),
        ],
    },
    Template {
        id: "vfd-command",
        title: "VFD drive — command",
        blurb: "Control word, speed reference (0.01 Hz), run / reset coils",
        writable: true,
        rows: &[
            ("Control word", "holding", 100, "u16", "abcd", 1.0, ""),
            ("Speed reference", "holding", 101, "u16", "abcd", 0.01, "Hz"),
            ("Run", "coil", 0, "bool", "abcd", 1.0, ""),
            ("Fault reset", "coil", 1, "bool", "abcd", 1.0, ""),
        ],
    },
    Template {
        id: "tank",
        title: "Tank / level transmitter",
        blurb: "Level, volume, temperature (float32 CDAB), high / low switches",
        writable: false,
        rows: &[
            ("Level", "holding", 0, "f32", "cdab", 1.0, "%"),
            ("Volume", "holding", 2, "f32", "cdab", 1.0, "m³"),
            ("Temperature", "holding", 4, "f32", "cdab", 1.0, "°C"),
            ("High level", "discrete", 0, "bool", "abcd", 1.0, ""),
            ("Low level", "discrete", 1, "bool", "abcd", 1.0, ""),
        ],
    },
    Template {
        id: "frac-pump",
        title: "Frac pump (triplex)",
        blurb: "Discharge / suction pressure, rate, engine RPM, gear, temps, hours",
        writable: false,
        rows: &[
            ("Discharge pressure", "holding", 0, "f32", "cdab", 1.0, "psi"),
            ("Suction pressure", "holding", 2, "f32", "cdab", 1.0, "psi"),
            ("Pump rate", "holding", 4, "f32", "cdab", 1.0, "bbl/min"),
            ("Engine speed", "holding", 6, "u16", "abcd", 1.0, "rpm"),
            ("Transmission gear", "holding", 7, "u16", "abcd", 1.0, ""),
            ("Engine coolant temp", "holding", 8, "i16", "abcd", 0.1, "°F"),
            ("Trans oil temp", "holding", 9, "i16", "abcd", 0.1, "°F"),
            ("Engine hours", "holding", 10, "u32", "abcd", 0.1, "h"),
            ("Pump stroke count", "holding", 12, "u32", "abcd", 1.0, ""),
            ("Engine running", "coil", 0, "bool", "abcd", 1.0, ""),
            ("Overpressure trip", "discrete", 0, "bool", "abcd", 1.0, ""),
        ],
    },
    Template {
        id: "flow-computer",
        title: "Flow computer",
        blurb: "Flow rate, totalizer, line pressure / temperature, density",
        writable: false,
        rows: &[
            ("Flow rate", "input", 0, "f32", "abcd", 1.0, "m³/h"),
            ("Totalizer", "input", 2, "u32", "abcd", 0.01, "m³"),
            ("Line pressure", "input", 4, "f32", "abcd", 1.0, "kPa"),
            ("Line temperature", "input", 6, "f32", "abcd", 1.0, "°C"),
            ("Density", "input", 8, "f32", "abcd", 1.0, "kg/m³"),
        ],
    },
    Template {
        id: "pump-command",
        title: "Pump / valve commands",
        blurb: "Start / stop coils, speed and pressure setpoints",
        writable: true,
        rows: &[
            ("Start", "coil", 10, "bool", "abcd", 1.0, ""),
            ("Stop", "coil", 11, "bool", "abcd", 1.0, ""),
            ("Speed setpoint", "holding", 200, "f32", "cdab", 1.0, "%"),
            ("Pressure setpoint", "holding", 202, "f32", "cdab", 1.0, "psi"),
        ],
    },
];

/// Template rows as binds.
pub fn template_binds(t: &Template) -> Vec<ModbusBind> {
    t.rows
        .iter()
        .map(|&(name, area, address, dtype, order, scale, units)| ModbusBind {
            name: name.into(),
            area: area.into(),
            address,
            dtype: dtype.into(),
            order: order.into(),
            scale,
            units: units.into(),
            ..Default::default()
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Widget
// ---------------------------------------------------------------------------

fn area_color(area: &str, p: &ThemePalette) -> Color32 {
    match area {
        "coil" => p.ok,
        "discrete" => p.live,
        "input" => p.accent,
        _ => p.warn,
    }
}

fn area_badge(area: &str) -> &'static str {
    match area {
        "coil" => "0x",
        "discrete" => "1x",
        "input" => "3x",
        _ => "4x",
    }
}

fn shown_address(b: &ModbusBind, modicon: bool) -> String {
    if modicon {
        modicon_ref(&b.area, b.address)
    } else {
        b.address.to_string()
    }
}

fn type_label(b: &ModbusBind) -> String {
    let order = b.data_type().is_some_and(|t| t.uses_order() && t.width() > 1) && b.byte_order() != ByteOrder::Abcd;
    if order {
        format!("{} {}", b.dtype, b.order.to_ascii_uppercase())
    } else {
        b.dtype.clone()
    }
}

fn scale_label(b: &ModbusBind) -> String {
    match (b.scale != 1.0, b.offset != 0.0) {
        (false, false) => String::new(),
        (true, false) => format!("×{}", fmt_num(b.scale)),
        (false, true) => format!("{:+}", b.offset),
        (true, true) => format!("×{} {:+}", fmt_num(b.scale), b.offset),
    }
}

/// Live point for a bind: by point name, else (Input) by wire address.
fn live_point<'a>(b: &ModbusBind, ctx: &'a MapCtx<'_>) -> Option<&'a PointSnapshot> {
    let name = b.point.trim();
    if !name.is_empty() {
        if let Some(pt) = ctx.host.points.iter().find(|p| p.name == name) {
            return Some(pt);
        }
    }
    if ctx.role == MapRole::Input {
        let src = pb_points::addr::modbus_bind_source(ctx.conn, b.unit as u64, &b.area, b.address as u64, &b.dtype, &b.order);
        return ctx.host.points.iter().find(|p| p.source == src);
    }
    None
}

/// Live value for display: floats rounded to 3 decimals (trailing zeros trimmed).
fn short_value(v: &str) -> String {
    let t = v.trim();
    if t.contains('.') && !t.contains(['e', 'E']) {
        if let Ok(x) = t.parse::<f64>() {
            if x.is_finite() {
                let s = format!("{x:.3}");
                let s = s.trim_end_matches('0').trim_end_matches('.');
                return if s == "-0" { "0".into() } else { s.to_string() };
            }
        }
    }
    t.to_string()
}

fn value_text(b: &ModbusBind, live: Option<&PointSnapshot>) -> String {
    match live {
        Some(pt) if !pt.value.is_empty() => {
            let v = short_value(&pt.value);
            if b.units.is_empty() { v } else { format!("{v} {}", b.units) }
        }
        _ => "—".to_string(),
    }
}

fn point_text(b: &ModbusBind, live: Option<&PointSnapshot>, role: MapRole, p: &ThemePalette) -> (String, Color32) {
    if b.point.trim().is_empty() {
        if role == MapRole::Input {
            (live.map(|pt| pt.name.clone()).unwrap_or_else(|| "modbus_N".into()), p.muted)
        } else {
            ("pick point".to_string(), p.warn)
        }
    } else {
        (b.point.trim().to_string(), p.text)
    }
}

/// Grow the point / value columns to fit their widest text, taking the room
/// from the name column (which keeps at least its own widest text, or a floor).
fn widen_cols(c: &mut Cols, want_point: f32, want_value: f32, want_name: f32) {
    let floor = want_name.clamp(96.0, 220.0);
    for (col, want) in [(&mut c.value, want_value.min(220.0)), (&mut c.point, want_point.min(240.0))] {
        if *col > 0.0 && want > *col {
            let give = (want - *col).min((c.name - floor).max(0.0));
            *col += give;
            c.name -= give;
        }
    }
}

fn quality_color(q: &str, p: &ThemePalette) -> Color32 {
    match q.to_ascii_lowercase().as_str() {
        "good" => p.live,
        "bad" | "error" => p.err,
        _ => p.warn,
    }
}

/// Small framed toolbar button.
fn tool_button(ui: &mut Ui, label: &str, on: bool, p: &ThemePalette) -> egui::Response {
    let galley = ui.fonts(|f| f.layout_no_wrap(label.to_owned(), font_ui(12.0), p.text));
    let (rect, resp) = ui.allocate_exact_size(galley.size() + vec2(16.0, 9.0), Sense::click());
    let fill = if on {
        tint(p.accent, 52)
    } else if resp.hovered() {
        tint(p.accent, 22)
    } else {
        p.header_top
    };
    ui.painter().rect_filled(rect, Rounding::same(5.0), fill);
    ui.painter().rect_stroke(rect, Rounding::same(5.0), Stroke::new(1.0, if on { p.accent } else { p.line }));
    ui.painter().galley(rect.center() - galley.size() * 0.5, galley, if on { p.accent } else { p.text });
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Combo listing host points, with a filter box and "use typed id" entry.
pub fn point_picker(ui: &mut Ui, salt: impl Hash, w: f32, value: &mut String, points: &[String]) -> bool {
    let mut changed = false;
    let shown = if value.trim().is_empty() { "(pick a point)".to_string() } else { value.clone() };
    let salt_id = egui::Id::new(("pb_pt_pick", salt));
    egui::ComboBox::from_id_salt(salt_id)
        .selected_text(shown)
        .width(w)
        .height(280.0)
        .show_ui(ui, |ui| {
            let fid = salt_id.with("filter");
            let mut f: String = ui.data_mut(|d| d.get_temp(fid)).unwrap_or_default();
            ui.add(egui::TextEdit::singleline(&mut f).hint_text("filter, or type a new point id").desired_width(220.0));
            let ft = f.trim().to_string();
            if !ft.is_empty() && !points.iter().any(|p| *p == ft) && ui.selectable_label(false, format!("Use \"{ft}\"")).clicked() {
                *value = ft.clone();
                changed = true;
            }
            if !value.is_empty() && ui.selectable_label(false, "(clear)").clicked() {
                value.clear();
                changed = true;
            }
            let low = ft.to_ascii_lowercase();
            for name in points.iter().filter(|n| low.is_empty() || n.to_ascii_lowercase().contains(&low)).take(400) {
                if ui.selectable_label(value == name, name).clicked() {
                    *value = name.clone();
                    changed = true;
                }
            }
            ui.data_mut(|d| d.insert_temp(fid, f));
        });
    changed
}

/// The register map editor. Returns true when `binds` changed (save the project).
pub fn register_map(ui: &mut Ui, salt: impl Hash, binds: &mut Vec<ModbusBind>, ctx: &MapCtx<'_>) -> bool {
    let id = ui.make_persistent_id(("pb_regmap", salt));
    let mut st: EdState = ui.data_mut(|d| d.get_temp(id)).unwrap_or_default();
    if st.selected.is_some_and(|i| i >= binds.len()) {
        st.selected = None;
    }
    let mut dirty = false;
    let p = ctx.p;

    toolbar(ui, id, &mut st, binds, ctx, &mut dirty);
    match st.drawer {
        Drawer::Block => block_drawer(ui, id, &mut st, binds, ctx, &mut dirty),
        Drawer::Templates => template_drawer(ui, &mut st, binds, ctx, &mut dirty),
        Drawer::Import => import_drawer(ui, &mut st, binds, ctx, &mut dirty),
        Drawer::None => {}
    }
    if !st.message.is_empty() {
        ui.label(RichText::new(&st.message).font(font_ui(11.0)).color(p.muted));
    }

    if binds.is_empty() {
        empty_state(ui, ctx);
    } else {
        ui.add_space(4.0);
        memory_strip(ui, binds, &mut st, ctx);
        ui.add_space(6.0);
        table(ui, id, binds, &mut st, ctx, &mut dirty);
    }

    if let Some(i) = st.selected {
        ui.add_space(8.0);
        edit_card(ui, id, i, binds, &mut st, ctx, &mut dirty);
    }

    // Whole-list problems.
    let mut msgs: Vec<String> = Vec::new();
    match ctx.role {
        MapRole::Server => msgs.extend(map_problems(binds)),
        MapRole::Output => {
            for b in binds.iter() {
                if let Some(m) = b.problem(true) {
                    msgs.push(format!("{}: {m}", row_title(b)));
                }
                if Area::parse(&b.area).is_some_and(|a| !a.writable()) {
                    msgs.push(format!("{}: {} is read-only", row_title(b), b.area));
                }
            }
        }
        MapRole::Input => {
            for b in binds.iter() {
                if let Some(m) = b.problem(false) {
                    msgs.push(format!("{}: {m}", row_title(b)));
                }
            }
            for i in duplicate_rows(binds) {
                msgs.push(format!("{}: listed twice", row_title(&binds[i])));
            }
        }
    }
    if !msgs.is_empty() {
        ui.add_space(4.0);
        for m in msgs.iter().take(8) {
            ui.label(RichText::new(format!("⚠ {m}")).font(font_ui(11.0)).color(p.warn));
        }
        if msgs.len() > 8 {
            ui.label(RichText::new(format!("… and {} more", msgs.len() - 8)).font(font_ui(11.0)).color(p.muted));
        }
    }

    ui.data_mut(|d| d.insert_temp(id, st));
    dirty
}

fn row_title(b: &ModbusBind) -> String {
    if b.name.trim().is_empty() {
        b.describe()
    } else {
        format!("{} ({})", b.name.trim(), b.describe())
    }
}

fn toolbar(ui: &mut Ui, _id: egui::Id, st: &mut EdState, binds: &mut Vec<ModbusBind>, ctx: &MapCtx<'_>, dirty: &mut bool) {
    let p = ctx.p;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        if tool_button(ui, "+ Register", false, p).on_hover_text("Add the next register after the last one").clicked() {
            binds.push(continue_list(binds, ctx.role));
            st.selected = Some(binds.len() - 1);
            *dirty = true;
        }
        let toggle = |st: &mut EdState, d: Drawer| st.drawer = if st.drawer == d { Drawer::None } else { d };
        if tool_button(ui, "+ Block", st.drawer == Drawer::Block, p).on_hover_text("Add N consecutive registers").clicked() {
            if st.drawer != Drawer::Block {
                st.block.area = if ctx.role.writable_only() { "holding".into() } else { st.block.area.clone() };
                st.block.start = next_address(binds, &st.block.area);
            }
            toggle(st, Drawer::Block);
        }
        if tool_button(ui, "Templates", st.drawer == Drawer::Templates, p).on_hover_text("Ready-made maps for common devices").clicked() {
            toggle(st, Drawer::Templates);
        }
        if tool_button(ui, "Import", st.drawer == Drawer::Import, p).on_hover_text("Paste a register list (CSV)").clicked() {
            toggle(st, Drawer::Import);
        }
        if tool_button(ui, "Copy CSV", false, p).on_hover_text("Copy this map to the clipboard as CSV").clicked() {
            ui.ctx().copy_text(to_csv(binds));
            st.message = format!("Copied {} register(s) as CSV.", binds.len());
        }
        if !binds.is_empty() && tool_button(ui, "Sort", false, p).on_hover_text("Order by area, then address").clicked() {
            let sel = st.selected.map(|i| binds[i].clone());
            binds.sort_by(|a, b| (a.unit, area_rank(&a.area), a.address).cmp(&(b.unit, area_rank(&b.area), b.address)));
            st.selected = sel.and_then(|s| binds.iter().position(|b| *b == s));
            *dirty = true;
        }
        ui.add_space(4.0);
        if tool_button(ui, if st.modicon { "40001" } else { "0-based" }, st.modicon, p)
            .on_hover_text("Address style: 0-based protocol address, or Modicon reference (40001 = holding 0)")
            .clicked()
        {
            st.modicon = !st.modicon;
        }
    });
    let summary = if binds.is_empty() {
        format!("No registers — {}", ctx.role.noun())
    } else if ctx.role == MapRole::Input {
        let n = request_count(binds);
        format!("{} register(s)  ·  {n} read request{} per poll", binds.len(), if n == 1 { "" } else { "s" })
    } else {
        format!("{} register(s)  ·  {}", binds.len(), ctx.role.noun())
    };
    ui.label(RichText::new(summary).font(font_ui(11.0)).color(p.muted));
}

fn area_rank(a: &str) -> u8 {
    match a {
        "coil" => 0,
        "discrete" => 1,
        "input" => 2,
        _ => 3,
    }
}

fn drawer_frame<R>(ui: &mut Ui, p: &ThemePalette, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::none()
        .fill(tint(p.accent, 10))
        .stroke(Stroke::new(1.0, tint(p.accent, 60)))
        .rounding(Rounding::same(6.0))
        .inner_margin(egui::Margin::same(8.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

fn areas_for(role: MapRole) -> Vec<(&'static str, &'static str)> {
    ModbusBind::AREAS
        .iter()
        .copied()
        .filter(|(k, _)| !role.writable_only() || matches!(*k, "coil" | "holding"))
        .collect()
}

const TYPES: [&str; 8] = ["u16", "i16", "u32", "i32", "u64", "i64", "f32", "f64"];

fn block_drawer(ui: &mut Ui, id: egui::Id, st: &mut EdState, binds: &mut Vec<ModbusBind>, ctx: &MapCtx<'_>, dirty: &mut bool) {
    let p = ctx.p;
    let areas = areas_for(ctx.role);
    drawer_frame(ui, p, |ui| {
        ui.label(RichText::new("ADD A BLOCK").font(font_ui(10.5)).color(p.muted));
        ui.add_space(space::AFTER_HEADING);
        let start_label = if st.modicon { "Start (0-based)" } else { "Start" };
        let changed = form::flow(ui, &mut st.block, p, vec![
            Cell::fixed("Area", 150.0, |ui, w, b: &mut BlockSpec| form::combo(ui, id.with("blk_area"), w, &mut b.area, &areas)),
            Cell::fixed(start_label, 70.0, |ui, w, b: &mut BlockSpec| {
                ui.add_sized([w, form::ROW_H - 2.0], egui::DragValue::new(&mut b.start).range(0..=65535)).changed()
            }),
            Cell::fixed("Count", 56.0, |ui, w, b: &mut BlockSpec| {
                ui.add_sized([w, form::ROW_H - 2.0], egui::DragValue::new(&mut b.count).range(1..=125)).changed()
            }),
            Cell::fixed("Type", 74.0, |ui, w, b: &mut BlockSpec| {
                let opts: Vec<(&str, &str)> = TYPES.iter().map(|t| (*t, *t)).collect();
                ui.add_enabled_ui(!Area::parse(&b.area).is_some_and(|a| a.is_bit()), |ui| form::combo(ui, id.with("blk_type"), w, &mut b.dtype, &opts)).inner
            }),
            Cell::fixed("Order", 64.0, |ui, w, b: &mut BlockSpec| {
                let opts: Vec<(&str, &str)> = ByteOrder::ALL.iter().map(|o| (o.as_str(), o.as_str())).collect();
                form::combo(ui, id.with("blk_order"), w, &mut b.order, &opts)
            }),
            Cell::grow("Name prefix", 80.0, |ui, w, b: &mut BlockSpec| form::text(ui, w, &mut b.prefix)),
        ]);
        let _ = changed;
        let preview = make_block(&st.block);
        if let (Some(first), Some(last)) = (preview.first(), preview.last()) {
            ui.label(
                RichText::new(format!(
                    "{} register(s): {} … {}",
                    preview.len(),
                    shown_address(first, st.modicon),
                    shown_address(last, st.modicon)
                ))
                .font(font_ui(11.0))
                .color(p.muted),
            );
        }
        ui.horizontal(|ui| {
            if tool_button(ui, "Add block", true, p).clicked() && !preview.is_empty() {
                let n = preview.len();
                binds.extend(preview.clone());
                st.message = format!("Added {n} register(s).");
                st.block.start = next_address(binds, &st.block.area);
                st.drawer = Drawer::None;
                *dirty = true;
            }
            if tool_button(ui, "Cancel", false, p).clicked() {
                st.drawer = Drawer::None;
            }
        });
    });
}

fn template_drawer(ui: &mut Ui, st: &mut EdState, binds: &mut Vec<ModbusBind>, ctx: &MapCtx<'_>, dirty: &mut bool) {
    let p = ctx.p;
    drawer_frame(ui, p, |ui| {
        ui.label(RichText::new("DEVICE TEMPLATES  ·  appended at their documented addresses").font(font_ui(10.5)).color(p.muted));
        ui.add_space(space::AFTER_HEADING);
        for t in TEMPLATES.iter().filter(|t| !ctx.role.writable_only() || t.writable) {
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::click());
            let fill = if resp.hovered() { tint(p.accent, 26) } else { tint(p.pane, 0) };
            ui.painter().rect_filled(rect, Rounding::same(5.0), fill);
            ui.painter().text(rect.min + vec2(10.0, 5.0), Align2::LEFT_TOP, t.title, font_semi(12.5), p.text);
            ui.painter().text(rect.min + vec2(10.0, 21.0), Align2::LEFT_TOP, t.blurb, font_ui(11.0), p.muted);
            ui.painter().text(
                rect.right_center() - vec2(10.0, 0.0),
                Align2::RIGHT_CENTER,
                format!("{} regs  +", t.rows.len()),
                font_ui(11.5),
                p.accent,
            );
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                let mut add = template_binds(t);
                if ctx.role.writable_only() {
                    add.retain(|b| Area::parse(&b.area).is_some_and(|a| a.writable()));
                }
                let n = add.len();
                binds.extend(add);
                st.message = if ctx.role.needs_point() {
                    format!("Added {n} register(s) from {} — pick a point for each.", t.title)
                } else {
                    format!("Added {n} register(s) from {}.", t.title)
                };
                st.drawer = Drawer::None;
                *dirty = true;
            }
        }
    });
}

fn import_drawer(ui: &mut Ui, st: &mut EdState, binds: &mut Vec<ModbusBind>, ctx: &MapCtx<'_>, dirty: &mut bool) {
    let p = ctx.p;
    drawer_frame(ui, p, |ui| {
        ui.label(RichText::new("IMPORT CSV").font(font_ui(10.5)).color(p.muted));
        ui.label(
            RichText::new(format!("Columns: {CSV_HEADER}. Header optional; Modicon refs (40101) work with a blank area."))
                .font(font_ui(11.0))
                .color(p.muted),
        );
        ui.add(
            egui::TextEdit::multiline(&mut st.import_text)
                .font(font_mono(11.5))
                .desired_rows(6)
                .desired_width(f32::INFINITY)
                .hint_text("Discharge pressure,holding,0,f32,cdab,0,1,0,psi,"),
        );
        let (parsed, errs) = from_csv(&st.import_text);
        if !st.import_text.trim().is_empty() {
            ui.label(
                RichText::new(format!("{} register(s) ready{}", parsed.len(), if errs.is_empty() { String::new() } else { format!(", {} line(s) skipped", errs.len()) }))
                    .font(font_ui(11.0))
                    .color(if errs.is_empty() { p.muted } else { p.warn }),
            );
            for e in errs.iter().take(4) {
                ui.label(RichText::new(e).font(font_ui(11.0)).color(p.warn));
            }
        }
        ui.horizontal(|ui| {
            let ok = !parsed.is_empty();
            if ui.add_enabled(ok, egui::Button::new("Append")).clicked() {
                st.message = format!("Imported {} register(s).", parsed.len());
                binds.extend(parsed.clone());
                st.import_text.clear();
                st.drawer = Drawer::None;
                *dirty = true;
            }
            if ui.add_enabled(ok, egui::Button::new("Replace all")).clicked() {
                st.message = format!("Replaced the map with {} register(s).", parsed.len());
                *binds = parsed.clone();
                st.selected = None;
                st.import_text.clear();
                st.drawer = Drawer::None;
                *dirty = true;
            }
            if ui.button("Paste example").clicked() {
                st.import_text = to_csv(&template_binds(&TEMPLATES[0]));
            }
            if ui.button("Cancel").clicked() {
                st.drawer = Drawer::None;
            }
        });
    });
}

fn empty_state(ui: &mut Ui, ctx: &MapCtx<'_>) {
    let p = ctx.p;
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, 64.0), Sense::hover());
    ui.painter().rect(
        rect.shrink(1.0),
        Rounding::same(6.0),
        tint(p.accent, 8),
        Stroke::new(1.0, tint(p.line, 200)),
    );
    let tip = match ctx.role {
        MapRole::Input => "Add registers to poll — + Register, a Block, or a device Template.",
        MapRole::Output => "Add coils / holding registers and pick the host point each one writes.",
        MapRole::Server => "Empty — remote masters get exception 2 for every address. Add the points to serve.",
    };
    ui.painter().text(rect.center() - vec2(0.0, 8.0), Align2::CENTER_CENTER, "Register map is empty", font_semi(12.5), p.text);
    ui.painter().text(rect.center() + vec2(0.0, 10.0), Align2::CENTER_CENTER, tip, font_ui(11.0), p.muted);
}

/// One horizontal bar per area: used address ranges, overlaps in red, selection outlined.
fn memory_strip(ui: &mut Ui, binds: &[ModbusBind], st: &mut EdState, ctx: &MapCtx<'_>) {
    let p = ctx.p;
    let mut areas: Vec<&str> = binds.iter().map(|b| b.area.as_str()).collect();
    areas.sort_by_key(|a| area_rank(a));
    areas.dedup();
    let w = ui.available_width();
    for area in areas {
        let rows: Vec<(usize, &ModbusBind)> = binds.iter().enumerate().filter(|(_, b)| b.area == area).collect();
        let lo = rows.iter().map(|(_, b)| b.address as u32).min().unwrap_or(0);
        let hi = rows.iter().map(|(_, b)| b.address as u32 + b.width() as u32).max().unwrap_or(1);
        let span = (hi - lo).max(1) as f32;
        let (rect, _) = ui.allocate_exact_size(vec2(w, 18.0), Sense::hover());
        let badge = Rect::from_min_size(rect.min, vec2(28.0, rect.height()));
        let color = area_color(area, p);
        ui.painter().rect_filled(badge, Rounding::same(4.0), tint(color, 60));
        ui.painter().text(badge.center(), Align2::CENTER_CENTER, area_badge(area), font_semi(10.5), p.text);
        let range_txt = format!(
            "{}–{}",
            shown_address(&ModbusBind { area: area.into(), address: lo as u16, ..Default::default() }, st.modicon),
            shown_address(&ModbusBind { area: area.into(), address: (hi - 1) as u16, ..Default::default() }, st.modicon)
        );
        let label_w = 92.0;
        let bar = Rect::from_min_max(rect.min + vec2(34.0, 3.0), rect.max - vec2(label_w, 3.0));
        ui.painter().rect_filled(bar, Rounding::same(3.0), tint(p.line, 90));
        ui.painter().text(rect.right_center() - vec2(2.0, 0.0), Align2::RIGHT_CENTER, range_txt, font_mono(10.0), p.muted);
        for (i, b) in &rows {
            let x0 = bar.min.x + (b.address as u32 - lo) as f32 / span * bar.width();
            let x1 = bar.min.x + (b.address as u32 + b.width() as u32 - lo) as f32 / span * bar.width();
            let seg = Rect::from_min_max(egui::pos2(x0, bar.min.y), egui::pos2(x1.max(x0 + 3.0), bar.max.y));
            let overlap = rows.iter().any(|(j, o)| {
                j != i
                    && (o.address as u32) < b.address as u32 + b.width() as u32
                    && (b.address as u32) < o.address as u32 + o.width() as u32
                    && !(b.data_type().is_some_and(|t| matches!(t, DataType::Bit(_))) || o.data_type().is_some_and(|t| matches!(t, DataType::Bit(_))))
            });
            let fill = if overlap && ctx.role == MapRole::Server { p.err } else { color };
            ui.painter().rect_filled(seg.shrink2(vec2(0.5, 0.0)), Rounding::same(2.0), fill);
            if st.selected == Some(*i) {
                ui.painter().rect_stroke(seg.expand(1.5), Rounding::same(3.0), Stroke::new(1.5, p.text));
            }
            let resp = ui.interact(seg, ui.id().with(("pb_strip", *i)), Sense::click());
            if resp.clicked() {
                st.selected = Some(*i);
            }
            resp.on_hover_text(format!("{}  ·  {} {}", row_title(b), shown_address(b, st.modicon), type_label(b)));
        }
    }
}

/// Column plan for the table at a given width.
struct Cols {
    name: f32,
    area: f32,
    addr: f32,
    dtype: f32,
    scale: f32,
    point: f32,
    value: f32,
    /// Reserved width for row actions (0: they overlay the value column on hover).
    #[allow(dead_code)]
    act: f32,
}

fn plan_cols(w: f32, role: MapRole) -> Cols {
    // Fixed identity columns, then optional ones by priority while the name keeps room.
    // Row actions overlay the right edge on hover, so they need no column.
    let (area, addr) = (30.0, 52.0);
    let avail = w - 16.0 - area - addr;
    let name_min = 96.0;
    let (mut dtype, mut point) = (0.0, 0.0);
    let mut used = 0.0;
    let take = |want: f32, used: &mut f32| -> f32 {
        if avail - *used - want >= name_min {
            *used += want;
            want
        } else {
            0.0
        }
    };
    let value = take(84.0, &mut used);
    if role != MapRole::Input {
        point = take(100.0, &mut used).max(point);
    }
    dtype = take(74.0, &mut used).max(dtype);
    if role == MapRole::Input {
        point = take(80.0, &mut used).max(point);
    }
    let scale = take(64.0, &mut used);
    Cols { name: (avail - used).max(40.0), area, addr, dtype, scale, point, value, act: 0.0 }
}

pub(crate) fn clip_text(ui: &Ui, text: &str, font: egui::FontId, max_w: f32) -> String {
    if max_w <= 8.0 {
        return String::new();
    }
    let fits = |s: &str| ui.fonts(|f| f.layout_no_wrap(s.to_string(), font.clone(), Color32::WHITE).size().x) <= max_w;
    if fits(text) {
        return text.to_string();
    }
    let mut s: String = text.to_string();
    while !s.is_empty() && !fits(&format!("{s}…")) {
        s.pop();
    }
    format!("{s}…")
}

fn table(ui: &mut Ui, id: egui::Id, binds: &mut Vec<ModbusBind>, st: &mut EdState, ctx: &MapCtx<'_>, dirty: &mut bool) {
    let p = ctx.p;
    let w = ui.available_width();
    let mut c = plan_cols(w, ctx.role);
    {
        let measure = |t: &str, f: egui::FontId| ui.fonts(|fo| fo.layout_no_wrap(t.to_string(), f, Color32::WHITE).size().x);
        let (mut wp, mut wv, mut wn) = (0.0f32, 0.0f32, 0.0f32);
        for b in binds.iter() {
            let live = live_point(b, ctx);
            wp = wp.max(measure(&point_text(b, live, ctx.role, p).0, font_mono(11.5)));
            wv = wv.max(measure(&value_text(b, live), font_ui(12.0)));
            wn = wn.max(measure(b.name.trim(), font_ui(12.5)));
        }
        // Quantize the live value width so the column does not jitter as values tick.
        widen_cols(&mut c, wp + 12.0, ((wv + 10.0) / 16.0).ceil() * 16.0, wn + 12.0);
    }
    let row_h = 24.0;
    // Filter box for long maps.
    if binds.len() > 12 {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Filter").font(font_ui(11.5)).color(p.muted));
            ui.add(egui::TextEdit::singleline(&mut st.filter).desired_width(160.0).hint_text("name, address, point"));
        });
    }
    let filter = st.filter.trim().to_ascii_lowercase();

    // Header.
    let (hrect, _) = ui.allocate_exact_size(vec2(w, 20.0), Sense::hover());
    let mut x = hrect.min.x + 16.0;
    let head = |label: &str, width: f32, x: &mut f32| {
        if width > 0.0 {
            ui.painter().text(egui::pos2(*x + 2.0, hrect.center().y), Align2::LEFT_CENTER, label, font_ui(10.0), p.muted);
            *x += width;
        }
    };
    head("NAME", c.name, &mut x);
    head("", c.area, &mut x);
    head(if st.modicon { "REF" } else { "ADDR" }, c.addr, &mut x);
    head("TYPE", c.dtype, &mut x);
    head("SCALE", c.scale, &mut x);
    head("POINT", c.point, &mut x);
    head("VALUE", c.value, &mut x);
    ui.painter().hline(hrect.x_range(), hrect.max.y, Stroke::new(1.0, p.line));

    let dups = if ctx.role == MapRole::Input { duplicate_rows(binds) } else { Vec::new() };
    let mut action: Option<(usize, &str)> = None;
    for (i, b) in binds.iter().enumerate() {
        if !filter.is_empty() {
            let hay = format!("{} {} {} {} {}", b.name, b.area, b.address, modicon_ref(&b.area, b.address), b.point).to_ascii_lowercase();
            if !hay.contains(&filter) {
                continue;
            }
        }
        let (rect, resp) = ui.allocate_exact_size(vec2(w, row_h), Sense::click());
        let selected = st.selected == Some(i);
        let fill = if selected {
            tint(p.accent, 46)
        } else if resp.hovered() {
            tint(p.accent, 18)
        } else if i % 2 == 1 {
            tint(p.line, 40)
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, Rounding::same(4.0), fill);
        if selected {
            ui.painter().rect_filled(Rect::from_min_size(rect.min, vec2(3.0, row_h)), Rounding::same(1.5), p.accent);
        }
        let problem = match ctx.role {
            MapRole::Input => b.problem(false).or_else(|| dups.contains(&i).then(|| "listed twice".to_string())),
            MapRole::Output => b.problem(true).or_else(|| {
                Area::parse(&b.area).is_some_and(|a| !a.writable()).then(|| format!("{} is read-only", b.area))
            }),
            MapRole::Server => b.problem(true),
        };
        let live = live_point(b, ctx);
        let cy = rect.center().y;
        // Status dot.
        let dot_color = if problem.is_some() {
            p.err
        } else {
            live.map(|pt| quality_color(&pt.quality, p)).unwrap_or(tint(p.muted, 120))
        };
        ui.painter().circle_filled(egui::pos2(rect.min.x + 9.0, cy), 3.5, dot_color);
        let mut x = rect.min.x + 16.0;
        // Name.
        let name = if b.name.trim().is_empty() { "—".to_string() } else { b.name.trim().to_string() };
        let name_col = if b.name.trim().is_empty() { p.muted } else { p.text };
        ui.painter().text(egui::pos2(x + 2.0, cy), Align2::LEFT_CENTER, clip_text(ui, &name, font_ui(12.5), c.name - 6.0), font_ui(12.5), name_col);
        x += c.name;
        // Area badge.
        let badge = Rect::from_center_size(egui::pos2(x + c.area * 0.5 - 2.0, cy), vec2(24.0, 16.0));
        let ac = area_color(&b.area, p);
        ui.painter().rect_filled(badge, Rounding::same(3.0), tint(ac, 70));
        ui.painter().text(badge.center(), Align2::CENTER_CENTER, area_badge(&b.area), font_semi(10.0), p.text);
        x += c.area;
        // Address.
        ui.painter().text(egui::pos2(x + 2.0, cy), Align2::LEFT_CENTER, shown_address(b, st.modicon), font_mono(12.0), p.text);
        x += c.addr;
        // Type.
        if c.dtype > 0.0 {
            ui.painter().text(egui::pos2(x + 2.0, cy), Align2::LEFT_CENTER, clip_text(ui, &type_label(b), font_mono(11.5), c.dtype - 4.0), font_mono(11.5), p.muted);
            x += c.dtype;
        }
        if c.scale > 0.0 {
            let s = scale_label(b);
            ui.painter().text(egui::pos2(x + 2.0, cy), Align2::LEFT_CENTER, clip_text(ui, &s, font_mono(11.0), c.scale - 4.0), font_mono(11.0), p.muted);
            x += c.scale;
        }
        if c.point > 0.0 {
            let (txt, col) = point_text(b, live, ctx.role, p);
            ui.painter().text(egui::pos2(x + 2.0, cy), Align2::LEFT_CENTER, clip_text(ui, &txt, font_mono(11.5), c.point - 6.0), font_mono(11.5), col);
            x += c.point;
        }
        if c.value > 0.0 {
            let txt = value_text(b, live);
            ui.painter().text(egui::pos2(x + 2.0, cy), Align2::LEFT_CENTER, clip_text(ui, &txt, font_ui(12.0), c.value - 4.0), font_ui(12.0), if live.is_some() { p.text } else { p.muted });
        }
        // Row actions (duplicate, delete) on the right edge.
        let del = Rect::from_center_size(egui::pos2(rect.max.x - 12.0, cy), vec2(20.0, 20.0));
        let dup = Rect::from_center_size(egui::pos2(rect.max.x - 34.0, cy), vec2(20.0, 20.0));
        let show_actions = resp.hovered() || selected || ui.rect_contains_pointer(rect);
        let mut on_action = false;
        if show_actions {
            // Actions sit over the value column; give them a solid backing.
            let back = Rect::from_min_max(egui::pos2(dup.min.x - 4.0, rect.min.y + 1.0), egui::pos2(rect.max.x, rect.max.y - 1.0));
            ui.painter().rect_filled(back, Rounding::same(4.0), p.header_top);
            for (r, glyph, what, tip) in [(dup, "", "dup", "Duplicate"), (del, "×", "del", "Delete")] {
                let a = ui.interact(r, id.with(("row_act", i, what)), Sense::click()).on_hover_text(tip);
                if a.hovered() {
                    on_action = true;
                    ui.painter().rect_filled(r, Rounding::same(4.0), tint(if what == "del" { p.err } else { p.accent }, 50));
                }
                if what == "dup" {
                    // Drawn, not a glyph: the bundled fonts have no "copy" symbol.
                    let stroke = egui::Stroke::new(1.3, p.text);
                    let back = Rect::from_min_size(r.center() + vec2(-5.0, -5.0), vec2(7.5, 7.5));
                    let front = back.translate(vec2(3.0, 3.0));
                    ui.painter().rect_stroke(back, Rounding::same(1.5), stroke);
                    ui.painter().rect_filled(front, Rounding::same(1.5), p.header_top);
                    ui.painter().rect_stroke(front, Rounding::same(1.5), stroke);
                } else {
                    ui.painter().text(r.center(), Align2::CENTER_CENTER, glyph, font_ui(14.0), p.text);
                }
                if a.clicked() {
                    action = Some((i, what));
                }
            }
        }
        let mut tip = format!("{}  ·  {}", modicon_ref(&b.area, b.address), b.describe());
        if let Some(m) = &problem {
            tip.push_str(&format!("\n⚠ {m}"));
        }
        if let Some(pt) = live {
            tip.push_str(&format!("\n{} = {} ({})", pt.name, pt.value, pt.quality));
        }
        if !on_action && resp.clone().on_hover_text(tip).clicked() && action.is_none() {
            st.selected = if selected { None } else { Some(i) };
        }
    }
    match action {
        Some((i, "dup")) => {
            let mut b = binds[i].clone();
            b.address = next_address(binds, &b.area);
            b.point.clear();
            if !b.name.is_empty() {
                b.name.push_str(" (copy)");
            }
            binds.insert(i + 1, b);
            st.selected = Some(i + 1);
            *dirty = true;
        }
        Some((i, _)) => {
            binds.remove(i);
            st.selected = match st.selected {
                Some(s) if s == i => None,
                Some(s) if s > i => Some(s - 1),
                s => s,
            };
            *dirty = true;
        }
        None => {}
    }
}

/// Full editor for the selected register.
fn edit_card(ui: &mut Ui, id: egui::Id, i: usize, binds: &mut Vec<ModbusBind>, st: &mut EdState, ctx: &MapCtx<'_>, dirty: &mut bool) {
    let p = ctx.p;
    let areas = areas_for(ctx.role);
    let modicon = st.modicon;
    let mut close = false;
    let mut move_by: i32 = 0;
    egui::Frame::none()
        .fill(tint(p.accent, 12))
        .stroke(Stroke::new(1.0, tint(p.accent, 90)))
        .rounding(Rounding::same(7.0))
        .inner_margin(egui::Margin::same(10.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let b = &mut binds[i];
            ui.horizontal(|ui| {
                let ac = area_color(&b.area, p);
                ui.label(RichText::new(area_badge(&b.area)).font(font_semi(11.0)).color(ac));
                ui.label(
                    RichText::new(format!("{}  ·  {}", modicon_ref(&b.area, b.address), if b.name.is_empty() { "Register" } else { b.name.as_str() }))
                        .font(font_semi(13.0))
                        .color(p.text),
                );
            });
            ui.add_space(6.0);
            let bit_before = Area::parse(&b.area).is_some_and(|a| a.is_bit());
            let mut cells: Vec<Cell<ModbusBind>> = vec![
                Cell::grow("Name", 120.0, |ui, w, d: &mut ModbusBind| form::text(ui, w, &mut d.name)),
                Cell::fixed("Area", 150.0, |ui, w, d: &mut ModbusBind| form::combo(ui, id.with(("ed_area", i)), w, &mut d.area, &areas)),
                Cell::fixed(if modicon { "Address (0-based)" } else { "Address" }, 70.0, |ui, w, d: &mut ModbusBind| {
                    let r = ui.add_sized([w, form::ROW_H - 2.0], egui::DragValue::new(&mut d.address).range(0..=65535));
                    let tip = format!("Modicon {}", modicon_ref(&d.area, d.address));
                    r.on_hover_text(tip).changed()
                }),
            ];
            if ctx.role != MapRole::Server {
                cells.push(Cell::fixed("Unit", 52.0, |ui, w, d: &mut ModbusBind| {
                    ui.add_sized([w, form::ROW_H - 2.0], egui::DragValue::new(&mut d.unit).range(0..=247))
                        .on_hover_text("Slave / unit id; 0 = connection default")
                        .changed()
                }));
            }
            if !bit_before {
                cells.push(Cell::fixed("Type", 80.0, |ui, w, d: &mut ModbusBind| {
                    let kind = match d.data_type() {
                        Some(DataType::Bit(_)) => "bit".to_string(),
                        Some(DataType::Str(_)) => "string".to_string(),
                        _ => d.dtype.clone(),
                    };
                    let mut k = kind.clone();
                    let mut opts: Vec<(&str, &str)> = TYPES.iter().map(|t| (*t, *t)).collect();
                    opts.push(("bit", "bit (of a word)"));
                    opts.push(("string", "string"));
                    form::combo(ui, id.with(("ed_type", i)), w, &mut k, &opts);
                    if k != kind {
                        d.dtype = match k.as_str() {
                            "bit" => "bit0".into(),
                            "string" => "str4".into(),
                            other => other.into(),
                        };
                        return true;
                    }
                    false
                }));
                match b.data_type() {
                    Some(DataType::Bit(_)) => cells.push(Cell::fixed("Bit", 50.0, |ui, w, d: &mut ModbusBind| {
                        let Some(DataType::Bit(mut n)) = d.data_type() else { return false };
                        let ch = ui.add_sized([w, form::ROW_H - 2.0], egui::DragValue::new(&mut n).range(0..=15)).changed();
                        if ch {
                            d.dtype = format!("bit{n}");
                        }
                        ch
                    })),
                    Some(DataType::Str(_)) => cells.push(Cell::fixed("Registers", 54.0, |ui, w, d: &mut ModbusBind| {
                        let Some(DataType::Str(mut n)) = d.data_type() else { return false };
                        let ch = ui.add_sized([w, form::ROW_H - 2.0], egui::DragValue::new(&mut n).range(1..=125)).changed();
                        if ch {
                            d.dtype = format!("str{n}");
                        }
                        ch
                    })),
                    _ => {}
                }
                if b.data_type().is_some_and(|t| t.uses_order()) {
                    cells.push(Cell::grow_to("Byte order", 130.0, 200.0, |ui, w, d: &mut ModbusBind| {
                        let mut cur = d.byte_order();
                        egui::ComboBox::from_id_salt(id.with(("ed_order", i)))
                            .selected_text(cur.label())
                            .width(w)
                            .show_ui(ui, |ui| {
                                for o in ByteOrder::ALL {
                                    ui.selectable_value(&mut cur, o, o.label());
                                }
                            });
                        let ch = cur.as_str() != d.order;
                        d.order = cur.as_str().into();
                        ch
                    }));
                }
                let numeric = b.data_type().is_some_and(|t| !matches!(t, DataType::Bit(_) | DataType::Str(_)));
                if numeric {
                    cells.push(Cell::fixed("Scale", 76.0, |ui, w, d: &mut ModbusBind| {
                        ui.add_sized([w, form::ROW_H - 2.0], egui::DragValue::new(&mut d.scale).speed(0.001).max_decimals(6))
                            .on_hover_text("engineering = raw × scale + offset")
                            .changed()
                    }));
                    cells.push(Cell::fixed("Offset", 70.0, |ui, w, d: &mut ModbusBind| {
                        ui.add_sized([w, form::ROW_H - 2.0], egui::DragValue::new(&mut d.offset).speed(0.1).max_decimals(4)).changed()
                    }));
                }
            }
            cells.push(Cell::fixed("Units", 64.0, |ui, w, d: &mut ModbusBind| form::text(ui, w, &mut d.units)));
            let points = ctx.points;
            let role = ctx.role;
            cells.push(Cell::grow("Point", 130.0, move |ui, w, d: &mut ModbusBind| {
                if role == MapRole::Input {
                    let txt = if d.point.is_empty() { "assigned automatically (modbus_N)".to_string() } else { d.point.clone() };
                    ui.add_sized([w, form::ROW_H - 2.0], egui::Label::new(RichText::new(txt).font(font_mono(11.5))));
                    false
                } else {
                    point_picker(ui, id.with(("ed_pt", i)), w, &mut d.point, points)
                }
            }));
            if form::flow(ui, b, p, cells) {
                let bit_now = Area::parse(&b.area).is_some_and(|a| a.is_bit());
                if bit_now {
                    b.dtype = "bool".into();
                } else if b.dtype == "bool" || bit_before != bit_now {
                    b.dtype = default_modbus_type();
                }
                if b.scale == 0.0 {
                    b.scale = 1.0;
                }
                *dirty = true;
            }
            // Scaling explanation + live sample.
            if b.scale != 1.0 || b.offset != 0.0 {
                ui.label(
                    RichText::new(format!(
                        "Value = raw × {} {} {}{}",
                        fmt_num(b.scale),
                        if b.offset < 0.0 { "−" } else { "+" },
                        fmt_num(b.offset.abs()),
                        if ctx.role == MapRole::Input { "" } else { "  ·  writes divide it back out" }
                    ))
                    .font(font_ui(11.0))
                    .color(p.muted),
                );
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if tool_button(ui, "↑", false, p).on_hover_text("Move up").clicked() {
                    move_by = -1;
                }
                if tool_button(ui, "↓", false, p).on_hover_text("Move down").clicked() {
                    move_by = 1;
                }
                if tool_button(ui, "Done", true, p).clicked() {
                    close = true;
                }
            });
        });
    if move_by != 0 {
        let j = i as i32 + move_by;
        if j >= 0 && (j as usize) < binds.len() {
            binds.swap(i, j as usize);
            st.selected = Some(j as usize);
            *dirty = true;
        }
    }
    if close {
        st.selected = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bind(area: &str, address: u16, dtype: &str) -> ModbusBind {
        ModbusBind { area: area.into(), address, dtype: dtype.into(), ..Default::default() }
    }

    #[test]
    fn modicon_refs_round_trip() {
        assert_eq!(modicon_ref("holding", 0), "40001");
        assert_eq!(modicon_ref("holding", 100), "40101");
        assert_eq!(modicon_ref("input", 9), "30010");
        assert_eq!(modicon_ref("coil", 0), "00001");
        assert_eq!(modicon_ref("discrete", 4), "10005");
        assert_eq!(modicon_ref("holding", 9999), "410000");
        assert_eq!(parse_modicon("40101"), Some(("holding", 100)));
        assert_eq!(parse_modicon("400101"), Some(("holding", 100)));
        assert_eq!(parse_modicon("30001"), Some(("input", 0)));
        assert_eq!(parse_modicon("10005"), Some(("discrete", 4)));
        assert_eq!(parse_modicon("17"), Some(("coil", 16)));
        assert_eq!(parse_modicon("40000"), None);
        assert_eq!(parse_modicon("50001"), None);
        assert_eq!(parse_modicon("abc"), None);
        for area in ["coil", "discrete", "input", "holding"] {
            for a in [0u16, 1, 99, 9998, 9999, 65535] {
                assert_eq!(parse_modicon(&modicon_ref(area, a)), Some((area_static(area), a)), "{area} {a}");
            }
        }
    }

    fn area_static(a: &str) -> &'static str {
        match a {
            "coil" => "coil",
            "discrete" => "discrete",
            "input" => "input",
            _ => "holding",
        }
    }

    #[test]
    fn next_address_and_continue_list() {
        let list = vec![bind("holding", 0, "f32"), bind("holding", 2, "u16"), bind("coil", 5, "bool")];
        assert_eq!(next_address(&list, "holding"), 3);
        assert_eq!(next_address(&list, "coil"), 6);
        assert_eq!(next_address(&list, "input"), 0);
        let n = continue_list(&list, MapRole::Input);
        assert_eq!((n.area.as_str(), n.address, n.dtype.as_str()), ("coil", 6, "bool"));
        let only_input = vec![bind("input", 4, "f32")];
        let o = continue_list(&only_input, MapRole::Output);
        assert_eq!((o.area.as_str(), o.address), ("holding", 0));
        assert_eq!(continue_list(&[], MapRole::Input).address, 0);
    }

    #[test]
    fn blocks_step_by_type_width_and_stop_at_the_top() {
        let spec = BlockSpec { area: "holding".into(), start: 10, count: 3, dtype: "f32".into(), order: "cdab".into(), prefix: "Temp".into() };
        let b = make_block(&spec);
        assert_eq!(b.iter().map(|x| x.address).collect::<Vec<_>>(), vec![10, 12, 14]);
        assert_eq!(b[2].name, "Temp 3");
        assert_eq!(b[0].order, "cdab");
        let coils = make_block(&BlockSpec { area: "coil".into(), dtype: "f32".into(), count: 4, ..Default::default() });
        assert!(coils.iter().all(|c| c.dtype == "bool"));
        let top = make_block(&BlockSpec { start: 65534, count: 5, dtype: "u32".into(), ..Default::default() });
        assert_eq!(top.len(), 1);
        assert!(make_block(&spec).iter().all(|x| x.problem(false).is_none()));
    }

    #[test]
    fn csv_round_trips_and_accepts_foreign_headers() {
        let mut list = template_binds(&TEMPLATES[0]);
        list[0].point = "volts, L1".into();
        list[1].scale = 0.1;
        list[1].offset = -2.5;
        list[2].unit = 3;
        let csv = to_csv(&list);
        let (back, errs) = from_csv(&csv);
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(back, list);

        let foreign = "Tag;Ref;DataType;Gain\nPressure;40101;float;0.5\nRun;00003;;\n";
        let (b, errs) = from_csv(foreign);
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!((b[0].area.as_str(), b[0].address, b[0].dtype.as_str(), b[0].scale), ("holding", 100, "f32", 0.5));
        assert_eq!((b[1].area.as_str(), b[1].address, b[1].dtype.as_str()), ("coil", 2, "bool"));

        // Headerless rows use the export column order; bad rows are reported, good ones kept.
        let (b, errs) = from_csv("Level,holding,0,f32,cdab\nBad,holding,x,u16\nNope,attic,1,u16\n");
        assert_eq!(b.len(), 1);
        assert_eq!(errs.len(), 2);
        assert_eq!(b[0].order, "cdab");
    }

    #[test]
    fn duplicates_and_request_counts() {
        let list = vec![bind("holding", 0, "u16"), bind("holding", 1, "u16"), bind("holding", 0, "u16"), bind("input", 0, "f32")];
        assert_eq!(duplicate_rows(&list), vec![2]);
        assert_eq!(request_count(&list), 2);
        // Two bits of the same word are not duplicates.
        assert!(duplicate_rows(&[bind("holding", 0, "bit1"), bind("holding", 0, "bit2")]).is_empty());
        let far = vec![bind("holding", 0, "u16"), bind("holding", 500, "u16")];
        assert_eq!(request_count(&far), 2);
    }

    #[test]
    fn templates_are_valid_and_writable_ones_only_write() {
        for t in TEMPLATES {
            let binds = template_binds(t);
            assert!(!binds.is_empty(), "{}", t.id);
            for b in &binds {
                assert!(b.problem(false).is_none(), "{}: {} {:?}", t.id, b.describe(), b.problem(false));
                if t.writable {
                    assert!(Area::parse(&b.area).unwrap().writable(), "{}: {}", t.id, b.describe());
                }
            }
            // No overlapping whole registers (bits of one status word are fine).
            let whole: Vec<ModbusBind> = binds.iter().filter(|b| !matches!(b.data_type(), Some(DataType::Bit(_)))).cloned().collect();
            let overlaps: Vec<String> = map_problems(&whole.iter().cloned().map(|mut b| { b.point = "x".into(); b }).collect::<Vec<_>>());
            assert!(overlaps.is_empty(), "{}: {overlaps:?}", t.id);
        }
    }

    #[test]
    fn column_plan_fits_narrow_and_wide() {
        for w in [260.0, 340.0, 420.0, 600.0, 900.0] {
            for role in [MapRole::Input, MapRole::Output, MapRole::Server] {
                let c = plan_cols(w, role);
                let total = 16.0 + c.name + c.area + c.addr + c.dtype + c.scale + c.point + c.value + c.act;
                assert!(total <= w + 1.0 || c.name <= 96.0, "{w} {role:?}: {total}");
                assert!(c.name >= 40.0);
            }
        }
        assert!(plan_cols(900.0, MapRole::Server).scale > 0.0);
        // The live value is the last optional column to go.
        let narrow = plan_cols(300.0, MapRole::Input);
        assert!(narrow.value > 0.0 && narrow.dtype == 0.0, "{}", narrow.value);
    }

    #[test]
    fn values_round_to_three_decimals() {
        assert_eq!(short_value("2129.5496"), "2129.55");
        assert_eq!(short_value("2.9594793123"), "2.959");
        assert_eq!(short_value("67.70622"), "67.706");
        assert_eq!(short_value("7.2"), "7.2");
        assert_eq!(short_value("1549"), "1549");
        assert_eq!(short_value("true"), "true");
        assert_eq!(short_value("-0.0001"), "0");
    }

    #[test]
    fn widen_takes_room_from_name() {
        let mut c = plan_cols(1200.0, MapRole::Server);
        let total = c.name + c.point + c.value;
        widen_cols(&mut c, 180.0, 150.0, 160.0);
        assert!(c.point >= 180.0 && c.value >= 150.0);
        assert!((c.name + c.point + c.value - total).abs() < 0.01);
        assert!(c.name >= 160.0);
    }

    #[test]
    fn scale_labels() {
        let mut b = bind("holding", 0, "u16");
        assert_eq!(scale_label(&b), "");
        b.scale = 0.1;
        assert_eq!(scale_label(&b), "×0.1");
        b.offset = -40.0;
        assert_eq!(scale_label(&b), "×0.1 -40");
    }
}
