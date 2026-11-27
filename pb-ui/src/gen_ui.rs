//! Generator UI: the hub Input inspector for a Generator node and the extras on its panel.
//!
//! A generator signal is one bind on a hub Input node. Adding one works like picking from a
//! catalog rather than filling a blank form:
//!
//! 1. choose a **category** tab — *Real-world* presets (temperature, tank level, pump run, …) or
//!    one of the model families (Waveform, Random, Process, Digital, Counter, Formula);
//! 2. click a **card** (each draws its own shape) or a preset chip;
//! 3. tweak only the parameters that model has, pick *when* it generates (regular / random timer /
//!    bursts) and watch the **preview** redraw as you type;
//! 4. Add.
//!
//! The shapes come from `pb-signal`, the same code the pack runs, so the preview is the real
//! thing, not an illustration.

use egui::{pos2, vec2, Align2, Color32, Painter, Rect, RichText, Rounding, Sense, Stroke, Ui};
use pb_engine::HostSnapshot;
use pb_signal::Kind;
use serde_json::Value;

use crate::ext_proto::{self, fields_form, obj_with_defaults, text_of, ExtProto, Obj, PortFields, GEN_OUT, GEN_SHAPE, GEN_TRIGGER};
use crate::theme::{apply_form_spacing, font_mono, font_semi, font_ui, space, tint, ThemePalette};

// ---------------------------------------------------------------------------
// Catalog
// ---------------------------------------------------------------------------

/// One model in the catalog.
pub struct ModelUi {
    pub id: &'static str,
    pub category: &'static str,
    pub label: &'static str,
    pub desc: &'static str,
}

pub const MODELS: &[ModelUi] = &[
    ModelUi { id: "constant", category: "Waveform", label: "Constant", desc: "A fixed value. Handy as a setpoint other formulas read." },
    ModelUi { id: "sine", category: "Waveform", label: "Sine", desc: "Swings smoothly between Min and Max once per period." },
    ModelUi { id: "square", category: "Waveform", label: "Square", desc: "Jumps between Min and Max; Duty is the share of the period spent high." },
    ModelUi { id: "triangle", category: "Waveform", label: "Triangle", desc: "Ramps up and down. Duty sets the rising share — 70 % gives a slow fill and a quicker drain." },
    ModelUi { id: "sawtooth", category: "Waveform", label: "Sawtooth", desc: "Ramps from Min to Max, then snaps back." },
    ModelUi { id: "ramp", category: "Waveform", label: "Ramp", desc: "Moves at a fixed rate from a start value — battery drain, warm-up. Hold, wrap or bounce at the limits." },
    ModelUi { id: "noise", category: "Random", label: "Noise", desc: "Random values scattered around a mean (gaussian)." },
    ModelUi { id: "uniform", category: "Random", label: "Uniform", desc: "Any value between Min and Max, all equally likely." },
    ModelUi { id: "walk", category: "Random", label: "Random walk", desc: "Wanders: each value is the last one plus a random step, kept between Min and Max." },
    ModelUi { id: "drift", category: "Random", label: "Drift", desc: "Wanders like a walk but is pulled back toward a mean — room temperature, line voltage." },
    ModelUi { id: "lag", category: "Process", label: "Wandering setpoint", desc: "A smooth value chasing a new random target every so often — a valve, a heater, a flow loop." },
    ModelUi { id: "sequence", category: "Process", label: "Sequence", desc: "Steps through your list of numbers, one per value; repeat or stay on the last." },
    ModelUi { id: "cycle", category: "Digital", label: "On / off cycle", desc: "True for N seconds, false for N seconds, repeating — a pump or compressor." },
    ModelUi { id: "toggle", category: "Digital", label: "Toggle", desc: "Flips true / false on every value — a heartbeat." },
    ModelUi { id: "flip", category: "Digital", label: "Random on / off", desc: "True with a given chance on each value — random alarms and events." },
    ModelUi { id: "counter", category: "Counter", label: "Counter", desc: "Adds a step on each value, optionally rolling over — production counts, message numbers." },
    ModelUi { id: "totaliser", category: "Counter", label: "Totaliser", desc: "Adds rate × elapsed time — energy, volume, run hours. Stays right even with irregular timing." },
    ModelUi { id: "formula", category: "Formula", label: "Formula", desc: "Your own expression: t, dt, prev, n, other signals by name, sin cos abs min max clamp rand() randn() if(c,a,b)." },
];

/// Catalog tabs: the presets first, then the model families in catalog order.
pub const TABS: &[&str] = &["Real-world", "Waveform", "Random", "Process", "Digital", "Counter", "Formula"];

/// A ready-made signal: a name, a unit, and the bind fields it sets.
pub struct Preset {
    pub label: &'static str,
    pub unit: &'static str,
    pub desc: &'static str,
    pub params: &'static [(&'static str, &'static str)],
}

pub const PRESETS: &[Preset] = &[
    Preset { label: "Temperature", unit: "°C", desc: "Room temperature hovering near 21 °C.", params: &[("name", "temperature"), ("model", "drift"), ("mean", "21"), ("sigma", "0.15"), ("reversion", "0.05"), ("decimals", "1")] },
    Preset { label: "Humidity", unit: "%", desc: "Relative humidity drifting around 45 %.", params: &[("name", "humidity"), ("model", "drift"), ("mean", "45"), ("sigma", "0.4"), ("reversion", "0.03"), ("decimals", "1")] },
    Preset { label: "Tank level", unit: "%", desc: "Fills slowly, drains faster, with a little slosh. Three-minute cycle.", params: &[("name", "tank_level"), ("model", "triangle"), ("low", "20"), ("high", "85"), ("period", "180"), ("duty", "65"), ("noise", "0.3"), ("decimals", "1")] },
    Preset { label: "Pump running", unit: "on/off", desc: "On 45 s, off 75 s.", params: &[("name", "pump_run"), ("model", "cycle"), ("on_secs", "45"), ("off_secs", "75")] },
    Preset { label: "Discharge pressure", unit: "kPa", desc: "Pulsing around 400 kPa with a little jitter.", params: &[("name", "pressure"), ("model", "sine"), ("low", "380"), ("high", "420"), ("period", "20"), ("noise", "1.5"), ("decimals", "1")] },
    Preset { label: "Flow rate", unit: "m³/h", desc: "Settles toward a new flow target every 30 s.", params: &[("name", "flow"), ("model", "lag"), ("low", "40"), ("high", "120"), ("hold", "30"), ("tau", "6"), ("noise", "0.8"), ("decimals", "1")] },
    Preset { label: "Motor current", unit: "A", desc: "Steady load around 12.5 A.", params: &[("name", "motor_amps"), ("model", "drift"), ("mean", "12.5"), ("sigma", "0.3"), ("reversion", "0.2"), ("decimals", "2")] },
    Preset { label: "Vibration", unit: "mm/s", desc: "Random vibration velocity around 2.2 mm/s.", params: &[("name", "vibration"), ("model", "noise"), ("mean", "2.2"), ("sigma", "0.35"), ("decimals", "2")] },
    Preset { label: "Battery voltage", unit: "V", desc: "Slowly discharging from 13.8 V and holding at 11.8 V.", params: &[("name", "battery_v"), ("model", "ramp"), ("start", "13.8"), ("rate", "-0.002"), ("low", "11.8"), ("high", "13.8"), ("edge", "hold"), ("noise", "0.01"), ("decimals", "2")] },
    Preset { label: "Energy", unit: "kWh", desc: "Totaliser for a 45 kW load.", params: &[("name", "energy_kwh"), ("model", "totaliser"), ("start", "12000"), ("rate", "0.0125"), ("noise", "0.002"), ("decimals", "2")] },
    Preset { label: "Daily load", unit: "kW", desc: "A day compressed into two minutes: low overnight, peak mid-afternoon.", params: &[("name", "load_kw"), ("model", "sine"), ("low", "120"), ("high", "480"), ("period", "120"), ("phase", "-90"), ("noise", "6"), ("decimals", "0")] },
    Preset { label: "Valve position", unit: "%", desc: "Steps through 0 → 100 → 0 in quarters, every 5 s.", params: &[("name", "valve_pos"), ("model", "sequence"), ("values", "0, 25, 50, 75, 100, 75, 50, 25"), ("every", "5")] },
    Preset { label: "Wind speed", unit: "m/s", desc: "A random walk between calm and gale.", params: &[("name", "wind"), ("model", "walk"), ("start", "6"), ("step", "0.4"), ("low", "0"), ("high", "18"), ("decimals", "1")] },
    Preset { label: "Heartbeat", unit: "count", desc: "A message counter that rolls over at 65536.", params: &[("name", "heartbeat"), ("model", "counter"), ("step", "1"), ("wrap", "65536")] },
    Preset { label: "Random alarm", unit: "on/off", desc: "Fires on a random timer, true about one time in twelve.", params: &[("name", "alarm"), ("model", "flip"), ("prob", "8"), ("trigger", "random"), ("every_min", "2"), ("every_max", "20")] },
    Preset { label: "Parts counter", unit: "count", desc: "Counts in bursts of six, then idles 10–25 s.", params: &[("name", "parts"), ("model", "counter"), ("step", "1"), ("trigger", "burst"), ("burst_count", "6"), ("burst_gap", "0.2"), ("idle_min", "10"), ("idle_max", "25")] },
];

/// Label for a model id.
pub fn model_label(id: &str) -> &'static str {
    MODELS.iter().find(|m| m.id == id).map(|m| m.label).unwrap_or("?")
}

fn model_ui(id: &str) -> Option<&'static ModelUi> {
    MODELS.iter().find(|m| m.id == id)
}

// ---------------------------------------------------------------------------
// Draft handling
// ---------------------------------------------------------------------------

fn fields() -> &'static [ext_proto::Field] {
    ExtProto::Gen.spec().in_fields
}

/// A new signal draft: every field at its default.
pub fn new_draft() -> Obj {
    obj_with_defaults(fields())
}

/// Fill any keys an older / hand-edited bind lacks.
fn complete(o: &mut Obj) {
    for fd in fields() {
        o.entry(fd.key.to_string()).or_insert_with(|| ext_proto::def_value(fd));
    }
}

fn typed(key: &str, raw: &str) -> Value {
    match fields().iter().find(|f| f.key == key).map(|f| f.kind) {
        Some(ext_proto::Kind::Num { dec, .. }) => match raw.trim().parse::<f64>() {
            Ok(v) if dec == 0 => Value::from(v.round() as i64),
            Ok(v) => Value::from(v),
            Err(_) => Value::from(raw),
        },
        Some(ext_proto::Kind::Bool) => Value::from(matches!(raw.trim(), "true" | "1" | "yes" | "on")),
        _ => Value::from(raw),
    }
}

/// Reset the parameters of one model to defaults (name and trigger are kept).
fn reset_shape(o: &mut Obj) {
    for r in [GEN_SHAPE, GEN_OUT] {
        for fd in &fields()[r] {
            o.insert(fd.key.into(), ext_proto::def_value(fd));
        }
    }
}

/// Switch the draft to `model`, resetting its parameters.
pub fn pick_model(o: &mut Obj, model: &str) {
    reset_shape(o);
    o.insert("model".into(), Value::from(model));
}

/// Apply a preset on top of a fresh draft; the trigger resets unless the preset sets one.
pub fn apply_preset(o: &mut Obj, preset: &Preset) {
    reset_shape(o);
    for fd in &fields()[GEN_TRIGGER] {
        o.insert(fd.key.into(), ext_proto::def_value(fd));
    }
    for (k, v) in preset.params {
        o.insert((*k).into(), typed(k, v));
    }
}

/// `base`, `base_2`, … not already used by another signal.
pub fn unique_name(base: &str, used: &dyn Fn(&str) -> bool) -> String {
    let base = base.trim();
    let base = if base.is_empty() { "signal" } else { base };
    if !used(base) {
        return base.to_string();
    }
    (2..).map(|i| format!("{base}_{i}")).find(|n| !used(n)).unwrap()
}

fn name_of(o: &Obj) -> String {
    text_of(o, "name").trim().to_string()
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

/// Draw a curve into `rect`; bools are stepped, everything else a polyline.
fn draw_curve(painter: &Painter, rect: Rect, pts: &[(f64, f64)], kind: Kind, ink: Color32, width: f32) {
    if pts.len() < 2 {
        return;
    }
    let (x0, x1) = (pts[0].0, pts[pts.len() - 1].0);
    let (mut lo, mut hi) = pts.iter().fold((f64::MAX, f64::MIN), |(a, b), (_, v)| (a.min(*v), b.max(*v)));
    if kind == Kind::Bool {
        lo = 0.0;
        hi = 1.0;
    }
    if (hi - lo).abs() < 1e-12 {
        lo -= 1.0;
        hi += 1.0;
    }
    let xr = (x1 - x0).max(1e-12);
    let to = |t: f64, v: f64| pos2(rect.min.x + rect.width() * ((t - x0) / xr) as f32, rect.max.y - rect.height() * ((v - lo) / (hi - lo)) as f32);
    let mut line: Vec<egui::Pos2> = Vec::with_capacity(pts.len() * 2);
    let mut prev: Option<f64> = None;
    for (t, v) in pts {
        if kind != Kind::Float {
            if let Some(pv) = prev {
                line.push(to(*t, pv));
            }
        }
        line.push(to(*t, *v));
        prev = Some(*v);
    }
    painter.add(egui::Shape::line(line, Stroke::new(width, ink)));
}

/// The model's own tiny shape, for its catalog card (cached per model in egui memory).
fn card_shape(ui: &Ui, id: &str) -> Option<(Vec<(f64, f64)>, Kind)> {
    let key = egui::Id::new(("pb_gen_card_shape", id));
    if let Some(v) = ui.data(|d| d.get_temp::<(Vec<(f64, f64)>, bool, bool)>(key)) {
        let kind = if v.1 { Kind::Bool } else if v.2 { Kind::Int } else { Kind::Float };
        return Some((v.0, kind));
    }
    let mut o = new_draft();
    pick_model(&mut o, id);
    o.insert("name".into(), Value::from("card"));
    // Make the card shapes show off their character rather than the (slow) defaults.
    match id {
        "sine" | "square" | "triangle" | "sawtooth" => {
            o.insert("period".into(), Value::from(20));
            if id == "triangle" {
                o.insert("duty".into(), Value::from(70));
            }
        }
        "ramp" => {
            o.insert("edge".into(), Value::from("bounce"));
            o.insert("rate".into(), Value::from(10));
        }
        "lag" => {
            o.insert("hold".into(), Value::from(10));
            o.insert("tau".into(), Value::from(2));
        }
        "cycle" => {
            o.insert("on_secs".into(), Value::from(8));
            o.insert("off_secs".into(), Value::from(12));
        }
        "noise" => {
            o.insert("mean".into(), Value::from(50));
            o.insert("sigma".into(), Value::from(6));
        }
        "walk" => {
            o.insert("step".into(), Value::from(4));
        }
        "drift" => {
            o.insert("sigma".into(), Value::from(6));
            o.insert("reversion".into(), Value::from(0.4));
        }
        "sequence" => {
            o.insert("values".into(), Value::from("20, 60, 40, 90, 10"));
        }
        "formula" => {
            o.insert("expr".into(), Value::from("50 + 30 * sin(t / 6) * sin(t / 17)"));
        }
        "totaliser" | "counter" => {
            o.insert("step".into(), Value::from(1));
            o.insert("rate".into(), Value::from(1));
        }
        _ => {}
    }
    let pv = pb_signal::preview(&o).ok()?;
    ui.data_mut(|d| d.insert_temp(key, (pv.points.clone(), pv.kind == Kind::Bool, pv.kind == Kind::Int)));
    Some((pv.points, pv.kind))
}

fn fmt_val(v: f64) -> String {
    if v.abs() >= 10_000.0 {
        format!("{v:.0}")
    } else if v.abs() >= 100.0 {
        format!("{v:.1}")
    } else {
        let t = format!("{v:.2}");
        t.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

fn fmt_span(s: f64) -> String {
    if s < 1.0 {
        format!("{:.0} ms", s * 1000.0)
    } else if s < 120.0 {
        format!("{} s", fmt_val(s))
    } else if s < 7200.0 {
        format!("{} min", fmt_val(s / 60.0))
    } else {
        format!("{} h", fmt_val(s / 3600.0))
    }
}

/// The big preview under the editor.
fn paint_preview(ui: &mut Ui, o: &Obj, ink: Color32, p: &ThemePalette) -> Result<(), String> {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, 86.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, Rounding::same(6.0), tint(ink, 12));
    painter.rect_stroke(rect, Rounding::same(6.0), Stroke::new(1.0, tint(ink, 60)));
    let plot = Rect::from_min_max(rect.min + vec2(8.0, 10.0), rect.max - vec2(54.0, 20.0));
    for k in 1..4 {
        let y = plot.min.y + plot.height() * k as f32 / 4.0;
        painter.line_segment([pos2(plot.min.x, y), pos2(plot.max.x, y)], Stroke::new(1.0, tint(p.line, 90)));
    }
    match pb_signal::preview(o) {
        Ok(pv) => {
            draw_curve(&painter, plot, &pv.points, pv.kind, ink, 1.8);
            let (lo, hi) = pv.points.iter().fold((f64::MAX, f64::MIN), |(a, b), (_, v)| (a.min(*v), b.max(*v)));
            let (lo, hi) = if pv.kind == Kind::Bool { (0.0, 1.0) } else { (lo, hi) };
            painter.text(pos2(rect.max.x - 6.0, plot.min.y), Align2::RIGHT_TOP, fmt_val(hi), font_mono(10.5), p.muted);
            painter.text(pos2(rect.max.x - 6.0, plot.max.y), Align2::RIGHT_BOTTOM, fmt_val(lo), font_mono(10.5), p.muted);
            painter.text(pos2(plot.min.x, rect.max.y - 4.0), Align2::LEFT_BOTTOM, "0", font_mono(10.0), p.muted);
            painter.text(pos2(plot.max.x, rect.max.y - 4.0), Align2::RIGHT_BOTTOM, format!("{} of model time", fmt_span(pv.span_secs)), font_ui(10.5), p.muted);
            Ok(())
        }
        Err(e) => {
            painter.text(rect.center(), Align2::CENTER_CENTER, "no preview", font_ui(12.0), p.muted);
            Err(e)
        }
    }
}

// ---------------------------------------------------------------------------
// Inspector
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct Draft {
    obj: Option<Obj>,
    editing: Option<usize>,
    /// Index into [`TABS`].
    tab: usize,
    open: bool,
    /// Just opened: scroll the editor into view once.
    fresh: bool,
}

#[derive(Clone, Default)]
struct Hist {
    last_ms: u64,
    vals: Vec<f32>,
}

const HIST_MAX: usize = 60;

/// Live history of one point, appended when the host reports a new update.
fn history(ui: &Ui, point: &str, updated_ms: u64, value: &str) -> Vec<f32> {
    let key = egui::Id::new(("pb_gen_hist", point));
    let mut h: Hist = ui.data(|d| d.get_temp(key)).unwrap_or_default();
    if updated_ms != h.last_ms {
        h.last_ms = updated_ms;
        let v = match value.trim() {
            "true" => Some(1.0),
            "false" => Some(0.0),
            s => s.parse::<f32>().ok(),
        };
        if let Some(v) = v {
            h.vals.push(v);
            if h.vals.len() > HIST_MAX {
                h.vals.remove(0);
            }
        }
        ui.data_mut(|d| d.insert_temp(key, h.clone()));
    }
    h.vals
}

fn paint_spark(painter: &Painter, rect: Rect, vals: &[f32], ink: Color32) {
    if vals.len() < 2 {
        return;
    }
    let pts: Vec<(f64, f64)> = vals.iter().enumerate().map(|(i, v)| (i as f64, *v as f64)).collect();
    draw_curve(painter, rect, &pts, Kind::Float, ink, 1.3);
}

/// Hub Input inspector for a Generator node. Returns true when it changed.
#[allow(clippy::too_many_arguments)]
pub fn edit_port(
    ui: &mut Ui,
    port_key: (u64, u64),
    port: PortFields<'_>,
    conns: &[(String, String)],
    host: &HostSnapshot,
    p: &ThemePalette,
) -> bool {
    apply_form_spacing(ui);
    let proto = ExtProto::Gen;
    let ink = if p.is_light { proto.ink_deep() } else { proto.ink_core() };
    let mut dirty = false;

    // Board.
    ext_proto::section(ui, "GENERATOR BOARD", p);
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("Board").font(font_ui(12.0)).color(p.muted));
        let shown = conns.iter().find(|(id, _)| id == port.conn).map(|(_, l)| l.clone()).unwrap_or_else(|| if port.conn.is_empty() { "(pick board)".into() } else { port.conn.clone() });
        let mut next = port.conn.clone();
        egui::ComboBox::from_id_salt(("pb_gen_board", port_key)).selected_text(shown).width(180.0).show_ui(ui, |ui| {
            for (id, l) in conns {
                ui.selectable_value(&mut next, id.clone(), l);
            }
        });
        if next != *port.conn {
            *port.conn = next;
            dirty = true;
        }
    });
    let (live, pack_loaded) = ext_proto::live_for(host, proto);
    let link = live.iter().find(|l| l.id == *port.conn);
    if !port.conn.is_empty() {
        let (dot, state) = ext_proto::status_dot(link, pack_loaded, true, p);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("●").font(font_ui(12.0)).color(dot));
            ui.label(RichText::new(state).font(font_ui(12.0)).color(p.text));
        });
        if let Some(l) = link.filter(|l| !l.error.is_empty()) {
            ui.label(RichText::new(&l.error).font(font_ui(11.0)).color(p.warn));
        }
    }
    if conns.is_empty() {
        ui.label(RichText::new("Add a board in the Generator panel.").font(font_ui(11.0)).color(p.muted));
    }

    // Signals.
    ui.add_space(space::SECTION - 2.0);
    let draft_id = egui::Id::new(("pb_gen_draft", port_key));
    let mut draft: Draft = ui.data_mut(|d| d.get_temp(draft_id)).unwrap_or_default();
    ext_proto::section(ui, &format!("SIGNALS  ·  {}", port.inputs.len()), p);
    let w = ui.available_width();
    let mut remove = None;
    let mut edit = None;
    for (i, b) in port.inputs.iter().enumerate() {
        let name = name_of(b);
        let point = text_of(b, "point");
        let source = ext_proto::bind_source(proto, port.conn, b);
        let lv = ext_proto::live_value(host, &point, source.as_deref());
        let (r, resp) = ui.allocate_exact_size(vec2(w, 38.0), Sense::click());
        let sel = draft.editing == Some(i);
        let fill = if sel { tint(ink, 40) } else if resp.hovered() { tint(ink, 18) } else if i % 2 == 1 { tint(p.line, 40) } else { Color32::TRANSPARENT };
        ui.painter().rect_filled(r, Rounding::same(4.0), fill);
        let dotc = lv.map(|x| match x.quality.as_str() {
            "good" => p.live,
            "bad" => p.err,
            _ => p.warn,
        });
        ui.painter().circle_filled(pos2(r.min.x + 9.0, r.center().y), 3.5, dotc.unwrap_or(tint(p.muted, 120)));
        let model = text_of(b, "model");
        let sub = format!("{}  ·  {}", model_label(&model), pb_signal::describe_trigger(b));
        let xr = Rect::from_center_size(pos2(r.max.x - 12.0, r.center().y), vec2(20.0, 20.0));
        let val = lv.map(|x| x.value.chars().take(14).collect::<String>()).unwrap_or_else(|| "—".into());
        let vr = ui.painter().text(pos2(r.max.x - 28.0, r.min.y + 4.0), Align2::RIGHT_TOP, val, font_mono(12.0), if lv.is_some() { ink } else { p.muted });
        let mut right_edge = vr.min.x - 8.0;
        if let Some(x) = lv {
            let h = history(ui, &x.name, x.updated_ms, &x.value);
            if h.len() >= 2 && r.width() > 250.0 {
                let sp = Rect::from_min_size(pos2(r.max.x - 28.0 - 70.0, r.min.y + 20.0), vec2(70.0, 13.0));
                paint_spark(&ui.painter_at(r), sp, &h, ink);
                right_edge = right_edge.min(sp.min.x - 8.0);
            }
        }
        let room = (right_edge - (r.min.x + 20.0)).max(40.0);
        let nm = crate::register_map::clip_text(ui, &name, font_semi(12.5), room);
        ui.painter().text(pos2(r.min.x + 20.0, r.min.y + 4.0), Align2::LEFT_TOP, nm, font_semi(12.5), p.text);
        let sb = crate::register_map::clip_text(ui, &sub, font_ui(10.5), (r.max.x - 32.0 - (r.min.x + 20.0)).max(40.0) - if right_edge < r.max.x - 100.0 { 76.0 } else { 0.0 });
        ui.painter().text(pos2(r.min.x + 20.0, r.min.y + 21.0), Align2::LEFT_TOP, sb, font_ui(10.5), p.muted);
        let xresp = ui.interact(xr, draft_id.with(("x", i)), Sense::click()).on_hover_text("Remove");
        ui.painter().text(xr.center(), Align2::CENTER_CENTER, "×", font_ui(14.0), if xresp.hovered() { p.err } else { p.muted });
        if xresp.clicked() {
            remove = Some(i);
        } else if resp.clicked() {
            edit = Some(i);
        }
    }
    if let Some(i) = remove {
        port.inputs.remove(i);
        if draft.editing == Some(i) {
            draft.editing = None;
            draft.obj = None;
            draft.open = false;
        }
        dirty = true;
    }
    if let Some(i) = edit {
        if draft.editing == Some(i) {
            draft = Draft { tab: draft.tab, ..Default::default() };
        } else {
            let mut o = port.inputs[i].clone();
            complete(&mut o);
            let tab = TABS.iter().position(|t| model_ui(&text_of(&o, "model")).is_some_and(|m| m.category == *t)).unwrap_or(draft.tab);
            draft = Draft { obj: Some(o), editing: Some(i), tab, open: true, fresh: true };
        }
    }
    if port.inputs.is_empty() && !draft.open {
        ui.label(RichText::new("No signals yet — add one below.").font(font_ui(11.5)).color(p.muted));
    }

    // Editor.
    ui.add_space(8.0);
    if !draft.open {
        if ui.add(egui::Button::new(RichText::new("+  Add signal").font(font_semi(12.5)).color(p.text)).fill(tint(ink, 30)).stroke(Stroke::new(1.0, tint(ink, 120))).rounding(Rounding::same(6.0)).min_size(vec2(w, 30.0))).clicked() {
            let mut o = new_draft();
            o.insert("name".into(), Value::from(unique_name("sine", &|n| port.other_names.iter().any(|x| x == n) || port.inputs.iter().any(|b| name_of(b) == n))));
            draft = Draft { obj: Some(o), editing: None, tab: 0, open: true, fresh: true };
        }
    } else {
        let mut used_names: Vec<String> = port.inputs.iter().enumerate().filter(|(i, _)| Some(*i) != draft.editing).map(|(_, b)| name_of(b)).collect();
        used_names.extend(port.other_names.iter().cloned());
        let mut o = draft.obj.take().unwrap_or_else(new_draft);
        complete(&mut o);
        let editing = draft.editing.filter(|i| *i < port.inputs.len());
        let mut close = false;
        let mut commit = false;
        let frame = egui::Frame::none()
            .fill(tint(ink, 10))
            .stroke(Stroke::new(1.0, tint(ink, 70)))
            .rounding(Rounding::same(6.0))
            .inner_margin(egui::Margin::same(8.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(if editing.is_some() { "EDIT SIGNAL" } else { "NEW SIGNAL" }).font(font_ui(10.5)).color(p.muted));
                ui.add_space(4.0);
                paint_catalog(ui, &mut o, &mut draft.tab, &used_names, port_key, ink, p);
                if let Some(desc) = model_ui(&text_of(&o, "model")).map(|m| m.desc) {
                    ui.add_space(4.0);
                    ui.label(RichText::new(desc).font(font_ui(11.0)).color(p.muted));
                }
                ui.add_space(8.0);
                // The shape sits right under the catalog so a pick is answered at once; it is
                // redrawn every frame from the real signal engine.
                let preview = paint_preview(ui, &o, ink, p);
                if let Err(e) = &preview {
                    ui.label(RichText::new(e).font(font_ui(11.5)).color(p.warn));
                }
                ui.add_space(8.0);

                let salt = draft_id.with("form");
                ui.label(RichText::new("SIGNAL").font(font_ui(10.0)).color(p.muted));
                ui.add_space(space::AFTER_HEADING);
                fields_form(ui, salt.with("id"), &fields()[0..2], &mut o, p);
                let shape_shown = fields()[GEN_SHAPE].iter().any(|fd| ext_proto::is_visible(&o, fd, fields()));
                if shape_shown {
                    ui.add_space(space::AFTER_FIELD);
                    ui.label(RichText::new("SHAPE").font(font_ui(10.0)).color(p.muted));
                    ui.add_space(space::AFTER_HEADING);
                    fields_form(ui, salt.with("shape"), &fields()[GEN_SHAPE], &mut o, p);
                }
                ui.add_space(space::AFTER_FIELD);
                ui.label(RichText::new("WHEN").font(font_ui(10.0)).color(p.muted));
                ui.add_space(space::AFTER_HEADING);
                fields_form(ui, salt.with("trig"), &fields()[GEN_TRIGGER], &mut o, p);
                if fields()[GEN_OUT].iter().any(|fd| ext_proto::is_visible(&o, fd, fields())) {
                    ui.add_space(space::AFTER_FIELD);
                    fields_form(ui, salt.with("out"), &fields()[GEN_OUT], &mut o, p);
                }
                // Edits above change the shape: validate the values as they are now.
                let preview = if preview.is_ok() { pb_signal::preview(&o).map(|_| ()) } else { preview.map(|_| ()) };
                ui.add_space(6.0);

                let name = name_of(&o);
                let dup = used_names.iter().any(|n| *n == name);
                let name_problem = if name.is_empty() {
                    Some("Give the signal a name.")
                } else if dup {
                    Some("Another signal on this board already has that name.")
                } else if name.contains(['/', ':']) {
                    Some("Names cannot contain / or :.")
                } else {
                    None
                };
                if let Some(msg) = name_problem {
                    ui.label(RichText::new(msg).font(font_ui(11.0)).color(p.muted));
                }
                let ready = name_problem.is_none() && preview.is_ok();
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let label = if editing.is_some() { "Save" } else { "Add signal" };
                    if ui.add_enabled(ready, egui::Button::new(label)).clicked() {
                        commit = true;
                    }
                    if ui.button(if editing.is_some() { "Cancel" } else { "Close" }).clicked() {
                        close = true;
                    }
                });
            });
        if std::mem::take(&mut draft.fresh) {
            frame.response.scroll_to_me(Some(egui::Align::TOP));
        }
        if commit {
            let mut b = o.clone();
            match editing {
                Some(i) => {
                    let old = ext_proto::bind_source(proto, port.conn, &port.inputs[i]);
                    let new = ext_proto::bind_source(proto, port.conn, &b);
                    // Renaming re-allocates the point; any other edit keeps its id.
                    let keep = if old == new { text_of(&port.inputs[i], "point") } else { String::new() };
                    b.insert("point".into(), Value::from(keep));
                    port.inputs[i] = b;
                    draft = Draft { tab: draft.tab, ..Default::default() };
                }
                None => {
                    b.insert("point".into(), Value::from(""));
                    port.inputs.push(b);
                    // Stay open on a fresh draft so several signals can be added in a row.
                    let mut fresh = new_draft();
                    let first = TABS.get(draft.tab).copied().and_then(|t| MODELS.iter().find(|m| m.category == t)).map(|m| m.id).unwrap_or("sine");
                    pick_model(&mut fresh, first);
                    let taken = |n: &str| port.other_names.iter().any(|x| x == n) || port.inputs.iter().any(|b| name_of(b) == n);
                    fresh.insert("name".into(), Value::from(unique_name(first, &taken)));
                    o = fresh;
                }
            }
            dirty = true;
        }
        if close {
            draft = Draft { tab: draft.tab, ..Default::default() };
        } else if draft.open {
            draft.obj = Some(o);
        }
    }
    ui.data_mut(|d| d.insert_temp(draft_id, draft));
    dirty
}

/// Tabs plus the cards / presets of the selected tab. Picking one rewrites the draft.
fn paint_catalog(ui: &mut Ui, o: &mut Obj, tab: &mut usize, used: &[String], port_key: (u64, u64), ink: Color32, p: &ThemePalette) {
    // Tabs.
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        for (i, t) in TABS.iter().enumerate() {
            let sel = *tab == i;
            let btn = egui::Button::new(RichText::new(*t).font(if sel { font_semi(11.5) } else { font_ui(11.5) }).color(p.text))
                .fill(if sel { tint(ink, 70) } else { tint(p.line, 30) })
                .stroke(Stroke::new(1.0, tint(ink, if sel { 200 } else { 90 })))
                .rounding(Rounding::same(10.0));
            if ui.add(btn).clicked() {
                *tab = i;
            }
        }
    });
    ui.add_space(6.0);
    let cur_model = text_of(o, "model");
    let taken = |n: &str| used.iter().any(|u| u == n);
    if *tab == 0 {
        // Presets: pills with a unit tag.
        let mut chosen: Option<&Preset> = None;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(5.0, 5.0);
            for pr in PRESETS {
                let txt = RichText::new(format!("{}  {}", pr.label, pr.unit)).font(font_ui(11.5)).color(p.text);
                let r = ui.add(egui::Button::new(txt).fill(tint(ink, 24)).stroke(Stroke::new(1.0, tint(ink, 100))).rounding(Rounding::same(10.0)));
                if r.on_hover_text(pr.desc).clicked() {
                    chosen = Some(pr);
                }
            }
        });
        if let Some(pr) = chosen {
            apply_preset(o, pr);
            let base = text_of(o, "name");
            o.insert("name".into(), Value::from(unique_name(&base, &taken)));
        }
        ui.add_space(2.0);
        ui.label(RichText::new("Presets fill in a realistic signal — adjust anything after.").font(font_ui(10.5)).color(p.muted));
    } else {
        let cat = TABS[*tab];
        let list: Vec<&ModelUi> = MODELS.iter().filter(|m| m.category == cat).collect();
        let cols = if ui.available_width() > 330.0 { 3 } else { 2 };
        let gap = 6.0;
        let cw = ((ui.available_width() - gap * (cols as f32 - 1.0)) / cols as f32).floor();
        let mut chosen: Option<&ModelUi> = None;
        for row in list.chunks(cols) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for m in row {
                    let (r, resp) = ui.allocate_exact_size(vec2(cw, 62.0), Sense::click());
                    let sel = cur_model == m.id;
                    let fill = if sel { tint(ink, 50) } else if resp.hovered() { tint(ink, 26) } else { tint(p.line, 36) };
                    ui.painter().rect_filled(r, Rounding::same(6.0), fill);
                    ui.painter().rect_stroke(r, Rounding::same(6.0), Stroke::new(1.0, tint(ink, if sel { 190 } else { 55 })));
                    ui.painter().text(pos2(r.min.x + 7.0, r.min.y + 5.0), Align2::LEFT_TOP, m.label, font_semi(11.5), p.text);
                    if let Some((pts, kind)) = card_shape(ui, m.id) {
                        let plot = Rect::from_min_max(pos2(r.min.x + 7.0, r.min.y + 23.0), pos2(r.max.x - 7.0, r.max.y - 6.0));
                        draw_curve(&ui.painter_at(r), plot, &pts, kind, ink, 1.5);
                    }
                    if resp.on_hover_text(m.desc).clicked() {
                        chosen = Some(m);
                    }
                }
            });
            ui.add_space(gap - 2.0);
        }
        if let Some(m) = chosen {
            pick_model(o, m.id);
            // Name the signal after the model until the user types their own.
            let current = name_of(o);
            let auto = current.is_empty() || MODELS.iter().any(|x| current == x.id || current.starts_with(&format!("{}_", x.id))) || PRESETS.iter().any(|pr| pr.params.iter().any(|(k, v)| *k == "name" && *v == current));
            if auto {
                o.insert("name".into(), Value::from(unique_name(m.id, &taken)));
            }
        }
    }
    let _ = port_key;
}

// ---------------------------------------------------------------------------
// Panel extras
// ---------------------------------------------------------------------------

/// "Signal types" reference on the Generator panel: every family with its models.
pub fn types_reference(ui: &mut Ui, p: &ThemePalette) {
    let ink = if p.is_light { ExtProto::Gen.ink_deep() } else { ExtProto::Gen.ink_core() };
    ext_proto::section(ui, "SIGNAL TYPES", p);
    ui.label(RichText::new("Add signals on a hub Input node set to Generator — pick a type or a real-world preset and the shape previews as you edit.").font(font_ui(11.5)).color(p.muted));
    ui.add_space(6.0);
    for cat in &TABS[1..] {
        ui.label(RichText::new(cat.to_ascii_uppercase()).font(font_ui(10.0)).color(ink));
        for m in MODELS.iter().filter(|m| m.category == *cat) {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(m.label).font(font_semi(12.0)).color(p.text));
                ui.label(RichText::new(m.desc).font(font_ui(11.5)).color(p.muted));
            });
        }
        ui.add_space(5.0);
    }
    ui.label(RichText::new("WHEN A SIGNAL GENERATES").font(font_ui(10.0)).color(ink));
    for (a, b) in [
        ("Regular interval", "a new value every N seconds."),
        ("Random timer", "the next value arrives after a random wait between two limits — irregular reporters."),
        ("Bursts", "a quick run of values, then a random quiet spell — batch dumps and event storms."),
    ] {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(a).font(font_semi(12.0)).color(p.text));
            ui.label(RichText::new(b).font(font_ui(11.5)).color(p.muted));
        });
    }
}

/// A generator needs a board; make one when a Generator node exists without any, and point every
/// Generator Input at the first board when it has none. Returns true when something changed.
pub fn ensure_board(hubs: &mut crate::hubs::HubModel, ext: &mut ext_proto::ExtPanels) -> bool {
    use crate::hubs::HubProto;
    let uses_gen = hubs.hubs.iter().any(|h| h.sources.iter().any(|s| s.protocol == HubProto::Gen));
    if !uses_gen {
        return false;
    }
    let st = ext.get_mut(ExtProto::Gen);
    let mut changed = false;
    if st.conns.is_empty() {
        let c = st.new_conn();
        st.conns.push(c);
        st.selected = Some(0);
        changed = true;
    }
    let first = text_of(&st.conns[0], "id");
    for h in &mut hubs.hubs {
        for s in h.sources.iter_mut().filter(|s| s.protocol == HubProto::Gen) {
            if s.ext_connection.trim().is_empty() {
                s.ext_connection = first.clone();
                changed = true;
            }
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_catalog_matches_the_signal_engine() {
        // Every engine model has a card, in the same family and order; nothing extra.
        let engine: Vec<(&str, &str)> = pb_signal::catalog().iter().map(|m| (m.id, m.category)).collect();
        let ui: Vec<(&str, &str)> = MODELS.iter().map(|m| (m.id, m.category)).collect();
        assert_eq!(engine, ui);
        // …and the Type dropdown offers exactly those models.
        let dropdown: Vec<&str> = ext_proto::GEN_MODELS.iter().map(|(k, _)| *k).collect();
        assert_eq!(dropdown, MODELS.iter().map(|m| m.id).collect::<Vec<_>>());
        // Tabs are Real-world plus the families that exist.
        for m in MODELS {
            assert!(TABS.contains(&m.category), "{}", m.category);
        }
        for t in &TABS[1..] {
            assert!(MODELS.iter().any(|m| m.category == *t), "empty tab {t}");
        }
    }

    #[test]
    fn field_groups_cover_the_schema_exactly() {
        let f = fields();
        assert_eq!(f[0].key, "name");
        assert_eq!(f[1].key, "model");
        assert_eq!(f[GEN_SHAPE].first().map(|x| x.key), Some("value"));
        assert_eq!(f[GEN_SHAPE].last().map(|x| x.key), Some("expr"));
        assert_eq!(f[GEN_TRIGGER].first().map(|x| x.key), Some("trigger"));
        assert_eq!(f[GEN_TRIGGER].last().map(|x| x.key), Some("idle_max"));
        assert_eq!(f[GEN_OUT].first().map(|x| x.key), Some("decimals"));
        assert_eq!(GEN_OUT.end, f.len(), "no field falls outside a group");
        let mut keys = std::collections::HashSet::new();
        for fd in f {
            assert!(keys.insert(fd.key), "duplicate field {}", fd.key);
        }
    }

    #[test]
    fn every_field_the_signal_engine_reads_exists_in_the_form() {
        // Keys read by pb-signal; a typo on either side would silently ignore the setting.
        let read = [
            "name", "model", "value", "low", "high", "period", "phase", "duty", "start", "rate", "edge", "mean", "sigma", "reversion", "step", "wrap", "hold", "tau", "noise", "on_secs",
            "off_secs", "prob", "values", "loop", "expr", "trigger", "every", "every_min", "every_max", "burst_count", "burst_gap", "idle_min", "idle_max", "decimals",
        ];
        let have: Vec<&str> = fields().iter().map(|f| f.key).collect();
        for k in read {
            assert!(have.contains(&k), "form lacks {k}");
        }
        assert_eq!(have.len(), read.len(), "form has a field the engine never reads");
    }

    #[test]
    fn every_model_shows_only_its_own_parameters() {
        let shown = |model: &str| -> Vec<&'static str> {
            let mut o = new_draft();
            pick_model(&mut o, model);
            fields()[GEN_SHAPE].iter().filter(|fd| ext_proto::is_visible(&o, fd, fields())).map(|fd| fd.key).collect()
        };
        assert_eq!(shown("sine"), ["low", "high", "period", "phase", "noise"]);
        assert_eq!(shown("square"), ["low", "high", "period", "duty", "noise"]);
        assert_eq!(shown("walk"), ["low", "high", "start", "step"]);
        assert_eq!(shown("drift"), ["mean", "sigma", "reversion"]);
        assert_eq!(shown("lag"), ["low", "high", "hold", "tau", "noise"]);
        assert_eq!(shown("cycle"), ["on_secs", "off_secs"]);
        assert_eq!(shown("counter"), ["start", "step", "wrap"]);
        assert_eq!(shown("formula"), ["expr"]);
        assert!(shown("toggle").is_empty(), "nothing to configure on a toggle");
        // Trigger fields follow the trigger kind.
        let trig = |kind: &str| -> Vec<&'static str> {
            let mut o = new_draft();
            o.insert("trigger".into(), Value::from(kind));
            fields()[GEN_TRIGGER].iter().filter(|fd| ext_proto::is_visible(&o, fd, fields())).map(|fd| fd.key).collect()
        };
        assert_eq!(trig("interval"), ["trigger", "every"]);
        assert_eq!(trig("random"), ["trigger", "every_min", "every_max"]);
        assert_eq!(trig("burst"), ["trigger", "burst_count", "burst_gap", "idle_min", "idle_max"]);
    }

    #[test]
    fn every_default_model_draft_previews_cleanly() {
        for m in MODELS {
            let mut o = new_draft();
            pick_model(&mut o, m.id);
            o.insert("name".into(), Value::from("x"));
            let pv = pb_signal::preview(&o).unwrap_or_else(|e| panic!("{}: {e}", m.id));
            assert!(pv.points.len() >= 24, "{}", m.id);
            assert!(pv.points.iter().all(|(_, v)| v.is_finite()), "{}", m.id);
        }
    }

    #[test]
    fn every_preset_is_valid_and_previews() {
        let mut names = std::collections::HashSet::new();
        for pr in PRESETS {
            let mut o = new_draft();
            apply_preset(&mut o, pr);
            assert!(model_ui(&text_of(&o, "model")).is_some(), "{}: model", pr.label);
            assert!(!name_of(&o).is_empty() && names.insert(name_of(&o)), "{}: name", pr.label);
            for (k, _) in pr.params {
                assert!(fields().iter().any(|f| f.key == *k), "{}: unknown key {k}", pr.label);
            }
            // The bind is something the pack accepts and the preview can draw.
            let pv = pb_signal::preview(&o).unwrap_or_else(|e| panic!("{}: {e}", pr.label));
            assert!(pv.points.iter().all(|(_, v)| v.is_finite()), "{}", pr.label);
        }
        assert!(PRESETS.len() >= 12);
    }

    #[test]
    fn presets_stay_in_their_stated_ranges() {
        let by = |label: &str| PRESETS.iter().find(|p| p.label == label).unwrap();
        let range = |label: &str| {
            let mut o = new_draft();
            apply_preset(&mut o, by(label));
            let pv = pb_signal::preview(&o).unwrap();
            let lo = pv.points.iter().map(|p| p.1).fold(f64::MAX, f64::min);
            let hi = pv.points.iter().map(|p| p.1).fold(f64::MIN, f64::max);
            (lo, hi)
        };
        let (lo, hi) = range("Tank level");
        assert!(lo > 15.0 && hi < 90.0 && hi - lo > 50.0, "{lo} {hi}");
        let (lo, hi) = range("Discharge pressure");
        assert!(lo > 370.0 && hi < 430.0, "{lo} {hi}");
        let (lo, hi) = range("Battery voltage");
        assert!(lo >= 11.7 && hi <= 13.9, "{lo} {hi}");
        let (lo, hi) = range("Pump running");
        assert_eq!((lo, hi), (0.0, 1.0));
    }

    #[test]
    fn typed_values_follow_the_field_kind() {
        assert_eq!(typed("period", "180"), serde_json::json!(180.0));
        assert_eq!(typed("burst_count", "6"), serde_json::json!(6));
        assert_eq!(typed("decimals", "2"), serde_json::json!(2));
        assert_eq!(typed("loop", "true"), serde_json::json!(true));
        assert_eq!(typed("model", "sine"), serde_json::json!("sine"));
        assert_eq!(typed("values", "1, 2"), serde_json::json!("1, 2"));
    }

    #[test]
    fn picking_a_model_resets_parameters_but_keeps_name_and_trigger() {
        let mut o = new_draft();
        o.insert("name".into(), Value::from("keep_me"));
        o.insert("trigger".into(), Value::from("random"));
        o.insert("high".into(), Value::from(9999));
        pick_model(&mut o, "noise");
        assert_eq!(text_of(&o, "name"), "keep_me");
        assert_eq!(text_of(&o, "trigger"), "random");
        assert_eq!(text_of(&o, "model"), "noise");
        assert_eq!(o["high"], serde_json::json!(100.0), "shape params are back to defaults");
        // A preset also resets the trigger unless it sets one.
        let alarm = PRESETS.iter().find(|p| p.label == "Random alarm").unwrap();
        apply_preset(&mut o, alarm);
        assert_eq!(text_of(&o, "trigger"), "random");
        let temp = PRESETS.iter().find(|p| p.label == "Temperature").unwrap();
        apply_preset(&mut o, temp);
        assert_eq!(text_of(&o, "trigger"), "interval");
    }

    #[test]
    fn names_are_made_unique() {
        let used = |n: &str| ["sine", "sine_2"].contains(&n);
        assert_eq!(unique_name("sine", &used), "sine_3");
        assert_eq!(unique_name("walk", &used), "walk");
        assert_eq!(unique_name("  ", &used), "signal");
    }

    #[test]
    fn a_bind_built_by_the_form_is_what_the_pack_runs() {
        // End to end through the real signal engine: draft → Signal → values.
        let mut o = new_draft();
        let pr = PRESETS.iter().find(|p| p.label == "Heartbeat").unwrap();
        apply_preset(&mut o, pr);
        let bind = pb_extkit::Bind { fields: o.clone(), point: "p".into(), source: "gen/b::heartbeat".into(), interval: std::time::Duration::from_secs(1) };
        let mut s = pb_signal::Signal::from_bind(&bind, 1, 0).unwrap();
        let v: Vec<_> = (0..3).map(|i| s.sample(i as f64, 1.0, &|_| None).unwrap()).collect();
        assert_eq!(v, [pb_extkit::Val::Int(0), pb_extkit::Val::Int(1), pb_extkit::Val::Int(2)]);
    }
}
