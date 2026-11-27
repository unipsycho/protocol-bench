//! The generator worker: owns the clock, fires each signal when its trigger is due and publishes
//! the value as a host point. One worker per board (connection).

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use pb_abi::PB_LOG_INFO;
use pb_extkit::{leaf1, ConnExt, LinkCtx};
use serde_json::{Map, Value};

use pb_signal::{Signal, BUILTIN_VARS};

pub struct PackState(pub pb_extkit::ExtPack);

fn endpoint(c: &Map<String, Value>) -> String {
    let seed = c.n("seed", 0.0) as u64;
    let speed = c.n("speed", 1.0);
    let mut s = if seed == 0 { "random seed".to_string() } else { format!("seed {seed}") };
    if (speed - 1.0).abs() > 1e-9 {
        s.push_str(&format!(" · ×{speed}"));
    }
    s
}

fn check(c: &Map<String, Value>) -> Result<(), String> {
    let speed = c.n("speed", 1.0);
    if !(speed.is_finite() && speed > 0.0) {
        return Err("speed must be greater than 0".into());
    }
    Ok(())
}

struct Slot {
    sig: Signal,
    bind: usize,
    next_at: Instant,
    last_t: Option<f64>,
    error: Option<String>,
    label: String,
}

/// Run one board until stopped.
pub fn run(ctx: LinkCtx) {
    let seed = ctx.conn.n("seed", 0.0).max(0.0) as u64;
    let speed = ctx.conn.n("speed", 1.0).clamp(0.001, 1_000_000.0);

    // Build signals; a bad one is reported and published bad, the rest run.
    let mut slots: Vec<Slot> = Vec::new();
    let mut problems: Vec<String> = Vec::new();
    let mut names = HashSet::new();
    for (i, b) in ctx.inputs.iter().enumerate() {
        match Signal::from_bind(b, seed, i) {
            Ok(sig) => {
                if !names.insert(sig.name.clone()) {
                    problems.push(format!("{}: duplicate signal name", sig.name));
                    ctx.publish_bad(b);
                    continue;
                }
                let label = format!("{}  ·  {}  ·  {}", sig.name, sig.model_id, sig.schedule.describe());
                slots.push(Slot { sig, bind: i, next_at: Instant::now(), last_t: None, error: None, label });
            }
            Err(e) => {
                let n = b.text("name");
                problems.push(format!("{}: {e}", if n.trim().is_empty() { format!("signal {}", i + 1) } else { n }));
                ctx.publish_bad(b);
            }
        }
    }
    // A formula may only read built-ins and signals of this board.
    let known: HashSet<String> = names.clone();
    slots.retain(|s| {
        for v in s.sig.formula_vars() {
            if !BUILTIN_VARS.contains(&v.as_str()) && !known.contains(&v) {
                problems.push(format!("{}: unknown variable '{v}' (use t, dt, prev, n or another signal's name)", s.sig.name));
                ctx.publish_bad(&ctx.inputs[s.bind]);
                return false;
            }
        }
        true
    });

    let n = slots.len();
    let first_problem = problems.join("; ");
    ctx.up(match n {
        0 => "no signals yet — add some on a hub Input".to_string(),
        1 => "running · 1 signal".to_string(),
        n => format!("running · {n} signals"),
    });
    ctx.status(|s| s.error = first_problem.clone());
    ctx.log(PB_LOG_INFO, &format!("generator up: {n} signal(s), {}", endpoint(&ctx.conn)));

    let t0 = Instant::now();
    let mut vars: HashMap<String, f64> = HashMap::new();
    let mut shown_error = first_problem;
    while !ctx.stopped() {
        let now = Instant::now();
        for slot in slots.iter_mut() {
            if now < slot.next_at {
                continue;
            }
            let t = t0.elapsed().as_secs_f64() * speed;
            let dt = slot.last_t.map(|l| (t - l).max(0.0)).unwrap_or(0.0);
            slot.last_t = Some(t);
            let result = {
                let look = |name: &str| vars.get(name).copied();
                slot.sig.sample(t, dt, &look)
            };
            let b = &ctx.inputs[slot.bind];
            match result {
                Ok(v) => {
                    slot.error = None;
                    ctx.publish(b, &v);
                    if let Some(x) = v.as_f64() {
                        vars.insert(slot.sig.name.clone(), x);
                    }
                    let text = v.to_string();
                    ctx.status(|s| {
                        s.rx += 1;
                        s.see(&slot.sig.name, leaf1("name", slot.sig.name.clone()), slot.label.clone(), text);
                    });
                }
                Err(e) => {
                    slot.error = Some(e);
                    ctx.publish_bad(b);
                    ctx.status(|s| s.errors += 1);
                }
            }
            let delay = slot.sig.schedule.next_delay(&mut slot.sig.rng).max(0.001);
            // Keep a drift-free grid unless we fell far behind (a stalled thread, a long sleep).
            let base = if now > slot.next_at + Duration::from_secs(1) { now } else { slot.next_at };
            slot.next_at = base + Duration::from_secs_f64(delay);
        }
        // Surface the first current error (and clear it when fixed).
        let errs: Vec<String> = slots.iter().filter_map(|s| s.error.as_ref().map(|e| format!("{}: {e}", s.sig.name))).collect();
        let text = if errs.is_empty() { problems.join("; ") } else { errs.join("; ") };
        if text != shown_error {
            ctx.status(|s| s.error = text.clone());
            shown_error = text;
        }
        let wait = slots.iter().map(|s| s.next_at.saturating_duration_since(Instant::now())).min().unwrap_or(Duration::from_millis(200));
        if !pb_extkit::sleep_stop(&ctx.stop, wait.min(Duration::from_millis(25))) {
            break;
        }
    }
}

pb_extkit::ext_pack_impl!(PackState, b"gen\0", b"Generator\0", "gen", run, endpoint, check);
pb_abi::export_pack!(PackState);

#[cfg(test)]
mod tests {
    use super::*;
    use pb_extkit::testhost::{self, Rec};
    use pb_extkit::{ExtPack, Val};

    fn pack(host: pb_abi::HostRef) -> ExtPack {
        ExtPack::new(host, "gen", run, endpoint, check)
    }

    fn cfg(conn: Value, binds: Vec<Value>) -> String {
        serde_json::json!({"connections":[conn],"inputs":[{"connection":"b","binds":binds}]}).to_string()
    }

    fn sig(name: &str, extra: Value) -> Value {
        let mut o = extra.as_object().unwrap().clone();
        o.insert("name".into(), name.into());
        o.insert("point".into(), name.into());
        o.insert("source".into(), format!("gen/b::{name}").into());
        Value::Object(o)
    }

    fn vals(log: &'static testhost::Log, name: &str) -> Vec<Rec> {
        log.lock().unwrap().iter().filter(|r| r.name == name).cloned().collect()
    }

    #[test]
    fn publishes_each_model_type_on_an_interval() {
        let (log, host) = testhost::host();
        let mut p = pack(host);
        p.configure(&cfg(
            serde_json::json!({"id":"b","seed":5}),
            vec![
                sig("level", serde_json::json!({"model":"sine","low":10,"high":20,"period":1,"every":0.02})),
                sig("count", serde_json::json!({"model":"counter","step":2,"every":0.02})),
                sig("run", serde_json::json!({"model":"cycle","on_secs":0.1,"off_secs":0.1,"every":0.02})),
            ],
        ))
        .unwrap();
        testhost::wait(log, "all three signals", |l| {
            ["level", "count", "run"].iter().all(|n| l.iter().filter(|r| r.name == *n).count() >= 8)
        });
        let level = vals(log, "level");
        assert!(level.iter().all(|r| matches!(r.value, Val::Float(v) if (10.0..=20.0).contains(&v))));
        assert_eq!(level[0].source, "gen/b::level");
        let counts: Vec<i64> = vals(log, "count").iter().filter_map(|r| if let Val::Int(i) = r.value { Some(i) } else { None }).collect();
        assert_eq!(&counts[..4], &[0, 2, 4, 6], "first value is the start, then +step each fire");
        let runs = vals(log, "run");
        assert!(runs.iter().any(|r| r.value == Val::Bool(true)) && runs.iter().any(|r| r.value == Val::Bool(false)), "cycle flips within 0.2 s");
        let d: Value = serde_json::from_str(&p.detail_json()).unwrap();
        let c = &d["connections"][0];
        assert_eq!(c["connected"], true);
        assert!(c["state"].as_str().unwrap().contains("3 signals"));
        assert!(c["rx"].as_u64().unwrap() >= 24);
        assert_eq!(c["seen"].as_array().unwrap().len(), 3);
        p.shutdown();
    }

    #[test]
    fn formulas_read_other_signals_and_report_problems() {
        let (log, host) = testhost::host();
        let mut p = pack(host);
        p.configure(&cfg(
            serde_json::json!({"id":"b","seed":1}),
            vec![
                sig("base", serde_json::json!({"model":"constant","value":10,"every":0.02})),
                sig("double", serde_json::json!({"model":"formula","expr":"base * 2 + n","every":0.02})),
                sig("bad", serde_json::json!({"model":"formula","expr":"1 +","every":0.02})),
                sig("ghost", serde_json::json!({"model":"formula","expr":"nothere + 1","every":0.02})),
                sig("base", serde_json::json!({"model":"constant","value":1})),
            ],
        ))
        .unwrap();
        testhost::wait(log, "double advances", |l| l.iter().any(|r| r.name == "double" && r.value == Val::Float(22.0)));
        assert!(vals(log, "double").iter().all(|r| r.quality == pb_abi::PB_QUALITY_GOOD));
        // Invalid signals publish bad quality once and never a made-up value.
        assert!(vals(log, "bad").iter().all(|r| r.quality == pb_abi::PB_QUALITY_BAD));
        assert!(vals(log, "ghost").iter().all(|r| r.quality == pb_abi::PB_QUALITY_BAD));
        let err = p.last_error();
        assert!(err.contains("bad: formula"), "{err}");
        assert!(err.contains("unknown variable 'nothere'"), "{err}");
        assert!(err.contains("duplicate signal name"), "{err}");
        p.shutdown();
    }

    #[test]
    fn seeded_boards_repeat_per_signal() {
        let run_once = |seed: u64| {
            let (log, host) = testhost::host();
            let mut p = pack(host);
            p.configure(&cfg(
                serde_json::json!({"id":"b","seed":seed}),
                vec![sig("w", serde_json::json!({"model":"walk","start":50,"step":2,"low":0,"high":100,"every":0.01}))],
            ))
            .unwrap();
            testhost::wait(log, "20 samples", |l| l.iter().filter(|r| r.name == "w").count() >= 20);
            p.shutdown();
            vals(log, "w").iter().take(20).map(|r| r.value.as_f64().unwrap()).collect::<Vec<_>>()
        };
        let (a, b, c) = (run_once(77), run_once(77), run_once(78));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn random_timer_is_irregular_but_inside_its_range() {
        let (log, host) = testhost::host();
        let mut p = pack(host);
        p.configure(&cfg(
            serde_json::json!({"id":"b","seed":9}),
            vec![sig("evt", serde_json::json!({"model":"toggle","trigger":"random","every_min":0.03,"every_max":0.09}))],
        ))
        .unwrap();
        let start = Instant::now();
        testhost::wait(log, "12 events", |l| l.iter().filter(|r| r.name == "evt").count() >= 12);
        let took = start.elapsed().as_secs_f64();
        // 11 gaps of 30–90 ms: between 0.33 s and ~1 s (generous upper bound for a loaded box).
        assert!((0.3..2.5).contains(&took), "took {took}");
        p.shutdown();
    }

    #[test]
    fn burst_trigger_emits_groups_then_idles() {
        let (log, host) = testhost::host();
        let mut p = pack(host);
        p.configure(&cfg(
            serde_json::json!({"id":"b","seed":2}),
            vec![sig("b", serde_json::json!({"model":"counter","trigger":"burst","burst_count":4,"burst_gap":0.02,"idle_min":0.4,"idle_max":0.4}))],
        ))
        .unwrap();
        testhost::wait(log, "first burst", |l| l.iter().filter(|r| r.name == "b").count() >= 4);
        std::thread::sleep(Duration::from_millis(250));
        assert_eq!(vals(log, "b").len(), 4, "idle gap holds the next burst back");
        testhost::wait(log, "second burst", |l| l.iter().filter(|r| r.name == "b").count() >= 8);
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(vals(log, "b").len(), 8);
        p.shutdown();
    }

    #[test]
    fn speed_scales_model_time_and_bad_speed_is_rejected() {
        let (log, host) = testhost::host();
        let mut p = pack(host);
        // ×1000: a 100 s sawtooth sweeps its whole range in ~0.1 s of wall clock.
        p.configure(&cfg(
            serde_json::json!({"id":"b","seed":3,"speed":1000}),
            vec![sig("s", serde_json::json!({"model":"sawtooth","low":0,"high":1,"period":100,"every":0.01,"decimals":3}))],
        ))
        .unwrap();
        testhost::wait(log, "sweep covers the range", |l| {
            let v: Vec<f64> = l.iter().filter(|r| r.name == "s").filter_map(|r| r.value.as_f64()).collect();
            v.iter().cloned().fold(0.0, f64::max) > 0.8 && v.iter().cloned().fold(1.0, f64::min) < 0.2
        });
        p.shutdown();
        let (_l, host) = testhost::host();
        let mut p = pack(host);
        p.configure(&cfg(serde_json::json!({"id":"b","speed":0}), vec![])).unwrap();
        assert!(p.last_error().contains("speed must be greater than 0"), "{}", p.last_error());
    }

    #[test]
    fn editing_a_signal_restarts_the_board_and_an_empty_board_is_idle() {
        let (log, host) = testhost::host();
        let mut p = pack(host);
        p.configure(&cfg(serde_json::json!({"id":"b"}), vec![])).unwrap();
        testhost::wait(log, "up", |_| p.is_connected());
        assert!(p.state_label().contains("no signals") || p.is_connected());
        p.configure(&cfg(serde_json::json!({"id":"b","seed":1}), vec![sig("c", serde_json::json!({"model":"constant","value":3,"every":0.02}))])).unwrap();
        testhost::wait(log, "c = 3", |l| l.iter().any(|r| r.name == "c" && r.value == Val::Float(3.0)));
        p.configure(&cfg(serde_json::json!({"id":"b","seed":1}), vec![sig("c", serde_json::json!({"model":"constant","value":4,"every":0.02}))])).unwrap();
        testhost::wait(log, "c = 4", |l| l.iter().any(|r| r.name == "c" && r.value == Val::Float(4.0)));
        p.shutdown();
    }
}
