//! Desktop-only stand-in for the server: synthetic points, in-memory dashboards, history. Lets
//! the whole page be developed, tested and screenshotted without a browser or a service.
//! `cargo run -p pb-dash-web -- [--empty] [--read-only]`

use pb_dash_model::{Layout, PointState, Series, ServerMsg, Tile, Widget};
use serde_json::{json, Value};
use std::collections::BTreeMap;

use crate::state::{Cmd, Shared};

const POINTS: [&str; 8] = ["pressure", "flow", "tank_level", "temperature", "pump_run", "mode", "energy_kwh", "battery_v"];

pub struct Demo {
    dashboards: BTreeMap<String, String>,
    read_only: bool,
    t0: std::time::Instant,
    last: f64,
    started: bool,
    /// Theme the sample dashboard starts with (`--theme=<key>`).
    pub theme: String,
}

pub fn wall_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Value of a demo point `t` seconds after the demo started.
pub fn value(name: &str, t: f64) -> Value {
    match name {
        "pressure" => json!(((3000.0 + 400.0 * (t / 20.0).sin()) * 10.0).round() / 10.0),
        "flow" => json!(((80.0 + 25.0 * (t / 7.0 + 1.0).sin()) * 10.0).round() / 10.0),
        "tank_level" => json!(((50.0 + 38.0 * (t / 45.0).sin()) * 10.0).round() / 10.0),
        "temperature" => json!(((21.0 + 3.0 * (t / 30.0).sin()) * 10.0).round() / 10.0),
        "pump_run" => json!((t / 10.0) as i64 % 2 == 0),
        "mode" => json!(if (t / 25.0) as i64 % 2 == 0 { "AUTO" } else { "MANUAL" }),
        "energy_kwh" => json!(12000.0 + t * 0.4),
        _ => json!(13.0 + 0.4 * (t / 12.0).sin()),
    }
}

fn tile(id: u32, widget: Widget, x: u32, y: u32, w: u32, h: u32, f: impl FnOnce(&mut Tile)) -> Tile {
    let mut t = Tile::new(id, widget);
    t.x = x;
    t.y = y;
    t.w = w;
    t.h = h;
    f(&mut t);
    t
}

fn series(point: &str, label: &str, axis: u8, style: &str) -> Series {
    Series { point: point.into(), label: label.into(), color: String::new(), axis, style: style.into() }
}

pub fn sample_layout() -> Layout {
    let mut l = Layout::default();
    l.tiles = vec![
        tile(1, Widget::Label, 0, 0, 12, 1, |t| t.text = "Pump station 3".into()),
        tile(2, Widget::Numeric, 0, 1, 5, 3, |t| {
            t.point = "pressure".into();
            t.title = "Discharge pressure".into();
            t.units = "psi".into();
            t.warn_hi = Some(3300.0);
            t.alarm_hi = Some(3380.0);
            t.warn_lo = Some(2700.0);
            t.alarm_lo = Some(2640.0);
        }),
        tile(3, Widget::Gauge, 5, 1, 4, 4, |t| {
            t.point = "flow".into();
            t.title = "Flow".into();
            t.units = "gpm".into();
            t.min = Some(0.0);
            t.max = Some(140.0);
            t.warn_hi = Some(100.0);
            t.alarm_hi = Some(115.0);
        }),
        tile(4, Widget::Bar, 9, 1, 6, 2, |t| {
            t.point = "tank_level".into();
            t.title = "Tank level".into();
            t.units = "%".into();
            t.warn_lo = Some(20.0);
            t.decimals = Some(1);
        }),
        tile(5, Widget::Indicator, 9, 3, 3, 2, |t| {
            t.point = "pump_run".into();
            t.title = "Pump".into();
            t.on_text = "RUNNING".into();
            t.off_text = "STOPPED".into();
        }),
        tile(6, Widget::Text, 12, 3, 3, 2, |t| {
            t.point = "mode".into();
            t.title = "Mode".into();
        }),
        tile(7, Widget::Numeric, 0, 4, 5, 2, |t| {
            t.point = "temperature".into();
            t.title = "Temperature".into();
            t.units = "°C".into();
            t.decimals = Some(1);
        }),
        tile(8, Widget::Chart, 15, 0, 9, 6, |t| {
            t.title = "Pressure & flow".into();
            t.window_secs = 300;
            t.series = vec![series("pressure", "Pressure (psi)", 0, "area"), series("flow", "Flow (gpm)", 1, "")];
        }),
    ];
    l
}

impl Demo {
    pub fn new(empty: bool, read_only: bool) -> Demo {
        let mut dashboards = BTreeMap::new();
        if !empty {
            dashboards.insert("Main".to_string(), sample_layout().to_json());
            dashboards.insert("Tank farm".to_string(), Layout::default().to_json());
        }
        Demo { dashboards, read_only, t0: std::time::Instant::now(), last: -10.0, started: false, theme: String::new() }
    }

    fn t(&self) -> f64 {
        self.t0.elapsed().as_secs_f64() + 600.0 // pretend we've been running for ten minutes
    }

    /// Play the server for one frame: run queued commands and emit new point values.
    pub fn pump(&mut self, sh: &mut Shared, _now: f64) {
        if !self.started {
            self.started = true;
            sh.outbox.push(Cmd::List);
            sh.connected = true;
        }
        for cmd in std::mem::take(&mut sh.outbox) {
            match cmd {
                Cmd::List => sh.on_list(self.dashboards.keys().cloned().collect(), "Demo plant".into(), !self.read_only),
                Cmd::Load(name) => {
                    let text = self.dashboards.get(&name).cloned().map(|t| match Layout::from_json(&t) {
                        Ok(mut l) if l.theme.is_empty() && !self.theme.is_empty() => {
                            l.theme = self.theme.clone();
                            l.to_json()
                        }
                        _ => t,
                    });
                    sh.on_layout(&name, text.as_deref());
                }
                Cmd::Save(name, json) => {
                    if !self.read_only {
                        self.dashboards.insert(name, json);
                    }
                }
                Cmd::Delete(name) => {
                    if !self.read_only {
                        self.dashboards.remove(&name);
                    }
                }
                Cmd::History(points, _) => {
                    let now_t = self.t();
                    for p in points {
                        let rows: Vec<(u64, f64)> = (0..600u64)
                            .filter_map(|i| {
                                let ago = 600.0 - i as f64;
                                Some((wall_ms().saturating_sub((ago * 1000.0) as u64), crate::state::as_f64(&value(&p, now_t - ago))?))
                            })
                            .collect();
                        sh.on_history(&p, rows);
                    }
                }
            }
        }
        let t = self.t();
        if t - self.last >= 0.5 {
            self.last = t;
            let ts = wall_ms();
            let mut points = BTreeMap::new();
            for p in POINTS {
                points.insert(p.to_string(), PointState { v: value(p, t), q: 0, ts, u: String::new() });
            }
            let msg = if sh.points.is_empty() { ServerMsg::Snap { title: "Demo plant".into(), points } } else { ServerMsg::Chg { points } };
            sh.apply(&serde_json::to_string(&msg).unwrap_or_default());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sample_dashboard_is_a_valid_layout() {
        let l = sample_layout();
        l.validate().unwrap();
        let mut n = l.clone();
        n.normalize();
        assert_eq!(n, l, "no overlaps, nothing below its minimum size");
        assert_eq!(Layout::from_json(&l.to_json()).unwrap(), l);
    }

    #[test]
    fn demo_serves_dashboards_and_history() {
        let mut d = Demo::new(false, false);
        let mut sh = Shared::new();
        d.pump(&mut sh, 0.0);
        assert!(sh.names.contains(&"Main".to_string()) && sh.connected && sh.points.len() == POINTS.len());
        sh.open("Main");
        d.pump(&mut sh, 0.1);
        assert_eq!(sh.layout.tiles.len(), sample_layout().tiles.len());
        d.pump(&mut sh, 0.2);
        assert!(sh.history["pressure"].len() > 100, "history for the charted points arrived");
    }
}
