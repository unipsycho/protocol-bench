//! Edge calculation transforms applied before Output MQTT egress.
//! UI preview peeks the same runtime so deadband / rolling avg match publish.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// One calculation step synced from the hub diagram.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CalcConfig {
    pub id: u64,
    pub kind: String,
    pub source_point: String,
    pub param_a: String,
    pub param_b: String,
    pub window_secs: String,
    pub formula: String,
}

#[derive(Debug, Clone, Copy)]
struct Sample {
    t_ms: u64,
    v: f64,
}

/// Runtime hold / window state for stateful calcs.
#[derive(Debug, Default)]
pub struct CalcRuntime {
    /// Last deadband output per calculation step id.
    pub deadband_hold: HashMap<u64, f64>,
    /// Rolling-average sample buffers per step id.
    rolling: HashMap<u64, Vec<Sample>>,
}

impl CalcRuntime {
    pub fn clear(&mut self) {
        self.deadband_hold.clear();
        self.rolling.clear();
    }
}

/// Apply deadband: hold until `|x - held| > band`.
/// Returns `(output, new_held)`.
pub fn deadband_step(held: Option<f64>, x: f64, band: f64) -> (f64, f64) {
    let band = band.abs();
    match held {
        None => (x, x),
        Some(h) if (x - h).abs() > band => (x, x),
        Some(h) => (h, h),
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn parse_f64(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    match t.to_ascii_lowercase().as_str() {
        "true" | "on" => Some(1.0),
        "false" | "off" => Some(0.0),
        _ => t.parse().ok(),
    }
}

fn fmt_num(v: f64) -> String {
    if !v.is_finite() {
        return "nan".into();
    }
    if v.fract().abs() < 1e-12 {
        format!("{v:.0}")
    } else {
        let s = format!("{v:.10}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

fn rolling_max_n(param_a: &str) -> Option<usize> {
    let t = param_a.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("auto") {
        return None;
    }
    t.parse::<usize>().ok().filter(|n| *n > 0)
}

/// Push a sample and return the mean over the time window (and optional max N).
fn rolling_avg_step(
    buf: &mut Vec<Sample>,
    x: f64,
    now_ms: u64,
    window_secs: f64,
    max_n: Option<usize>,
) -> f64 {
    buf.push(Sample { t_ms: now_ms, v: x });
    let win_ms = (window_secs.max(0.001) * 1000.0) as u64;
    let cutoff = now_ms.saturating_sub(win_ms);
    buf.retain(|s| s.t_ms >= cutoff);
    if let Some(n) = max_n {
        while buf.len() > n {
            buf.remove(0);
        }
    }
    if buf.is_empty() {
        return x;
    }
    buf.iter().map(|s| s.v).sum::<f64>() / buf.len() as f64
}

fn mean_buf(buf: &[Sample]) -> Option<f64> {
    if buf.is_empty() {
        None
    } else {
        Some(buf.iter().map(|s| s.v).sum::<f64>() / buf.len() as f64)
    }
}

/// Transform a point value through every calc that targets `point_name`.
/// Updates runtime state (deadband hold, rolling buffers).
pub fn apply_for_egress(
    configs: &[CalcConfig],
    rt: &mut CalcRuntime,
    point_name: &str,
    raw: &str,
) -> String {
    let mut out = raw.to_string();
    let t = now_ms();
    for cfg in configs.iter().filter(|c| c.source_point == point_name) {
        out = apply_one(cfg, rt, &out, t);
    }
    out
}

/// Peek transformed output without mutating runtime.
/// Deadband: next hold step from current held. Rolling: current window mean.
pub fn peek_for_point(
    configs: &[CalcConfig],
    rt: &CalcRuntime,
    point_name: &str,
    raw: &str,
) -> String {
    let mut out = raw.to_string();
    for cfg in configs.iter().filter(|c| c.source_point == point_name) {
        out = peek_one(cfg, rt, &out);
    }
    out
}

fn peek_one(cfg: &CalcConfig, rt: &CalcRuntime, raw: &str) -> String {
    let Some(x) = parse_f64(raw) else {
        return raw.to_string();
    };
    match cfg.kind.as_str() {
        "deadband" => {
            let band = parse_f64(&cfg.param_a).unwrap_or(0.5).abs();
            let held = rt.deadband_hold.get(&cfg.id).copied();
            let (y, _) = deadband_step(held, x, band);
            fmt_num(y)
        }
        "rolling_avg" => {
            if let Some(buf) = rt.rolling.get(&cfg.id) {
                fmt_num(mean_buf(buf).unwrap_or(x))
            } else {
                fmt_num(x)
            }
        }
        "scale_offset" => {
            let scale = parse_f64(&cfg.param_a).unwrap_or(1.0);
            let offset = parse_f64(&cfg.param_b).unwrap_or(0.0);
            fmt_num(x * scale + offset)
        }
        "clamp" => {
            let lo = parse_f64(&cfg.param_a).unwrap_or(0.0);
            let hi = parse_f64(&cfg.param_b).unwrap_or(100.0);
            fmt_num(x.clamp(lo.min(hi), lo.max(hi)))
        }
        _ => raw.to_string(),
    }
}

fn apply_one(cfg: &CalcConfig, rt: &mut CalcRuntime, raw: &str, now_ms: u64) -> String {
    let Some(x) = parse_f64(raw) else {
        return raw.to_string();
    };
    match cfg.kind.as_str() {
        "deadband" => {
            let band = parse_f64(&cfg.param_a).unwrap_or(0.5).abs();
            let held = rt.deadband_hold.get(&cfg.id).copied();
            let (y, new_held) = deadband_step(held, x, band);
            rt.deadband_hold.insert(cfg.id, new_held);
            fmt_num(y)
        }
        "rolling_avg" => {
            let window = parse_f64(&cfg.window_secs).unwrap_or(10.0).max(0.001);
            let max_n = rolling_max_n(&cfg.param_a);
            let buf = rt.rolling.entry(cfg.id).or_default();
            let y = rolling_avg_step(buf, x, now_ms, window, max_n);
            fmt_num(y)
        }
        "scale_offset" => {
            let scale = parse_f64(&cfg.param_a).unwrap_or(1.0);
            let offset = parse_f64(&cfg.param_b).unwrap_or(0.0);
            fmt_num(x * scale + offset)
        }
        "clamp" => {
            let lo = parse_f64(&cfg.param_a).unwrap_or(0.0);
            let hi = parse_f64(&cfg.param_b).unwrap_or(100.0);
            fmt_num(x.clamp(lo.min(hi), lo.max(hi)))
        }
        _ => raw.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadband_holds_inside_band() {
        let (y, h) = deadband_step(Some(100.0), 100.3, 40.0);
        assert_eq!(y, 100.0);
        assert_eq!(h, 100.0);
        let (y2, h2) = deadband_step(Some(100.0), 139.9, 40.0);
        assert_eq!(y2, 100.0);
        assert_eq!(h2, 100.0);
        let (y3, h3) = deadband_step(Some(100.0), 140.1, 40.0);
        assert_eq!(y3, 140.1);
        assert_eq!(h3, 140.1);
    }

    #[test]
    fn egress_skips_decimal_jitter() {
        let configs = vec![CalcConfig {
            id: 1,
            kind: "deadband".into(),
            source_point: "mqtt_1".into(),
            param_a: "40".into(),
            ..Default::default()
        }];
        let mut rt = CalcRuntime::default();
        let a = apply_for_egress(&configs, &mut rt, "mqtt_1", "100.0");
        assert_eq!(a, "100");
        let b = apply_for_egress(&configs, &mut rt, "mqtt_1", "100.4");
        assert_eq!(b, "100");
        let c = apply_for_egress(&configs, &mut rt, "mqtt_1", "141");
        assert_eq!(c, "141");
    }

    #[test]
    fn rolling_avg_means_samples() {
        let mut buf = Vec::new();
        let t0 = 1_000_000u64;
        let a = rolling_avg_step(&mut buf, 10.0, t0, 10.0, None);
        assert_eq!(a, 10.0);
        let b = rolling_avg_step(&mut buf, 20.0, t0 + 100, 10.0, None);
        assert_eq!(b, 15.0);
        let c = rolling_avg_step(&mut buf, 30.0, t0 + 200, 10.0, Some(2));
        // max 2 samples → 20 and 30
        assert_eq!(c, 25.0);
    }

    #[test]
    fn rolling_avg_egress() {
        let configs = vec![CalcConfig {
            id: 7,
            kind: "rolling_avg".into(),
            source_point: "mqtt_1".into(),
            param_a: "auto".into(),
            window_secs: "60".into(),
            ..Default::default()
        }];
        let mut rt = CalcRuntime::default();
        let a = apply_for_egress(&configs, &mut rt, "mqtt_1", "10");
        let b = apply_for_egress(&configs, &mut rt, "mqtt_1", "20");
        assert_eq!(a, "10");
        assert_eq!(b, "15");
        let peek = peek_for_point(&configs, &rt, "mqtt_1", "99");
        assert_eq!(peek, "15");
    }
}
