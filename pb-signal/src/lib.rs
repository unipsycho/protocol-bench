//! Signal models and triggers (pure; no clock, no I/O).
//!
//! Shared by the Generator pack (`pb-gen`, which owns the clock) and the UI (which previews a
//! signal's shape while you edit it). Lives in a plain rlib because a pack crate cannot be linked
//! into the UI binary (see docs/pack-development.md, "One pack per binary").

use pb_expr::{compile, Expr, Rng};
use pb_extkit::{Bind, Val};

/// How a model's value is typed on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Float,
    Int,
    Bool,
}

/// One entry of the model catalog (the UI mirrors this grouping).
#[derive(Debug, Clone, Copy)]
pub struct ModelInfo {
    pub id: &'static str,
    pub category: &'static str,
    pub kind: Kind,
}

/// Every model id with its category, in display order.
pub const CATALOG: &[ModelInfo] = &[
    ModelInfo { id: "constant", category: "Waveform", kind: Kind::Float },
    ModelInfo { id: "sine", category: "Waveform", kind: Kind::Float },
    ModelInfo { id: "square", category: "Waveform", kind: Kind::Float },
    ModelInfo { id: "triangle", category: "Waveform", kind: Kind::Float },
    ModelInfo { id: "sawtooth", category: "Waveform", kind: Kind::Float },
    ModelInfo { id: "ramp", category: "Waveform", kind: Kind::Float },
    ModelInfo { id: "noise", category: "Random", kind: Kind::Float },
    ModelInfo { id: "uniform", category: "Random", kind: Kind::Float },
    ModelInfo { id: "walk", category: "Random", kind: Kind::Float },
    ModelInfo { id: "drift", category: "Random", kind: Kind::Float },
    ModelInfo { id: "lag", category: "Process", kind: Kind::Float },
    ModelInfo { id: "sequence", category: "Process", kind: Kind::Float },
    ModelInfo { id: "cycle", category: "Digital", kind: Kind::Bool },
    ModelInfo { id: "toggle", category: "Digital", kind: Kind::Bool },
    ModelInfo { id: "flip", category: "Digital", kind: Kind::Bool },
    ModelInfo { id: "counter", category: "Counter", kind: Kind::Int },
    ModelInfo { id: "totaliser", category: "Counter", kind: Kind::Float },
    ModelInfo { id: "formula", category: "Formula", kind: Kind::Float },
];

pub fn catalog() -> &'static [ModelInfo] {
    CATALOG
}

// ---------------------------------------------------------------------------
// Triggers
// ---------------------------------------------------------------------------

/// When a signal produces a value.
#[derive(Debug, Clone, PartialEq)]
pub enum Trigger {
    /// Every `secs`.
    Interval { secs: f64 },
    /// A random wait in `[min, max]` seconds after each value (irregular reporters).
    Random { min: f64, max: f64 },
    /// `count` values `gap` seconds apart, then idle for a random `[idle_min, idle_max]`.
    Burst { count: u32, gap: f64, idle_min: f64, idle_max: f64 },
}

impl Trigger {
    pub fn from_bind(b: &Bind) -> Trigger {
        let pos = |k: &str, d: f64| b.num(k).filter(|v| v.is_finite() && *v > 0.0).unwrap_or(d);
        match b.text("trigger").as_str() {
            "random" => {
                let (a, c) = (pos("every_min", 0.5), pos("every_max", 5.0));
                Trigger::Random { min: a.min(c), max: a.max(c) }
            }
            "burst" => {
                let (a, c) = (pos("idle_min", 5.0), pos("idle_max", 15.0));
                Trigger::Burst { count: pos("burst_count", 5.0).round().clamp(1.0, 10_000.0) as u32, gap: pos("burst_gap", 0.1), idle_min: a.min(c), idle_max: a.max(c) }
            }
            _ => Trigger::Interval { secs: pos("every", 1.0) },
        }
    }

    /// A short human description (panel rows).
    pub fn describe(&self) -> String {
        match self {
            Trigger::Interval { secs } => format!("every {}", fmt_secs(*secs)),
            Trigger::Random { min, max } => format!("random {}–{}", fmt_secs(*min), fmt_secs(*max)),
            Trigger::Burst { count, gap, .. } => format!("bursts of {count} @ {}", fmt_secs(*gap)),
        }
    }
}

fn fmt_secs(s: f64) -> String {
    if s < 1.0 {
        format!("{:.0} ms", s * 1000.0)
    } else if s < 120.0 {
        format!("{} s", trim(s))
    } else {
        format!("{} min", trim(s / 60.0))
    }
}

fn trim(v: f64) -> String {
    let t = format!("{v:.2}");
    t.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Per-signal schedule state.
#[derive(Debug, Clone)]
pub struct Schedule {
    trigger: Trigger,
    /// Values still to come in the current burst.
    left: u32,
}

impl Schedule {
    pub fn new(trigger: Trigger) -> Self {
        let left = match &trigger {
            Trigger::Burst { count, .. } => count.saturating_sub(1),
            _ => 0,
        };
        Self { trigger, left }
    }

    pub fn describe(&self) -> String {
        self.trigger.describe()
    }

    /// Seconds to wait after a value was produced until the next one.
    pub fn next_delay(&mut self, rng: &mut Rng) -> f64 {
        match &self.trigger {
            Trigger::Interval { secs } => *secs,
            Trigger::Random { min, max } => rng.range(*min, *max),
            Trigger::Burst { count, gap, idle_min, idle_max } => {
                if self.left > 0 {
                    self.left -= 1;
                    *gap
                } else {
                    self.left = count.saturating_sub(1);
                    rng.range(*idle_min, *idle_max)
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Models
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Hold,
    Wrap,
    Bounce,
}

#[derive(Debug, Clone)]
enum Model {
    Constant { value: f64 },
    Sine { low: f64, high: f64, period: f64, phase: f64, noise: f64 },
    Square { low: f64, high: f64, period: f64, duty: f64, noise: f64 },
    Triangle { low: f64, high: f64, period: f64, duty: f64, noise: f64 },
    Sawtooth { low: f64, high: f64, period: f64, noise: f64 },
    Ramp { start: f64, rate: f64, low: f64, high: f64, edge: Edge, noise: f64 },
    Noise { mean: f64, sigma: f64 },
    Uniform { low: f64, high: f64 },
    Walk { start: f64, step: f64, low: f64, high: f64 },
    Drift { mean: f64, sigma: f64, reversion: f64 },
    Lag { low: f64, high: f64, hold: f64, tau: f64, noise: f64 },
    Sequence { values: Vec<f64>, looped: bool },
    Cycle { on_secs: f64, off_secs: f64 },
    Toggle,
    Flip { prob: f64 },
    Counter { start: i64, step: i64, wrap: i64 },
    Totaliser { start: f64, rate: f64, noise: f64 },
    Formula { expr: Expr },
}

/// Mutable state a model carries between values.
#[derive(Debug, Clone, Default)]
struct State {
    /// Last numeric output (random walk / drift position, counter, totaliser, `prev`).
    last: f64,
    started: bool,
    fired: u64,
    /// `lag`: current setpoint and when it next changes.
    setpoint: f64,
    next_setpoint_at: f64,
    flag: bool,
    index: usize,
}

/// One configured signal.
#[derive(Debug)]
pub struct Signal {
    pub name: String,
    pub model_id: String,
    pub kind: Kind,
    pub decimals: u32,
    model: Model,
    st: State,
    pub schedule: Schedule,
    pub rng: Rng,
    /// How much model time shows the signal's character (two cycles of a waveform), if it has one.
    pub natural_span: Option<f64>,
}

/// Names a formula may use besides other signals.
pub const BUILTIN_VARS: &[&str] = &["t", "dt", "prev", "n"];

fn parse_list(s: &str) -> Vec<f64> {
    s.split([',', ';', ' ', '\n', '\t']).filter_map(|x| x.trim().parse::<f64>().ok()).filter(|v| v.is_finite()).collect()
}

impl Signal {
    /// Build a signal from a bind. `seed` 0 = clock-seeded; otherwise each signal gets its own
    /// stream derived from the board seed and its name, so a seeded board repeats per signal.
    pub fn from_bind(b: &Bind, seed: u64, index: usize) -> Result<Signal, String> {
        let name = b.text("name").trim().to_string();
        if name.is_empty() {
            return Err("signal has no name".into());
        }
        let n = |k: &str, d: f64| b.num(k).filter(|v| v.is_finite()).unwrap_or(d);
        let (low, high) = (n("low", 0.0), n("high", 100.0));
        let period = n("period", 60.0).max(1e-3);
        let noise = n("noise", 0.0).abs();
        let duty = (n("duty", 50.0) / 100.0).clamp(0.01, 0.99);
        let id = {
            let m = b.text("model");
            if m.is_empty() { "sine".to_string() } else { m }
        };
        let model = match id.as_str() {
            "constant" => Model::Constant { value: n("value", 0.0) },
            "sine" => Model::Sine { low, high, period, phase: n("phase", 0.0), noise },
            "square" => Model::Square { low, high, period, duty, noise },
            "triangle" => Model::Triangle { low, high, period, duty, noise },
            "sawtooth" => Model::Sawtooth { low, high, period, noise },
            "ramp" => Model::Ramp {
                start: n("start", low),
                rate: n("rate", 1.0),
                low,
                high,
                edge: match b.text("edge").as_str() {
                    "wrap" => Edge::Wrap,
                    "bounce" => Edge::Bounce,
                    _ => Edge::Hold,
                },
                noise,
            },
            "noise" => Model::Noise { mean: n("mean", 50.0), sigma: n("sigma", 1.0).abs() },
            "uniform" => Model::Uniform { low, high },
            "walk" => Model::Walk { start: n("start", (low + high) / 2.0), step: n("step", 1.0).abs(), low, high },
            "drift" => Model::Drift { mean: n("mean", 50.0), sigma: n("sigma", 1.0).abs(), reversion: n("reversion", 0.1).max(0.0) },
            "lag" => Model::Lag { low, high, hold: n("hold", 30.0).max(1e-3), tau: n("tau", 5.0).max(1e-3), noise },
            "sequence" => {
                let values = parse_list(&b.text("values"));
                if values.is_empty() {
                    return Err("sequence needs a list of numbers, e.g. 10, 20, 15".into());
                }
                Model::Sequence { values, looped: b.fields.get("loop").map(|_| b.flag("loop")).unwrap_or(true) }
            }
            "cycle" => Model::Cycle { on_secs: n("on_secs", 30.0).max(1e-3), off_secs: n("off_secs", 30.0).max(1e-3) },
            "toggle" => Model::Toggle,
            "flip" => Model::Flip { prob: (n("prob", 50.0) / 100.0).clamp(0.0, 1.0) },
            "counter" => Model::Counter { start: n("start", 0.0) as i64, step: n("step", 1.0) as i64, wrap: n("wrap", 0.0) as i64 },
            "totaliser" => Model::Totaliser { start: n("start", 0.0), rate: n("rate", 1.0), noise },
            "formula" => {
                let src = b.text("expr");
                Model::Formula { expr: compile(&src).map_err(|e| format!("formula: {e}"))? }
            }
            other => return Err(format!("unknown model '{other}'")),
        };
        let kind = CATALOG.iter().find(|m| m.id == id).map(|m| m.kind).unwrap_or(Kind::Float);
        let decimals = b.num("decimals").filter(|d| *d >= 0.0).map(|d| d.min(9.0) as u32).unwrap_or(3);
        let trigger = Trigger::from_bind(b);
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in name.bytes() {
            h = (h ^ byte as u64).wrapping_mul(0x0100_0000_01b3);
        }
        let rng = if seed == 0 { Rng::new(Rng::from_clock().next_u64() ^ (index as u64).wrapping_mul(0x9E37_79B9)) } else { Rng::new(seed ^ h) };
        let mut st = State::default();
        match &model {
            Model::Walk { start, .. } | Model::Totaliser { start, .. } => st.last = *start,
            Model::Ramp { start, .. } => st.last = *start,
            Model::Drift { mean, .. } => st.last = *mean,
            Model::Counter { start, .. } => st.last = *start as f64,
            Model::Lag { low, high, .. } => {
                st.last = (low + high) / 2.0;
                st.setpoint = st.last;
            }
            _ => {}
        }
        let natural_span = match &model {
            Model::Sine { period, .. } | Model::Square { period, .. } | Model::Triangle { period, .. } | Model::Sawtooth { period, .. } => Some(period * 2.0),
            Model::Lag { hold, .. } => Some(hold * 3.0),
            Model::Cycle { on_secs, off_secs } => Some((on_secs + off_secs) * 2.0),
            Model::Ramp { rate, low, high, edge, .. } if *rate != 0.0 => {
                let sweep = (high - low).abs() / rate.abs();
                Some(if matches!(edge, Edge::Hold) { sweep * 1.5 } else { sweep * 2.5 })
            }
            _ => None,
        };
        Ok(Signal { name, model_id: id, kind, decimals, model, st, schedule: Schedule::new(trigger), rng, natural_span })
    }

    /// Variable names a formula signal reads (empty for other models).
    pub fn formula_vars(&self) -> Vec<String> {
        match &self.model {
            Model::Formula { expr } => expr.variables().to_vec(),
            _ => Vec::new(),
        }
    }

    /// Produce the next value. `t` is model time since the board started, `dt` model time since
    /// this signal's previous value (0 for the first). `vars` resolves other signals' last values.
    pub fn sample(&mut self, t: f64, dt: f64, vars: &dyn Fn(&str) -> Option<f64>) -> Result<Val, String> {
        let first = !self.st.started;
        self.st.started = true;
        let rng = &mut self.rng;
        let st = &mut self.st;
        let v: f64 = match &self.model {
            Model::Constant { value } => *value,
            Model::Sine { low, high, period, phase, noise } => {
                let mid = (low + high) / 2.0;
                let amp = (high - low) / 2.0;
                mid + amp * (std::f64::consts::TAU * t / period + phase.to_radians()).sin() + rng.gauss() * noise
            }
            Model::Square { low, high, period, duty, noise } => {
                let x = (t / period).fract();
                (if x < *duty { *high } else { *low }) + rng.gauss() * noise
            }
            Model::Triangle { low, high, period, duty, noise } => {
                let x = (t / period).fract();
                let f = if x < *duty { x / duty } else { 1.0 - (x - duty) / (1.0 - duty) };
                low + (high - low) * f + rng.gauss() * noise
            }
            Model::Sawtooth { low, high, period, noise } => low + (high - low) * (t / period).fract() + rng.gauss() * noise,
            Model::Ramp { rate, low, high, edge, noise, .. } => {
                let (lo, hi) = (low.min(*high), low.max(*high));
                let span = (hi - lo).max(1e-12);
                if !first {
                    st.last += rate * dt;
                }
                let out = match edge {
                    Edge::Hold => {
                        st.last = st.last.clamp(lo, hi);
                        st.last
                    }
                    Edge::Wrap => {
                        st.last = lo + (st.last - lo).rem_euclid(span);
                        st.last
                    }
                    Edge::Bounce => {
                        // Keep the unfolded position bounded (mod 2·span) and fold it for output.
                        st.last = lo + (st.last - lo).rem_euclid(2.0 * span);
                        let m = st.last - lo;
                        lo + if m <= span { m } else { 2.0 * span - m }
                    }
                };
                out + rng.gauss() * noise
            }
            Model::Noise { mean, sigma } => mean + rng.gauss() * sigma,
            Model::Uniform { low, high } => rng.range(*low, *high),
            Model::Walk { step, low, high, .. } => {
                if !first {
                    st.last += rng.gauss() * step;
                }
                let (lo, hi) = (low.min(*high), low.max(*high));
                st.last = st.last.clamp(lo, hi);
                st.last
            }
            Model::Drift { mean, sigma, reversion } => {
                if !first {
                    let k = (reversion * dt).clamp(0.0, 1.0);
                    st.last += k * (mean - st.last) + sigma * dt.max(0.0).sqrt() * rng.gauss();
                }
                st.last
            }
            Model::Lag { low, high, hold, tau, noise } => {
                if first || t >= st.next_setpoint_at {
                    st.setpoint = rng.range(*low, *high);
                    st.next_setpoint_at = t + hold;
                }
                if !first {
                    // Exact first-order response, stable for any dt.
                    let a = 1.0 - (-dt / tau).exp();
                    st.last += a * (st.setpoint - st.last);
                }
                st.last + rng.gauss() * noise
            }
            Model::Sequence { values, looped } => {
                if !first {
                    if st.index + 1 < values.len() {
                        st.index += 1;
                    } else if *looped {
                        st.index = 0;
                    }
                }
                values[st.index.min(values.len() - 1)]
            }
            Model::Cycle { on_secs, off_secs } => {
                let x = t.rem_euclid(on_secs + off_secs);
                return Ok(Val::Bool(x < *on_secs));
            }
            Model::Toggle => {
                if !first {
                    st.flag = !st.flag;
                }
                return Ok(Val::Bool(st.flag));
            }
            Model::Flip { prob } => return Ok(Val::Bool(rng.uniform() < *prob)),
            Model::Counter { step, wrap, .. } => {
                if !first {
                    st.last += *step as f64;
                    if *wrap > 0 && st.last >= *wrap as f64 {
                        st.last -= *wrap as f64;
                    }
                }
                return Ok(Val::Int(st.last as i64));
            }
            Model::Totaliser { rate, noise, .. } => {
                if !first {
                    st.last += (rate + rng.gauss() * noise) * dt;
                }
                st.last
            }
            Model::Formula { expr } => {
                let (last, fired) = (st.last, st.fired as f64);
                let look = |name: &str| match name {
                    "t" => Some(t),
                    "dt" => Some(dt),
                    "prev" => Some(last),
                    "n" => Some(fired),
                    other => vars(other),
                };
                let out = expr.eval(&look, rng)?;
                st.last = out;
                out
            }
        };
        st.fired += 1;
        if !v.is_finite() {
            return Err("value is not finite".into());
        }
        Ok(Val::Float(round_to(v, self.decimals)))
    }
}

/// A simulated run of one signal, for drawing its shape while it is being edited.
#[derive(Debug, Clone, PartialEq)]
pub struct Preview {
    /// `(model seconds, value)`; bools are 0 / 1.
    pub points: Vec<(f64, f64)>,
    pub span_secs: f64,
    pub kind: Kind,
}

/// Trigger description for a bind object (`every 1 s`, `random 500 ms–5 s`, …).
pub fn describe_trigger(fields: &serde_json::Map<String, serde_json::Value>) -> String {
    let bind = Bind { fields: fields.clone(), point: String::new(), source: String::new(), interval: std::time::Duration::from_secs(1) };
    Trigger::from_bind(&bind).describe()
}

/// Simulate `fields` (a bind object) offline with a fixed seed. Formulas see every other name as 0.
pub fn preview(fields: &serde_json::Map<String, serde_json::Value>) -> Result<Preview, String> {
    let mut f = fields.clone();
    if f.get("name").and_then(|v| v.as_str()).is_none_or(|n| n.trim().is_empty()) {
        f.insert("name".into(), "preview".into());
    }
    let bind = Bind { fields: f, point: String::new(), source: String::new(), interval: std::time::Duration::from_secs(1) };
    let mut sig = Signal::from_bind(&bind, 7, 0)?;
    let zero = |_: &str| Some(0.0);
    let mut points = Vec::new();
    let (mut t, mut last) = (0.0f64, 0.0f64);
    let span = match sig.natural_span {
        Some(span) => {
            let n = 160;
            let dt = span / n as f64;
            for i in 0..=n {
                t = dt * i as f64;
                let v = sig.sample(t, if i == 0 { 0.0 } else { dt }, &zero)?;
                points.push((t, v.as_f64().unwrap_or(0.0)));
            }
            span
        }
        None => {
            let n = match sig.model_id.as_str() {
                "toggle" => 24,
                "flip" => 48,
                "counter" | "sequence" => 40,
                _ => 120,
            };
            for i in 0..n {
                let v = sig.sample(t, t - last, &zero)?;
                points.push((t, v.as_f64().unwrap_or(0.0)));
                last = t;
                t += sig.schedule.next_delay(&mut sig.rng);
                let _ = i;
            }
            last
        }
    };
    Ok(Preview { points, span_secs: span, kind: sig.kind })
}

pub fn round_to(v: f64, decimals: u32) -> f64 {
    let m = 10f64.powi(decimals as i32);
    (v * m).round() / m
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Map, Value};
    use std::time::Duration;

    fn bind(v: Value) -> Bind {
        let fields: Map<String, Value> = v.as_object().unwrap().clone();
        Bind { fields, point: "p".into(), source: "s".into(), interval: Duration::from_secs(1) }
    }

    fn sig(v: Value) -> Signal {
        let mut o = v.as_object().unwrap().clone();
        o.entry("name").or_insert(json!("x"));
        Signal::from_bind(&bind(Value::Object(o)), 42, 0).unwrap()
    }

    fn none(_: &str) -> Option<f64> {
        None
    }

    /// Sample `n` values `dt` apart starting at t = 0.
    fn run(s: &mut Signal, n: usize, dt: f64) -> Vec<f64> {
        (0..n).map(|i| s.sample(i as f64 * dt, if i == 0 { 0.0 } else { dt }, &none).unwrap().as_f64().unwrap()).collect()
    }

    #[test]
    fn every_catalog_model_builds_from_its_defaults() {
        for m in catalog() {
            let mut o = json!({"model": m.id, "name": "x"});
            // Models with a mandatory text parameter.
            match m.id {
                "sequence" => o["values"] = json!("1,2,3"),
                "formula" => o["expr"] = json!("t + 1"),
                _ => {}
            }
            let mut s = Signal::from_bind(&bind(o), 1, 0).unwrap_or_else(|e| panic!("{}: {e}", m.id));
            let v = s.sample(0.0, 0.0, &none).unwrap_or_else(|e| panic!("{}: {e}", m.id));
            let ok = match (m.kind, &v) {
                (Kind::Float, Val::Float(_)) | (Kind::Int, Val::Int(_)) | (Kind::Bool, Val::Bool(_)) => true,
                _ => false,
            };
            assert!(ok, "{} produced {v:?}", m.id);
        }
        // Categories appear in a stable display order.
        let cats: Vec<&str> = catalog().iter().map(|m| m.category).fold(Vec::new(), |mut a, c| {
            if !a.contains(&c) {
                a.push(c);
            }
            a
        });
        assert_eq!(cats, ["Waveform", "Random", "Process", "Digital", "Counter", "Formula"]);
    }

    #[test]
    fn sine_spans_its_range_and_honours_phase() {
        let mut s = sig(json!({"model":"sine","low":10,"high":30,"period":4,"decimals":6}));
        let v = run(&mut s, 5, 1.0); // t = 0,1,2,3,4
        assert_eq!(v[0], 20.0); // mid at t=0
        assert_eq!(v[1], 30.0); // peak at quarter period
        assert_eq!(v[2], 20.0);
        assert_eq!(v[3], 10.0);
        let mut s = sig(json!({"model":"sine","low":0,"high":2,"period":4,"phase":90,"decimals":6}));
        assert_eq!(run(&mut s, 1, 1.0)[0], 2.0); // phase shifts the peak to t=0
    }

    #[test]
    fn square_and_triangle_respect_duty() {
        let mut s = sig(json!({"model":"square","low":0,"high":1,"period":10,"duty":30}));
        let v = run(&mut s, 10, 1.0);
        assert_eq!(v.iter().filter(|x| **x == 1.0).count(), 3);
        let mut t = sig(json!({"model":"triangle","low":0,"high":10,"period":10,"duty":70,"decimals":6}));
        let v = run(&mut t, 10, 1.0);
        assert_eq!(v[0], 0.0);
        assert!((v[7] - 10.0).abs() < 1e-9, "peak at 70 % of the period: {v:?}");
        assert!((v[9] - (10.0 / 3.0)).abs() < 0.01, "{v:?}");
        let mut w = sig(json!({"model":"sawtooth","low":0,"high":10,"period":10,"decimals":6}));
        let v = run(&mut w, 11, 1.0);
        assert_eq!(v[5], 5.0);
        assert_eq!(v[10], 0.0, "wraps back at the period");
    }

    #[test]
    fn ramp_edges_hold_wrap_and_bounce() {
        let mut hold = sig(json!({"model":"ramp","start":0,"rate":2,"low":0,"high":5,"edge":"hold"}));
        assert_eq!(run(&mut hold, 5, 1.0), [0.0, 2.0, 4.0, 5.0, 5.0]);
        let mut wrap = sig(json!({"model":"ramp","start":0,"rate":2,"low":0,"high":5,"edge":"wrap"}));
        assert_eq!(run(&mut wrap, 5, 1.0), [0.0, 2.0, 4.0, 1.0, 3.0]);
        let mut bounce = sig(json!({"model":"ramp","start":0,"rate":2,"low":0,"high":5,"edge":"bounce"}));
        assert_eq!(run(&mut bounce, 6, 1.0), [0.0, 2.0, 4.0, 4.0, 2.0, 0.0]);
        let mut down = sig(json!({"model":"ramp","start":5,"rate":-1,"low":0,"high":5}));
        assert_eq!(run(&mut down, 3, 1.0), [5.0, 4.0, 3.0]);
    }

    #[test]
    fn random_models_stay_in_bounds_and_start_where_told() {
        let mut w = sig(json!({"model":"walk","start":50,"step":5,"low":40,"high":60}));
        let v = run(&mut w, 2000, 1.0);
        assert_eq!(v[0], 50.0);
        assert!(v.iter().all(|x| (40.0..=60.0).contains(x)));
        assert!(v.iter().any(|x| *x != 50.0), "it actually moves");
        let mut u = sig(json!({"model":"uniform","low":-1,"high":1}));
        assert!(run(&mut u, 500, 1.0).iter().all(|x| (-1.0..=1.0).contains(x)));
        let mut n = sig(json!({"model":"noise","mean":100,"sigma":2}));
        let v = run(&mut n, 4000, 1.0);
        let mean = v.iter().sum::<f64>() / v.len() as f64;
        assert!((mean - 100.0).abs() < 0.2, "mean {mean}");
    }

    #[test]
    fn drift_is_pulled_back_to_its_mean() {
        let mut d = sig(json!({"model":"drift","mean":20,"sigma":0.5,"reversion":0.5}));
        let v = run(&mut d, 3000, 1.0);
        let mean = v.iter().sum::<f64>() / v.len() as f64;
        assert!((mean - 20.0).abs() < 1.5, "mean {mean}");
        assert!(v.iter().all(|x| (10.0..30.0).contains(x)), "stays near the mean");
        // No reversion = a plain random walk that wanders much further.
        let mut free = sig(json!({"model":"drift","mean":20,"sigma":2,"reversion":0}));
        let v = run(&mut free, 3000, 1.0);
        assert!(v.iter().any(|x| (x - 20.0).abs() > 15.0));
    }

    #[test]
    fn lag_chases_a_wandering_setpoint_smoothly() {
        let mut l = sig(json!({"model":"lag","low":0,"high":100,"hold":50,"tau":5,"decimals":6}));
        let v = run(&mut l, 400, 1.0);
        assert!(v.iter().all(|x| (0.0..=100.0).contains(x)));
        // Smooth: no sample jumps more than a fraction of the range (setpoint steps are filtered).
        let max_jump = v.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f64::max);
        assert!(max_jump < 25.0, "max step {max_jump}");
        // Stable even for a huge dt (exact exponential, not Euler).
        let mut big = sig(json!({"model":"lag","low":0,"high":100,"hold":1,"tau":1}));
        let v = run(&mut big, 50, 1000.0);
        assert!(v.iter().all(|x| x.is_finite() && (0.0..=100.0).contains(x)));
    }

    #[test]
    fn sequence_loops_or_holds() {
        let mut s = sig(json!({"model":"sequence","values":"10, 20 ; 30"}));
        assert_eq!(run(&mut s, 7, 1.0), [10.0, 20.0, 30.0, 10.0, 20.0, 30.0, 10.0]);
        let mut h = sig(json!({"model":"sequence","values":"1,2","loop":false}));
        assert_eq!(run(&mut h, 4, 1.0), [1.0, 2.0, 2.0, 2.0]);
        assert!(Signal::from_bind(&bind(json!({"name":"x","model":"sequence","values":"abc"})), 1, 0).unwrap_err().contains("list of numbers"));
    }

    #[test]
    fn digital_models() {
        let mut c = sig(json!({"model":"cycle","on_secs":2,"off_secs":3}));
        let v: Vec<bool> = (0..10).map(|i| matches!(c.sample(i as f64, 1.0, &none).unwrap(), Val::Bool(true))).collect();
        assert_eq!(v, [true, true, false, false, false, true, true, false, false, false]);
        let mut t = sig(json!({"model":"toggle"}));
        let v: Vec<Val> = (0..4).map(|i| t.sample(i as f64, 1.0, &none).unwrap()).collect();
        assert_eq!(v, [Val::Bool(false), Val::Bool(true), Val::Bool(false), Val::Bool(true)]);
        let mut f = sig(json!({"model":"flip","prob":25}));
        let hits = (0..4000).filter(|i| matches!(f.sample(*i as f64, 1.0, &none).unwrap(), Val::Bool(true))).count();
        assert!((800..1200).contains(&hits), "{hits}");
        let mut never = sig(json!({"model":"flip","prob":0}));
        assert!((0..100).all(|i| never.sample(i as f64, 1.0, &none).unwrap() == Val::Bool(false)));
    }

    #[test]
    fn counters_count_wrap_and_totalise() {
        let mut c = sig(json!({"model":"counter","start":5,"step":3}));
        let v: Vec<Val> = (0..3).map(|i| c.sample(i as f64, 1.0, &none).unwrap()).collect();
        assert_eq!(v, [Val::Int(5), Val::Int(8), Val::Int(11)]);
        let mut w = sig(json!({"model":"counter","start":0,"step":4,"wrap":10}));
        let v: Vec<i64> = (0..6).map(|i| if let Val::Int(x) = w.sample(i as f64, 1.0, &none).unwrap() { x } else { -1 }).collect();
        assert_eq!(v, [0, 4, 8, 2, 6, 0], "rolls over at `wrap`");
        // Totaliser integrates rate × dt, so irregular triggers still total correctly.
        let mut t = sig(json!({"model":"totaliser","start":100,"rate":2}));
        let a = t.sample(0.0, 0.0, &none).unwrap().as_f64().unwrap();
        let b = t.sample(0.5, 0.5, &none).unwrap().as_f64().unwrap();
        let c2 = t.sample(5.5, 5.0, &none).unwrap().as_f64().unwrap();
        assert_eq!((a, b, c2), (100.0, 101.0, 111.0));
    }

    #[test]
    fn formulas_get_t_dt_prev_n_and_other_signals() {
        let mut f = sig(json!({"model":"formula","expr":"prev + dt * 2 + other + n"}));
        let look = |n: &str| (n == "other").then_some(1.0);
        let v: Vec<f64> = [(0.0, 0.0), (1.0, 1.0), (3.0, 2.0)].iter().map(|(t, dt)| f.sample(*t, *dt, &look).unwrap().as_f64().unwrap()).collect();
        // n counts values produced so far: 0, 1, 2.
        assert_eq!(v, [1.0, 1.0 + 2.0 + 1.0 + 1.0, 5.0 + 4.0 + 1.0 + 2.0]);
        assert_eq!(f.formula_vars(), ["prev", "dt", "other", "n"]);
        // A runtime error (division by zero) is reported, not published as a number.
        let mut bad = sig(json!({"model":"formula","expr":"1 / (t - 5)"}));
        assert!(bad.sample(5.0, 1.0, &none).unwrap_err().contains("division by zero"));
        assert!(Signal::from_bind(&bind(json!({"name":"x","model":"formula","expr":"2 +"})), 1, 0).unwrap_err().starts_with("formula:"));
        assert!(Signal::from_bind(&bind(json!({"name":"x","model":"formula"})), 1, 0).is_err());
    }

    #[test]
    fn decimals_round_and_noise_is_added() {
        let mut s = sig(json!({"model":"constant","value":1.23456,"decimals":2}));
        assert_eq!(run(&mut s, 1, 1.0)[0], 1.23);
        let mut n = sig(json!({"model":"sine","low":0,"high":0,"period":10,"noise":1}));
        let v = run(&mut n, 200, 1.0);
        assert!(v.iter().any(|x| x.abs() > 0.3), "noise shows up on a flat waveform");
    }

    #[test]
    fn same_seed_same_signal_different_name_different_stream() {
        let make = |name: &str, seed: u64| Signal::from_bind(&bind(json!({"name":name,"model":"noise","mean":0,"sigma":1})), seed, 0).unwrap();
        let (mut a, mut b, mut c) = (make("a", 7), make("a", 7), make("b", 7));
        let (va, vb, vc) = (run(&mut a, 20, 1.0), run(&mut b, 20, 1.0), run(&mut c, 20, 1.0));
        assert_eq!(va, vb);
        assert_ne!(va, vc, "two signals on one board are independent");
    }

    #[test]
    fn bad_binds_are_refused_with_a_reason() {
        assert!(Signal::from_bind(&bind(json!({"model":"sine"})), 1, 0).unwrap_err().contains("no name"));
        assert!(Signal::from_bind(&bind(json!({"name":"x","model":"warp"})), 1, 0).unwrap_err().contains("unknown model"));
        // A missing model defaults to a sine rather than failing the whole board.
        assert_eq!(Signal::from_bind(&bind(json!({"name":"x"})), 1, 0).unwrap().model_id, "sine");
    }

    #[test]
    fn triggers_parse_and_schedule() {
        let t = |v: Value| Trigger::from_bind(&bind(v));
        assert_eq!(t(json!({})), Trigger::Interval { secs: 1.0 });
        assert_eq!(t(json!({"every":"0.25"})), Trigger::Interval { secs: 0.25 });
        assert_eq!(t(json!({"every":-3})), Trigger::Interval { secs: 1.0 }, "nonsense falls back");
        // Min/max given backwards are swapped.
        assert_eq!(t(json!({"trigger":"random","every_min":9,"every_max":2})), Trigger::Random { min: 2.0, max: 9.0 });
        assert_eq!(t(json!({"trigger":"burst","burst_count":3,"burst_gap":0.5,"idle_min":1,"idle_max":2})), Trigger::Burst { count: 3, gap: 0.5, idle_min: 1.0, idle_max: 2.0 });

        let mut rng = Rng::new(1);
        let mut s = Schedule::new(Trigger::Interval { secs: 2.0 });
        assert!((0..5).all(|_| s.next_delay(&mut rng) == 2.0));
        let mut s = Schedule::new(Trigger::Random { min: 1.0, max: 3.0 });
        let d: Vec<f64> = (0..200).map(|_| s.next_delay(&mut rng)).collect();
        assert!(d.iter().all(|x| (1.0..=3.0).contains(x)));
        assert!(d.iter().cloned().fold(f64::MAX, f64::min) < 1.2 && d.iter().cloned().fold(0.0, f64::max) > 2.8, "uses the whole range");
        // Burst of 3: gap, gap, idle, gap, gap, idle …
        let mut s = Schedule::new(Trigger::Burst { count: 3, gap: 0.1, idle_min: 10.0, idle_max: 20.0 });
        let d: Vec<f64> = (0..6).map(|_| s.next_delay(&mut rng)).collect();
        assert_eq!(&d[..2], &[0.1, 0.1]);
        assert!(d[2] >= 10.0);
        assert_eq!(&d[3..5], &[0.1, 0.1]);
        assert!(d[5] >= 10.0);
        // A burst of 1 is just irregular single values.
        let mut s = Schedule::new(Trigger::Burst { count: 1, gap: 0.1, idle_min: 5.0, idle_max: 5.0 });
        assert!((0..4).all(|_| s.next_delay(&mut rng) == 5.0));
    }

    #[test]
    fn preview_simulates_shapes_over_a_sensible_window() {
        let f = |v: Value| v.as_object().unwrap().clone();
        let p = preview(&f(json!({"model":"sine","low":0,"high":10,"period":60}))).unwrap();
        assert_eq!(p.span_secs, 120.0, "two cycles of a waveform");
        assert_eq!(p.points.len(), 161);
        assert!(p.points.iter().all(|(_, v)| (0.0..=10.0).contains(v)));
        // No natural period: follows the trigger (random-timer gaps show as uneven x steps).
        let p = preview(&f(json!({"model":"walk","trigger":"random","every_min":1,"every_max":3}))).unwrap();
        assert_eq!(p.points.len(), 120);
        // Stepping signals draw fewer, wider steps so a toggle reads as a wave, not a solid block.
        assert_eq!(preview(&f(json!({"model":"toggle"}))).unwrap().points.len(), 24);
        let gaps: Vec<f64> = p.points.windows(2).map(|w| w[1].0 - w[0].0).collect();
        assert!(gaps.iter().all(|g| (1.0..=3.0).contains(g)));
        assert!(gaps.iter().cloned().fold(0.0, f64::max) - gaps.iter().cloned().fold(9.0, f64::min) > 0.5);
        // Digital models plot as 0 / 1; the cycle shows two full on/off periods.
        let p = preview(&f(json!({"model":"cycle","on_secs":10,"off_secs":20}))).unwrap();
        assert!(p.points.iter().all(|(_, v)| *v == 0.0 || *v == 1.0));
        assert_eq!(p.kind, Kind::Bool);
        // Formulas preview even when they name other signals; errors come back as text.
        assert!(preview(&f(json!({"model":"formula","expr":"other * 2 + t"}))).is_ok());
        assert!(preview(&f(json!({"model":"formula","expr":"1 +"}))).is_err());
        // Same input, same picture (fixed seed).
        let a = preview(&f(json!({"model":"noise","mean":5,"sigma":1}))).unwrap();
        assert_eq!(a, preview(&f(json!({"model":"noise","mean":5,"sigma":1}))).unwrap());
    }

    #[test]
    fn trigger_descriptions_read_well() {
        assert_eq!(Trigger::Interval { secs: 0.25 }.describe(), "every 250 ms");
        assert_eq!(Trigger::Interval { secs: 2.0 }.describe(), "every 2 s");
        assert_eq!(Trigger::Interval { secs: 600.0 }.describe(), "every 10 min");
        assert_eq!(Trigger::Random { min: 0.5, max: 5.0 }.describe(), "random 500 ms–5 s");
        assert_eq!(Trigger::Burst { count: 5, gap: 0.1, idle_min: 1.0, idle_max: 2.0 }.describe(), "bursts of 5 @ 100 ms");
    }
}
