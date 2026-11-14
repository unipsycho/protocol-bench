//! Multi-member aggregators → host points `agg_N`.
//! Quality is worst-wins across contributing members.

use pb_points::{PointValue, Quality};
use std::collections::{HashMap, VecDeque};
use std::time::{SystemTime, UNIX_EPOCH};

/// One aggregator step synced from the hub diagram.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AggConfig {
    pub id: u64,
    pub label: String,
    pub mode: String,
    /// `none` | `n:<count>` | `t:<secs>`
    pub window: String,
    pub members: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
struct AggSample {
    t_ms: u64,
    v: f64,
    q: Quality,
}

/// Per-aggregator history for windowed modes.
#[derive(Debug, Default)]
pub struct AggRuntime {
    /// Samples per aggregator id (flattened member reads over time).
    hist: HashMap<u64, VecDeque<AggSample>>,
}

impl AggRuntime {
    pub fn clear(&mut self) {
        self.hist.clear();
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Parse window spec → (max_count, max_age_ms). Both None = current values only.
pub fn parse_window(spec: &str) -> (Option<usize>, Option<u64>) {
    let t = spec.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("none") {
        return (None, None);
    }
    if let Some(rest) = t.strip_prefix("n:") {
        let n = rest.trim().parse::<usize>().ok().filter(|n| *n > 0);
        return (n, None);
    }
    if let Some(rest) = t.strip_prefix("t:") {
        let secs = rest.trim().parse::<f64>().ok().filter(|s| *s > 0.0);
        return (None, secs.map(|s| (s * 1000.0).round() as u64));
    }
    // Bare number → treat as last N.
    if let Ok(n) = t.parse::<usize>() {
        if n > 0 {
            return (Some(n), None);
        }
    }
    (None, None)
}

fn worse(a: Quality, b: Quality) -> Quality {
    use Quality::*;
    match (a, b) {
        (Bad, _) | (_, Bad) => Bad,
        (Uncertain, _) | (_, Uncertain) => Uncertain,
        _ => Good,
    }
}

fn parse_num(v: &PointValue) -> Option<f64> {
    match v {
        PointValue::F64(x) => Some(*x),
        PointValue::I64(x) => Some(*x as f64),
        PointValue::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        PointValue::String(s) => {
            let t = s.trim();
            match t.to_ascii_lowercase().as_str() {
                "true" | "on" => Some(1.0),
                "false" | "off" => Some(0.0),
                _ => t.parse().ok(),
            }
        }
        PointValue::Bytes(_) => None,
    }
}

fn truthy(v: f64) -> bool {
    v.abs() > 1e-12
}

fn median(mut xs: Vec<f64>) -> Option<f64> {
    if xs.is_empty() {
        return None;
    }
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = xs.len();
    if n % 2 == 1 {
        Some(xs[n / 2])
    } else {
        Some((xs[n / 2 - 1] + xs[n / 2]) * 0.5)
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

/// Output short id for an aggregator step.
pub fn agg_point_name(id: u64) -> String {
    format!("agg_{id}")
}

/// One member reading from the host store.
#[derive(Debug, Clone)]
pub struct MemberReading {
    pub name: String,
    pub value: PointValue,
    pub quality: Quality,
    pub updated_ms: u64,
}

/// Result of evaluating one aggregator.
#[derive(Debug, Clone)]
pub struct AggOutput {
    pub name: String,
    pub value: String,
    pub quality: Quality,
}

fn combine(mode: &str, samples: &[AggSample], current: &[MemberReading]) -> Option<(f64, Quality)> {
    let mut q = Quality::Good;
    for s in samples {
        q = worse(q, s.q);
    }
    if samples.is_empty() && current.is_empty() {
        return None;
    }
    // Digital modes prefer current member set.
    match mode {
        "any-good" => {
            let mut q = Quality::Good;
            let mut any = false;
            for m in current {
                q = worse(q, m.quality);
                if m.quality == Quality::Good {
                    any = true;
                }
            }
            if current.is_empty() {
                return None;
            }
            Some((if any { 1.0 } else { 0.0 }, q))
        }
        "majority" => {
            let mut q = Quality::Good;
            let mut yes = 0usize;
            let mut n = 0usize;
            for m in current {
                q = worse(q, m.quality);
                n += 1;
                if parse_num(&m.value).map(truthy).unwrap_or(false) {
                    yes += 1;
                }
            }
            if n == 0 {
                return None;
            }
            Some((if yes * 2 > n { 1.0 } else { 0.0 }, q))
        }
        "count" => Some((samples.len() as f64, q)),
        "first" => samples.first().map(|s| (s.v, worse(q, s.q))),
        "last" => samples.last().map(|s| (s.v, worse(q, s.q))),
        "sum" => {
            if samples.is_empty() {
                return None;
            }
            Some((samples.iter().map(|s| s.v).sum(), q))
        }
        "mean" => {
            if samples.is_empty() {
                return None;
            }
            let sum: f64 = samples.iter().map(|s| s.v).sum();
            Some((sum / samples.len() as f64, q))
        }
        "min" => samples
            .iter()
            .map(|s| s.v)
            .reduce(f64::min)
            .map(|v| (v, q)),
        "max" => samples
            .iter()
            .map(|s| s.v)
            .reduce(f64::max)
            .map(|v| (v, q)),
        "median" => median(samples.iter().map(|s| s.v).collect()).map(|v| (v, q)),
        _ => {
            // Default mean.
            if samples.is_empty() {
                return None;
            }
            let sum: f64 = samples.iter().map(|s| s.v).sum();
            Some((sum / samples.len() as f64, q))
        }
    }
}

fn push_window(
    hist: &mut VecDeque<AggSample>,
    readings: &[MemberReading],
    at_ms: u64,
    max_n: Option<usize>,
    max_age: Option<u64>,
) {
    for m in readings {
        let Some(v) = parse_num(&m.value) else {
            continue;
        };
        hist.push_back(AggSample {
            t_ms: at_ms.max(m.updated_ms),
            v,
            q: m.quality,
        });
    }
    if let Some(age) = max_age {
        let cutoff = at_ms.saturating_sub(age);
        while hist.front().is_some_and(|s| s.t_ms < cutoff) {
            hist.pop_front();
        }
    }
    if let Some(n) = max_n {
        while hist.len() > n {
            hist.pop_front();
        }
    }
}

/// Evaluate all aggregators that have ≥2 members; update window history.
pub fn evaluate_all(
    configs: &[AggConfig],
    rt: &mut AggRuntime,
    lookup: &dyn Fn(&str) -> Option<MemberReading>,
) -> Vec<AggOutput> {
    let t = now_ms();
    let mut out = Vec::new();
    for cfg in configs {
        let members: Vec<String> = cfg
            .members
            .iter()
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty())
            .collect();
        if members.len() < 2 {
            continue;
        }
        let readings: Vec<MemberReading> = members
            .iter()
            .filter_map(|name| lookup(name))
            .collect();
        if readings.len() < 2 {
            continue;
        }
        let (max_n, max_age) = parse_window(&cfg.window);
        let hist = rt.hist.entry(cfg.id).or_default();
        if max_n.is_none() && max_age.is_none() {
            // none → current snapshot only (replace hist each tick).
            hist.clear();
            for m in &readings {
                if let Some(v) = parse_num(&m.value) {
                    hist.push_back(AggSample {
                        t_ms: t.max(m.updated_ms),
                        v,
                        q: m.quality,
                    });
                }
            }
        } else {
            push_window(hist, &readings, t, max_n, max_age);
        }
        let samples: Vec<AggSample> = hist.iter().copied().collect();
        if let Some((v, q)) = combine(&cfg.mode, &samples, &readings) {
            out.push(AggOutput {
                name: agg_point_name(cfg.id),
                value: fmt_num(v),
                quality: q,
            });
        }
    }
    out
}

/// Peek without mutating history (uses a throwaway runtime clone of hist… or current-only).
pub fn peek_one(
    cfg: &AggConfig,
    rt: &AggRuntime,
    lookup: &dyn Fn(&str) -> Option<MemberReading>,
) -> Option<AggOutput> {
    let members: Vec<String> = cfg
        .members
        .iter()
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty())
        .collect();
    if members.len() < 2 {
        return None;
    }
    let readings: Vec<MemberReading> = members
        .iter()
        .filter_map(|name| lookup(name))
        .collect();
    if readings.len() < 2 {
        return None;
    }
    let (max_n, max_age) = parse_window(&cfg.window);
    let samples: Vec<AggSample> = if max_n.is_none() && max_age.is_none() {
        readings
            .iter()
            .filter_map(|m| {
                parse_num(&m.value).map(|v| AggSample {
                    t_ms: m.updated_ms,
                    v,
                    q: m.quality,
                })
            })
            .collect()
    } else {
        rt.hist
            .get(&cfg.id)
            .map(|h| h.iter().copied().collect())
            .unwrap_or_default()
    };
    let (v, q) = combine(&cfg.mode, &samples, &readings)?;
    Some(AggOutput {
        name: agg_point_name(cfg.id),
        value: fmt_num(v),
        quality: q,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading(name: &str, v: f64, q: Quality) -> MemberReading {
        MemberReading {
            name: name.into(),
            value: PointValue::F64(v),
            quality: q,
            updated_ms: 1,
        }
    }

    #[test]
    fn mean_of_two_members() {
        let cfg = AggConfig {
            id: 7,
            mode: "mean".into(),
            window: "none".into(),
            members: vec!["a".into(), "b".into()],
            ..Default::default()
        };
        let mut rt = AggRuntime::default();
        let map = |n: &str| match n {
            "a" => Some(reading("a", 10.0, Quality::Good)),
            "b" => Some(reading("b", 30.0, Quality::Good)),
            _ => None,
        };
        let out = evaluate_all(&[cfg], &mut rt, &map);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "agg_7");
        assert_eq!(out[0].value, "20");
        assert_eq!(out[0].quality, Quality::Good);
    }

    #[test]
    fn quality_worst_wins() {
        let cfg = AggConfig {
            id: 1,
            mode: "sum".into(),
            window: "none".into(),
            members: vec!["a".into(), "b".into()],
            ..Default::default()
        };
        let mut rt = AggRuntime::default();
        let map = |n: &str| match n {
            "a" => Some(reading("a", 1.0, Quality::Good)),
            "b" => Some(reading("b", 2.0, Quality::Bad)),
            _ => None,
        };
        let out = evaluate_all(&[cfg], &mut rt, &map);
        assert_eq!(out[0].quality, Quality::Bad);
        assert_eq!(out[0].value, "3");
    }

    #[test]
    fn needs_two_members() {
        let cfg = AggConfig {
            id: 2,
            mode: "mean".into(),
            window: "none".into(),
            members: vec!["a".into()],
            ..Default::default()
        };
        let mut rt = AggRuntime::default();
        let map = |n: &str| Some(reading(n, 1.0, Quality::Good));
        assert!(evaluate_all(&[cfg], &mut rt, &map).is_empty());
    }

    #[test]
    fn hysteresis_majority() {
        let cfg = AggConfig {
            id: 3,
            mode: "majority".into(),
            window: "none".into(),
            members: vec!["a".into(), "b".into(), "c".into()],
            ..Default::default()
        };
        let mut rt = AggRuntime::default();
        let map = |n: &str| match n {
            "a" => Some(reading("a", 1.0, Quality::Good)),
            "b" => Some(reading("b", 1.0, Quality::Good)),
            "c" => Some(reading("c", 0.0, Quality::Good)),
            _ => None,
        };
        let out = evaluate_all(&[cfg], &mut rt, &map);
        assert_eq!(out[0].value, "1");
    }
}
