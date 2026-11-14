//! Edge calculation transforms applied before Output MQTT egress.
//! UI preview peeks the same runtime so stateful kinds match publish.

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
    /// Rolling-average sample buffers (zero-order-hold timeline).
    rolling: HashMap<u64, Vec<Sample>>,
    /// Last sample for rate-of-change.
    roc_prev: HashMap<u64, Sample>,
    /// Last digital state for hysteresis.
    hyst_on: HashMap<u64, bool>,
}

impl CalcRuntime {
    pub fn clear(&mut self) {
        self.deadband_hold.clear();
        self.rolling.clear();
        self.roc_prev.clear();
        self.hyst_on.clear();
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

fn window_ms(window_secs: f64) -> u64 {
    (window_secs.max(0.001) * 1000.0).round() as u64
}

/// Drop samples older than the window, but keep one at-or-before cutoff for ZOH.
fn trim_rolling(buf: &mut Vec<Sample>, now_ms: u64, win_ms: u64) {
    let cutoff = now_ms.saturating_sub(win_ms);
    let mut keep_from = 0usize;
    for (i, s) in buf.iter().enumerate() {
        if s.t_ms <= cutoff {
            keep_from = i;
        } else {
            break;
        }
    }
    if keep_from > 0 {
        buf.drain(..keep_from);
    }
}

fn cap_rolling(buf: &mut Vec<Sample>, max_n: Option<usize>) {
    if let Some(n) = max_n {
        let limit = n.saturating_add(1);
        while buf.len() > limit {
            buf.remove(0);
        }
    }
}

/// Time-weighted zero-order-hold mean over `[now - window, now]`.
fn time_weighted_avg(buf: &[Sample], now_ms: u64, win_ms: u64) -> Option<f64> {
    if buf.is_empty() || win_ms == 0 {
        return None;
    }
    let start = now_ms.saturating_sub(win_ms);
    let mut v = buf[0].v;
    for s in buf {
        if s.t_ms <= start {
            v = s.v;
        } else {
            break;
        }
    }
    let mut t = start;
    let mut area = 0.0_f64;
    for s in buf.iter().filter(|s| s.t_ms > start && s.t_ms <= now_ms) {
        if s.t_ms > t {
            area += v * (s.t_ms - t) as f64;
            t = s.t_ms;
        }
        v = s.v;
    }
    if now_ms > t {
        area += v * (now_ms - t) as f64;
    }
    Some(area / win_ms as f64)
}

fn rolling_avg_step(
    buf: &mut Vec<Sample>,
    x: f64,
    at_ms: u64,
    window_secs: f64,
    max_n: Option<usize>,
) -> f64 {
    let win_ms = window_ms(window_secs);
    let last_t = buf.last().map(|s| s.t_ms);
    match last_t {
        Some(t) if t == at_ms => {
            if let Some(last) = buf.last_mut() {
                last.v = x;
            }
        }
        Some(t) if at_ms > t => {
            buf.push(Sample {
                t_ms: at_ms,
                v: x,
            });
        }
        Some(t) => {
            // Out-of-order: keep timeline, eval at last stamp.
            return time_weighted_avg(buf, t, win_ms).unwrap_or(x);
        }
        None => {
            buf.push(Sample {
                t_ms: at_ms,
                v: x,
            });
        }
    }
    trim_rolling(buf, at_ms, win_ms);
    cap_rolling(buf, max_n);
    time_weighted_avg(buf, at_ms, win_ms).unwrap_or(x)
}

fn rolling_avg_peek(
    buf: &[Sample],
    tip: Option<(u64, f64)>,
    now_ms: u64,
    window_secs: f64,
) -> Option<f64> {
    let win_ms = window_ms(window_secs);
    if buf.is_empty() {
        return tip.map(|(_, v)| v);
    }
    if let Some((tip_ms, tip_v)) = tip {
        let last_t = buf.last().map(|s| s.t_ms).unwrap_or(0);
        if tip_ms > last_t {
            let mut tmp = buf.to_vec();
            tmp.push(Sample {
                t_ms: tip_ms,
                v: tip_v,
            });
            trim_rolling(&mut tmp, now_ms, win_ms);
            return time_weighted_avg(&tmp, now_ms, win_ms);
        }
    }
    let cutoff = now_ms.saturating_sub(win_ms);
    let start = buf.iter().rposition(|s| s.t_ms <= cutoff).unwrap_or(0);
    time_weighted_avg(&buf[start..], now_ms, win_ms)
}

/// Seconds-scale factor for rate units (`/s`, `/min`, `/h`).
fn roc_per_sec_scale(units: &str) -> f64 {
    match units.trim() {
        "/min" => 60.0,
        "/h" => 3600.0,
        _ => 1.0, // /s default
    }
}

fn rate_of_change(prev: Option<Sample>, x: f64, at_ms: u64, units: &str, abs: bool) -> (f64, Sample) {
    let cur = Sample {
        t_ms: at_ms,
        v: x,
    };
    let Some(p) = prev else {
        return (0.0, cur);
    };
    if at_ms <= p.t_ms {
        return (0.0, p);
    }
    let dt_s = (at_ms - p.t_ms) as f64 / 1000.0;
    if dt_s <= 0.0 {
        return (0.0, cur);
    }
    let mut rate = (x - p.v) / dt_s * roc_per_sec_scale(units);
    if abs {
        rate = rate.abs();
    }
    (rate, cur)
}

fn hysteresis_step(prev: Option<bool>, x: f64, on_at: f64, off_at: f64) -> bool {
    match prev {
        None => x >= on_at,
        Some(true) => !(x <= off_at),
        Some(false) => x >= on_at,
    }
}

/// Transform a point value through every calc that targets `point_name`.
pub fn apply_for_egress(
    configs: &[CalcConfig],
    rt: &mut CalcRuntime,
    point_name: &str,
    raw: &str,
) -> String {
    apply_at(configs, rt, point_name, raw, now_ms())
}

/// Same as [`apply_for_egress`] with an explicit sample timestamp (tests + host).
pub fn apply_at(
    configs: &[CalcConfig],
    rt: &mut CalcRuntime,
    point_name: &str,
    raw: &str,
    at_ms: u64,
) -> String {
    let mut out = raw.to_string();
    for cfg in configs.iter().filter(|c| c.source_point == point_name) {
        out = apply_one(cfg, rt, &out, at_ms);
    }
    out
}

/// Peek transformed output without mutating runtime.
pub fn peek_for_point(
    configs: &[CalcConfig],
    rt: &CalcRuntime,
    point_name: &str,
    raw: &str,
) -> String {
    let mut out = raw.to_string();
    let t = now_ms();
    for cfg in configs.iter().filter(|c| c.source_point == point_name) {
        out = peek_one(cfg, rt, &out, t);
    }
    out
}

fn peek_one(cfg: &CalcConfig, rt: &CalcRuntime, raw: &str, now_ms: u64) -> String {
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
            let window = parse_f64(&cfg.window_secs).unwrap_or(10.0);
            let buf = rt.rolling.get(&cfg.id).map(|b| b.as_slice()).unwrap_or(&[]);
            let y = rolling_avg_peek(buf, Some((now_ms, x)), now_ms, window).unwrap_or(x);
            fmt_num(y)
        }
        "rate_of_change" => {
            let abs = cfg.param_b.trim().eq_ignore_ascii_case("abs");
            let prev = rt.roc_prev.get(&cfg.id).copied();
            let (rate, _) = rate_of_change(prev, x, now_ms, &cfg.param_a, abs);
            fmt_num(rate)
        }
        "hysteresis" => {
            let on_at = parse_f64(&cfg.param_a).unwrap_or(80.0);
            let off_at = parse_f64(&cfg.param_b).unwrap_or(70.0);
            let prev = rt.hyst_on.get(&cfg.id).copied();
            let on = hysteresis_step(prev, x, on_at, off_at);
            if on {
                "1".into()
            } else {
                "0".into()
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

fn apply_one(cfg: &CalcConfig, rt: &mut CalcRuntime, raw: &str, at_ms: u64) -> String {
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
            let window = parse_f64(&cfg.window_secs).unwrap_or(10.0);
            let max_n = rolling_max_n(&cfg.param_a);
            let buf = rt.rolling.entry(cfg.id).or_default();
            let y = rolling_avg_step(buf, x, at_ms, window, max_n);
            fmt_num(y)
        }
        "rate_of_change" => {
            let abs = cfg.param_b.trim().eq_ignore_ascii_case("abs");
            let prev = rt.roc_prev.get(&cfg.id).copied();
            let (rate, cur) = rate_of_change(prev, x, at_ms, &cfg.param_a, abs);
            rt.roc_prev.insert(cfg.id, cur);
            fmt_num(rate)
        }
        "hysteresis" => {
            let on_at = parse_f64(&cfg.param_a).unwrap_or(80.0);
            let off_at = parse_f64(&cfg.param_b).unwrap_or(70.0);
            let prev = rt.hyst_on.get(&cfg.id).copied();
            let on = hysteresis_step(prev, x, on_at, off_at);
            rt.hyst_on.insert(cfg.id, on);
            if on {
                "1".into()
            } else {
                "0".into()
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
        let a = apply_at(&configs, &mut rt, "mqtt_1", "100.0", 1_000);
        assert_eq!(a, "100");
        let b = apply_at(&configs, &mut rt, "mqtt_1", "100.4", 1_100);
        assert_eq!(b, "100");
        let c = apply_at(&configs, &mut rt, "mqtt_1", "141", 1_200);
        assert_eq!(c, "141");
    }

    #[test]
    fn rolling_step_holds_previous_across_window() {
        let mut buf = Vec::new();
        let t0 = 1_000_000u64;
        let _ = rolling_avg_step(&mut buf, 10.0, t0, 10.0, None);
        let y = rolling_avg_step(&mut buf, 100.0, t0 + 9_000, 10.0, None);
        assert!((y - 10.0).abs() < 0.05, "step must not jump avg, got {y}");
        let y2 = rolling_avg_step(&mut buf, 100.0, t0 + 9_500, 10.0, None);
        assert!(
            (y2 - 14.5).abs() < 0.05,
            "expected ~14.5 after 0.5s at 100, got {y2}"
        );
    }

    #[test]
    fn rolling_avg_egress_smooths_square_wave() {
        let configs = vec![CalcConfig {
            id: 7,
            kind: "rolling_avg".into(),
            source_point: "mqtt_1".into(),
            param_a: "auto".into(),
            window_secs: "10".into(),
            ..Default::default()
        }];
        let mut rt = CalcRuntime::default();
        let t0 = 5_000_000u64;
        for i in 0..100 {
            let v = if i % 2 == 0 { "0" } else { "100" };
            let _ = apply_at(&configs, &mut rt, "mqtt_1", v, t0 + i * 100);
        }
        let mid = apply_at(&configs, &mut rt, "mqtt_1", "100", t0 + 10_000);
        let mid_n: f64 = mid.parse().unwrap();
        assert!(
            (mid_n - 50.0).abs() < 2.0,
            "10s square wave should average ~50, got {mid_n}"
        );
        let after = apply_at(&configs, &mut rt, "mqtt_1", "0", t0 + 10_100);
        let after_n: f64 = after.parse().unwrap();
        assert!(
            after_n > 40.0,
            "output must not track input jump; got {after_n}"
        );
    }

    #[test]
    fn rolling_avg_gaussian_noise_reduces_variance() {
        let configs = vec![CalcConfig {
            id: 9,
            kind: "rolling_avg".into(),
            source_point: "mqtt_1".into(),
            param_a: "auto".into(),
            window_secs: "10".into(),
            ..Default::default()
        }];
        let mut rt = CalcRuntime::default();
        let t0 = 9_000_000u64;
        let mut state = 0xC0FFEEu64;
        let mut raws = Vec::new();
        let mut outs = Vec::new();
        for i in 0..200 {
            let mut u = 0.0;
            for _ in 0..6 {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                u += (state % 1000) as f64 / 1000.0;
            }
            let x = 20.0 + u * (80.0 / 6.0);
            raws.push(x);
            let y: f64 = apply_at(
                &configs,
                &mut rt,
                "mqtt_1",
                &format!("{x:.6}"),
                t0 + i * 50,
            )
            .parse()
            .unwrap();
            outs.push(y);
        }
        fn variance(xs: &[f64]) -> f64 {
            let n = xs.len() as f64;
            let mean = xs.iter().sum::<f64>() / n;
            xs.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n
        }
        let raw_var = variance(&raws[raws.len() - 40..]);
        let out_var = variance(&outs[outs.len() - 40..]);
        assert!(
            out_var < raw_var * 0.35,
            "10s rolling avg must cut variance: raw={raw_var:.3} out={out_var:.3}"
        );
        let last_raw = *raws.last().unwrap();
        let last_out = *outs.last().unwrap();
        assert!(
            (last_out - last_raw).abs() > 0.5,
            "output tracking input too closely: raw={last_raw} out={last_out}"
        );
    }

    #[test]
    fn rate_of_change_per_second() {
        let configs = vec![CalcConfig {
            id: 3,
            kind: "rate_of_change".into(),
            source_point: "mqtt_1".into(),
            param_a: "/s".into(),
            ..Default::default()
        }];
        let mut rt = CalcRuntime::default();
        let a = apply_at(&configs, &mut rt, "mqtt_1", "10", 1_000);
        assert_eq!(a, "0");
        // +10 over 0.5s → 20 /s
        let b = apply_at(&configs, &mut rt, "mqtt_1", "20", 1_500);
        assert_eq!(b, "20");
        // +0 over next 1s → 0 /s
        let c = apply_at(&configs, &mut rt, "mqtt_1", "20", 2_500);
        assert_eq!(c, "0");
        // abs mode
        let configs_abs = vec![CalcConfig {
            id: 3,
            kind: "rate_of_change".into(),
            source_point: "mqtt_1".into(),
            param_a: "/s".into(),
            param_b: "abs".into(),
            ..Default::default()
        }];
        let d = apply_at(&configs_abs, &mut rt, "mqtt_1", "10", 3_500);
        assert_eq!(d, "10");
    }

    #[test]
    fn hysteresis_holds_between_thresholds() {
        let configs = vec![CalcConfig {
            id: 4,
            kind: "hysteresis".into(),
            source_point: "mqtt_1".into(),
            param_a: "80".into(),
            param_b: "70".into(),
            ..Default::default()
        }];
        let mut rt = CalcRuntime::default();
        assert_eq!(apply_at(&configs, &mut rt, "mqtt_1", "50", 1), "0");
        assert_eq!(apply_at(&configs, &mut rt, "mqtt_1", "75", 2), "0");
        assert_eq!(apply_at(&configs, &mut rt, "mqtt_1", "80", 3), "1");
        assert_eq!(apply_at(&configs, &mut rt, "mqtt_1", "75", 4), "1"); // hold
        assert_eq!(apply_at(&configs, &mut rt, "mqtt_1", "70", 5), "0");
        assert_eq!(peek_for_point(&configs, &rt, "mqtt_1", "75"), "0"); // still off until on
        assert_eq!(peek_for_point(&configs, &rt, "mqtt_1", "80"), "1");
    }
}
