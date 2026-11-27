//! Server-side state behind the dashboard page: saved layouts (one JSON file per dashboard) and
//! a rolling in-memory history of numeric points for charts.

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;

use pb_dash_model::{valid_dashboard_name, Layout};

/// Saved dashboards. With a directory they are `<dir>/<name>.json` files (written atomically);
/// without one (tests, or a service that gave no directory) they live in memory only.
pub struct Store {
    dir: Option<PathBuf>,
    mem: BTreeMap<String, String>,
}

impl Store {
    pub fn new(dir: Option<PathBuf>) -> Store {
        if let Some(d) = &dir {
            let _ = std::fs::create_dir_all(d);
        }
        Store { dir, mem: BTreeMap::new() }
    }

    fn path(&self, name: &str) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join(format!("{name}.json")))
    }

    /// Dashboard names, sorted.
    pub fn list(&self) -> Vec<String> {
        match &self.dir {
            Some(d) => {
                let mut v: Vec<String> = std::fs::read_dir(d)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .filter_map(|e| {
                        let n = e.file_name().to_string_lossy().into_owned();
                        n.strip_suffix(".json").map(str::to_string)
                    })
                    .filter(|n| valid_dashboard_name(n))
                    .collect();
                v.sort_by_key(|n| n.to_lowercase());
                v
            }
            None => self.mem.keys().cloned().collect(),
        }
    }

    pub fn get(&self, name: &str) -> Option<String> {
        if !valid_dashboard_name(name) {
            return None;
        }
        match self.path(name) {
            Some(p) => std::fs::read_to_string(p).ok(),
            None => self.mem.get(name).cloned(),
        }
    }

    /// Validate and save. The text is re-serialised so what is stored is always well-formed.
    pub fn put(&mut self, name: &str, text: &str) -> Result<(), String> {
        if !valid_dashboard_name(name) {
            return Err("bad dashboard name (letters, digits, space, - and _; up to 48)".into());
        }
        let layout = Layout::from_json(text)?;
        if self.list().len() >= 64 && self.get(name).is_none() {
            return Err("too many dashboards (max 64)".into());
        }
        let out = layout.to_json();
        match self.path(name) {
            Some(p) => {
                let tmp = p.with_extension("json.tmp");
                std::fs::write(&tmp, &out).map_err(|e| e.to_string())?;
                std::fs::rename(&tmp, &p).map_err(|e| e.to_string())?;
            }
            None => {
                self.mem.insert(name.to_string(), out);
            }
        }
        Ok(())
    }

    pub fn delete(&mut self, name: &str) -> bool {
        if !valid_dashboard_name(name) {
            return false;
        }
        match self.path(name) {
            Some(p) => std::fs::remove_file(p).is_ok(),
            None => self.mem.remove(name).is_some(),
        }
    }
}

/// Rolling history of numeric points: `(ms, value)` thinned to one sample per `min_gap_ms`,
/// at most `cap` samples per point and `max_points` tracked points.
pub struct History {
    min_gap_ms: u64,
    cap: usize,
    max_points: usize,
    series: BTreeMap<String, VecDeque<(u64, f64)>>,
}

impl History {
    pub fn new(minutes: u32, min_gap_ms: u64) -> History {
        let cap = ((minutes.max(1) as u64 * 60_000) / min_gap_ms.max(1)).clamp(60, 100_000) as usize;
        History { min_gap_ms: min_gap_ms.max(1), cap, max_points: 500, series: BTreeMap::new() }
    }

    pub fn record(&mut self, point: &str, ts: u64, v: f64) {
        if !v.is_finite() {
            return;
        }
        if !self.series.contains_key(point) && self.series.len() >= self.max_points {
            return;
        }
        let s = self.series.entry(point.to_string()).or_default();
        if let Some(&(last, _)) = s.back() {
            if ts < last.saturating_add(self.min_gap_ms) {
                // Inside the gap: keep the newest value for that slot.
                if let Some(b) = s.back_mut() {
                    b.1 = v;
                }
                return;
            }
        }
        s.push_back((ts, v));
        while s.len() > self.cap {
            s.pop_front();
        }
    }

    /// Samples of `point` at or after `since`, reduced to at most about `max` by min / max
    /// bucketing (so spikes survive).
    pub fn query(&self, point: &str, since: u64, max: usize) -> Vec<(u64, f64)> {
        let Some(s) = self.series.get(point) else { return Vec::new() };
        let v: Vec<(u64, f64)> = s.iter().copied().filter(|(t, _)| *t >= since).collect();
        downsample(&v, max.max(4))
    }
}

/// Reduce to ~`max` samples: keep everything when short enough, else split into `max / 2`
/// buckets and keep each bucket's min and max in time order.
pub fn downsample(v: &[(u64, f64)], max: usize) -> Vec<(u64, f64)> {
    if v.len() <= max {
        return v.to_vec();
    }
    let buckets = (max / 2).max(1);
    let per = v.len().div_ceil(buckets);
    let mut out = Vec::with_capacity(max);
    for chunk in v.chunks(per) {
        let (mut lo, mut hi) = (chunk[0], chunk[0]);
        for &s in chunk {
            if s.1 < lo.1 {
                lo = s;
            }
            if s.1 > hi.1 {
                hi = s;
            }
        }
        if lo.0 <= hi.0 {
            out.push(lo);
            if hi != lo {
                out.push(hi);
            }
        } else {
            out.push(hi);
            out.push(lo);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout_json() -> String {
        Layout::default().to_json()
    }

    #[test]
    fn memory_store_round_trip() {
        let mut s = Store::new(None);
        assert!(s.list().is_empty());
        s.put("Main", &layout_json()).unwrap();
        s.put("Pumps 2", &layout_json()).unwrap();
        assert_eq!(s.list(), vec!["Main", "Pumps 2"]);
        assert!(s.get("Main").unwrap().contains(r#""v":1"#));
        assert!(s.delete("Main"));
        assert!(!s.delete("Main"));
        assert!(s.get("Main").is_none());
    }

    #[test]
    fn file_store_is_atomic_and_rejects_bad_input() {
        let dir = std::env::temp_dir().join(format!("pb-dash-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut s = Store::new(Some(dir.clone()));
        s.put("Main", &layout_json()).unwrap();
        assert!(dir.join("Main.json").is_file());
        assert!(!dir.join("Main.json.tmp").exists());
        assert_eq!(s.list(), vec!["Main"]);
        // Path tricks and garbage never reach the disk.
        for bad in ["../evil", "a/b", "", ".hidden"] {
            assert!(s.put(bad, &layout_json()).is_err(), "{bad}");
            assert!(s.get(bad).is_none());
            assert!(!s.delete(bad));
        }
        assert!(s.put("Ok", "not json").is_err());
        assert!(s.put("Ok", r#"{"v":99}"#).is_err());
        assert_eq!(s.list(), vec!["Main"]);
        // A fresh store on the same directory sees what was saved.
        let s2 = Store::new(Some(dir.clone()));
        assert!(s2.get("Main").is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn history_thins_caps_and_downsamples() {
        let mut h = History::new(1, 1000); // 1 min at 1 sample/s = 60
        for i in 0..200u64 {
            h.record("p", i * 100, i as f64); // 10 per second: thinned to ~1/s
        }
        let all = h.query("p", 0, 1000);
        assert!(all.len() <= 25, "thinned to one sample per second, got {}", all.len());
        for i in 0..200u64 {
            h.record("q", i * 1000, i as f64);
        }
        assert_eq!(h.query("q", 0, 1000).len(), 60, "capped at 60");
        assert_eq!(h.query("q", 0, 1000).last().unwrap().1, 199.0);
        h.record("nan", 1, f64::NAN);
        assert!(h.query("nan", 0, 10).is_empty());
        assert!(h.query("missing", 0, 10).is_empty());
        assert!(h.query("q", 190_000, 100).iter().all(|(t, _)| *t >= 190_000));
    }

    #[test]
    fn downsampling_keeps_spikes_in_time_order() {
        let mut v: Vec<(u64, f64)> = (0..1000u64).map(|i| (i, 10.0)).collect();
        v[500].1 = 99.0;
        v[501].1 = -5.0;
        let d = downsample(&v, 40);
        assert!(d.len() <= 42, "{}", d.len());
        assert!(d.iter().any(|s| s.1 == 99.0) && d.iter().any(|s| s.1 == -5.0));
        assert!(d.windows(2).all(|w| w[0].0 <= w[1].0), "time order");
        assert_eq!(downsample(&v[..10], 40).len(), 10);
    }
}
