//! Everything the page knows, shared between the UI and the network layer (wasm: fetch +
//! WebSocket; native demo: an in-memory backend). The UI never talks to the network itself: it
//! reads this state and queues [`Cmd`]s in `outbox`.

use pb_dash_model::{Layout, PointState, ServerMsg, Widget};
use serde_json::Value;
use std::collections::BTreeMap;

/// Something the network layer should do.
#[derive(Debug, Clone, PartialEq)]
pub enum Cmd {
    List,
    Load(String),
    Save(String, String),
    Delete(String),
    /// History for these points since this server time (ms).
    History(Vec<String>, u64),
}

#[derive(Default)]
pub struct Shared {
    pub connected: bool,
    pub title: String,
    pub messages: u64,
    pub last_error: String,
    /// Latest value of every point the server exposes.
    pub points: BTreeMap<String, PointState>,
    /// Rolling numeric history per point `(server ms, value)` for charts.
    pub history: BTreeMap<String, Vec<(u64, f64)>>,
    /// Newest server timestamp seen: "now" for charts (immune to browser / server clock skew).
    pub server_now: u64,
    pub names: Vec<String>,
    pub names_loaded: bool,
    pub can_edit: bool,
    pub current: String,
    pub layout: Layout,
    pub layout_loaded: bool,
    /// Seconds (UI clock) of the last layout change not yet saved.
    pub dirty_at: Option<f64>,
    pub outbox: Vec<Cmd>,
    /// Layout changes since load; the UI snapshots / undoes through this.
    pub undo: Vec<Layout>,
    pub redo: Vec<Layout>,
    /// Points whose server history was already requested (asked once per page load).
    pub history_asked: std::collections::BTreeSet<String>,
}

/// History kept per point in the browser.
const HISTORY_CAP: usize = 7200;

impl Shared {
    pub fn new() -> Shared {
        Shared { can_edit: true, ..Shared::default() }
    }

    /// One text frame from the live feed.
    pub fn apply(&mut self, text: &str) {
        self.messages += 1;
        match serde_json::from_str::<ServerMsg>(text) {
            Ok(ServerMsg::Snap { title, points }) => {
                self.title = title;
                self.points.clear();
                self.merge_points(points);
            }
            Ok(ServerMsg::Chg { points }) => self.merge_points(points),
            Err(e) => self.last_error = format!("bad message: {e}"),
        }
    }

    fn merge_points(&mut self, points: BTreeMap<String, PointState>) {
        for (name, st) in points {
            self.server_now = self.server_now.max(st.ts);
            if let Some(v) = as_f64(&st.v) {
                let h = self.history.entry(name.clone()).or_default();
                // One sample per second: inside the current second only the value is updated. The
                // slot's time must stay put — moving it forward with every update meant a point
                // refreshed faster than once a second never added a new sample at all.
                match h.last_mut() {
                    Some(last) if st.ts < last.0 + 1000 => last.1 = v,
                    _ => h.push((st.ts, v)),
                }
                if h.len() > HISTORY_CAP {
                    h.drain(..h.len() - HISTORY_CAP);
                }
            }
            self.points.insert(name, st);
        }
    }

    /// Server history arrived for a point: merge it in front of what the live feed collected.
    pub fn on_history(&mut self, point: &str, rows: Vec<(u64, f64)>) {
        let live = self.history.remove(point).unwrap_or_default();
        let first_live = live.first().map(|r| r.0).unwrap_or(u64::MAX);
        let mut merged: Vec<(u64, f64)> = rows.into_iter().filter(|r| r.0 < first_live).collect();
        merged.extend(live);
        if let Some(t) = merged.last().map(|r| r.0) {
            self.server_now = self.server_now.max(t);
        }
        self.history.insert(point.to_string(), merged);
    }

    pub fn on_list(&mut self, names: Vec<String>, title: String, can_edit: bool) {
        self.names = names;
        self.names_loaded = true;
        self.can_edit = can_edit;
        if self.title.is_empty() {
            self.title = title;
        }
    }

    /// A layout arrived (or `None` = no such dashboard). Ignored if the user already moved on.
    pub fn on_layout(&mut self, name: &str, text: Option<&str>) {
        if name != self.current {
            return;
        }
        match text.map(Layout::from_json) {
            Some(Ok(l)) => {
                self.layout = l;
                self.last_error.clear();
            }
            Some(Err(e)) => {
                self.layout = Layout::default();
                self.last_error = format!("dashboard '{name}': {e}");
            }
            None => self.layout = Layout::default(),
        }
        self.layout_loaded = true;
        self.dirty_at = None;
        self.undo.clear();
        self.redo.clear();
        self.queue_history();
    }

    /// Ask for server history of every charted point (and numeric tiles' trend lines) not
    /// asked for yet since this page loaded.
    pub fn queue_history(&mut self) {
        let mut pts: Vec<String> = Vec::new();
        for t in &self.layout.tiles {
            let wanted: Vec<&String> = match t.widget {
                Widget::Chart => t.series.iter().map(|s| &s.point).collect(),
                Widget::Numeric if !t.no_trend => vec![&t.point],
                _ => vec![],
            };
            for p in wanted {
                if !p.is_empty() && !pts.contains(p) && !self.history_asked.contains(p) {
                    pts.push(p.clone());
                }
            }
        }
        if !pts.is_empty() {
            self.history_asked.extend(pts.iter().cloned());
            self.outbox.push(Cmd::History(pts, 0));
        }
    }

    /// Every point that can go on a dashboard, with the widget its data suggests and its units.
    pub fn point_suggestions(&self) -> Vec<(String, Widget, String)> {
        self.points.iter().map(|(n, st)| (n.clone(), pb_dash_model::suggest_widget(&st.v, &st.u, n), st.u.clone())).collect()
    }

    /// Switch dashboards (loads it from the server).
    pub fn open(&mut self, name: &str) {
        if name == self.current && self.layout_loaded {
            return;
        }
        self.flush_save(); // don't lose a pending edit of the one we are leaving
        self.current = name.to_string();
        self.layout_loaded = false;
        self.layout = Layout::default();
        self.outbox.push(Cmd::Load(name.to_string()));
    }

    /// Record a layout edit: snapshot for undo, mark for the debounced save.
    pub fn edited(&mut self, before: Layout, now: f64) {
        if self.undo.last() != Some(&before) {
            self.undo.push(before);
            if self.undo.len() > 100 {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.dirty_at = Some(now);
        self.queue_history();
    }

    pub fn undo(&mut self, now: f64) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.layout, prev));
            self.dirty_at = Some(now);
        }
    }

    pub fn redo(&mut self, now: f64) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.layout, next));
            self.dirty_at = Some(now);
        }
    }

    /// Send the save now when an edit is pending.
    pub fn flush_save(&mut self) {
        if self.dirty_at.take().is_some() && self.layout_loaded && !self.current.is_empty() && self.can_edit {
            self.outbox.push(Cmd::Save(self.current.clone(), self.layout.to_json()));
        }
    }

    /// Debounced save: call every frame with the UI clock.
    pub fn save_when_idle(&mut self, now: f64) {
        if matches!(self.dirty_at, Some(t) if now - t >= 0.6) {
            self.flush_save();
        }
    }

    pub fn create_dashboard(&mut self, name: &str, copy_of_current: bool) {
        self.flush_save();
        let layout = if copy_of_current { self.layout.clone() } else { Layout::default() };
        self.outbox.push(Cmd::Save(name.to_string(), layout.to_json()));
        if !self.names.iter().any(|n| n == name) {
            self.names.push(name.to_string());
            self.names.sort_by_key(|n| n.to_lowercase());
        }
        self.current = name.to_string();
        self.layout = layout;
        self.layout_loaded = true;
        self.undo.clear();
        self.redo.clear();
    }

    pub fn rename_dashboard(&mut self, to: &str) {
        self.flush_save();
        let from = self.current.clone();
        if from == to || from.is_empty() {
            return;
        }
        self.outbox.push(Cmd::Save(to.to_string(), self.layout.to_json()));
        self.outbox.push(Cmd::Delete(from.clone()));
        self.names.retain(|n| *n != from);
        self.names.push(to.to_string());
        self.names.sort_by_key(|n| n.to_lowercase());
        self.current = to.to_string();
    }

    pub fn delete_dashboard(&mut self) {
        let gone = self.current.clone();
        self.dirty_at = None;
        self.outbox.push(Cmd::Delete(gone.clone()));
        self.names.retain(|n| *n != gone);
        match self.names.first().cloned() {
            Some(next) => {
                self.current.clear();
                self.layout_loaded = false;
                self.open(&next);
            }
            None => self.create_dashboard("Main", false),
        }
    }
}

pub fn as_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(points: &str) -> String {
        format!(r#"{{"t":"snap","title":"Plant","points":{points}}}"#)
    }

    #[test]
    fn live_messages_update_points_and_history() {
        let mut s = Shared::new();
        s.apply(&snap(r#"{"psi":{"v":10,"q":0,"ts":1000}}"#));
        s.apply(r#"{"t":"chg","points":{"psi":{"v":11,"q":0,"ts":1200},"mode":{"v":"auto","q":0,"ts":1200}}}"#);
        s.apply(r#"{"t":"chg","points":{"psi":{"v":12,"q":0,"ts":2500}}}"#);
        assert_eq!((s.title.as_str(), s.points.len(), s.messages, s.server_now), ("Plant", 2, 3, 2500));
        assert_eq!(s.history["psi"], vec![(1000, 11.0), (2500, 12.0)], "thinned to one per second");
        assert!(!s.history.contains_key("mode"), "text has no history");
        s.apply("garbage");
        assert!(s.last_error.starts_with("bad message"));
    }

    #[test]
    fn fast_updates_still_build_a_history() {
        // Regression: a value refreshed every 250 ms used to keep a single, ever-moving sample.
        let mut s = Shared::new();
        for i in 0..40u64 {
            s.apply(&format!(r#"{{"t":"chg","points":{{"p":{{"v":{i},"q":0,"ts":{}}}}}}}"#, 10_000 + i * 250));
        }
        let h = &s.history["p"];
        assert_eq!(h.len(), 10, "about one sample per second over 10 s: {h:?}");
        assert!(h.windows(2).all(|w| w[1].0 >= w[0].0 + 1000));
        assert_eq!(h.last().unwrap().1, 39.0, "newest value kept");
    }

    #[test]
    fn server_history_merges_in_front_of_live_data() {
        let mut s = Shared::new();
        s.apply(&snap(r#"{"p":{"v":5,"q":0,"ts":10000}}"#));
        s.on_history("p", vec![(1000, 1.0), (5000, 2.0), (10000, 99.0)]);
        assert_eq!(s.history["p"], vec![(1000, 1.0), (5000, 2.0), (10000, 5.0)]);
    }

    #[test]
    fn layouts_load_save_debounced_and_undo() {
        let mut s = Shared::new();
        s.open("Main");
        assert_eq!(s.outbox.pop(), Some(Cmd::Load("Main".into())));
        s.on_layout("Other", Some(r#"{"v":1}"#));
        assert!(!s.layout_loaded, "a stale answer for another dashboard is ignored");
        s.on_layout("Main", Some(r#"{"v":1}"#));
        assert!(s.layout_loaded);
        let before = s.layout.clone();
        s.layout.add(Widget::Numeric, None);
        s.edited(before, 10.0);
        s.save_when_idle(10.3);
        assert!(s.outbox.is_empty(), "not yet");
        s.save_when_idle(10.7);
        assert!(matches!(s.outbox.pop(), Some(Cmd::Save(n, _)) if n == "Main"));
        s.undo(11.0);
        assert!(s.layout.tiles.is_empty());
        s.redo(11.1);
        assert_eq!(s.layout.tiles.len(), 1);
        // A missing dashboard opens empty; bad JSON shows an error instead of crashing.
        s.on_layout("Main", None);
        assert!(s.layout.tiles.is_empty());
        s.on_layout("Main", Some("{bad"));
        assert!(s.last_error.contains("Main"));
    }

    #[test]
    fn dashboards_can_be_created_renamed_deleted() {
        let mut s = Shared::new();
        s.on_list(vec!["Main".into()], "Plant".into(), true);
        s.open("Main");
        s.on_layout("Main", Some(r#"{"v":1}"#));
        s.outbox.clear();
        s.create_dashboard("Pumps", false);
        assert_eq!(s.names, vec!["Main", "Pumps"]);
        assert!(matches!(&s.outbox[0], Cmd::Save(n, _) if n == "Pumps"));
        s.outbox.clear();
        s.rename_dashboard("Pump room");
        assert_eq!(s.names, vec!["Main", "Pump room"]);
        assert!(matches!(&s.outbox[1], Cmd::Delete(n) if n == "Pumps"));
        s.outbox.clear();
        s.delete_dashboard();
        assert_eq!((s.current.as_str(), s.names.len()), ("Main", 1));
        assert!(s.outbox.contains(&Cmd::Delete("Pump room".into())));
        s.delete_dashboard();
        assert_eq!(s.current, "Main", "the last one is replaced by an empty Main");
    }

    #[test]
    fn read_only_pages_never_queue_saves() {
        let mut s = Shared::new();
        s.on_list(vec![], "x".into(), false);
        s.current = "Main".into();
        s.layout_loaded = true;
        s.dirty_at = Some(0.0);
        s.flush_save();
        assert!(s.outbox.is_empty());
    }

    #[test]
    fn charted_points_request_history() {
        let mut s = Shared::new();
        s.current = "Main".into();
        let id = s.layout.add(Widget::Chart, None);
        s.layout.tile_mut(id).unwrap().series = vec![pb_dash_model::Series { point: "a".into(), label: String::new(), color: String::new(), axis: 0, style: String::new() }];
        s.queue_history();
        assert_eq!(s.outbox, vec![Cmd::History(vec!["a".into()], 0)]);
        s.outbox.clear();
        let n = s.layout.add(Widget::Numeric, None);
        s.layout.tile_mut(n).unwrap().point = "b".into();
        s.queue_history();
        assert_eq!(s.outbox, vec![Cmd::History(vec!["b".into()], 0)], "numeric trend lines too; 'a' is not asked twice");
    }
}
