//! Types shared by the `pb-dashboard` pack and the wasm browser app, so the schema can't drift.
//!
//! * [`Layout`] — one saved dashboard (a grid of [`Tile`]s), stored as JSON by the pack.
//! * [`ServerMsg`] / [`ClientMsg`] — the WebSocket protocol.
//!
//! Pure data (serde only): must compile for `wasm32-unknown-unknown`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Layout JSON schema version.
pub const LAYOUT_VERSION: u32 = 1;
/// Grid columns; cell size is page width / `COLS`.
pub const COLS: u32 = 24;

pub mod grid;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    pub v: u32,
    #[serde(default = "default_cols")]
    pub cols: u32,
    /// Colour theme name (see the browser app's theme list); empty = default.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub theme: String,
    /// Size (px) of the point names / titles on tiles; 0 = default. Big screens want bigger.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub title_size: u32,
    #[serde(default)]
    pub tiles: Vec<Tile>,
}

fn default_cols() -> u32 {
    COLS
}

impl Default for Layout {
    fn default() -> Self {
        Self { v: LAYOUT_VERSION, cols: COLS, theme: String::new(), title_size: 0, tiles: Vec::new() }
    }
}

impl Layout {
    /// Parse a stored layout; rejects unknown future versions, fixes nothing silently.
    pub fn from_json(text: &str) -> Result<Layout, String> {
        let l: Layout = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if l.v == 0 || l.v > LAYOUT_VERSION {
            return Err(format!("layout version {} not supported (this build reads up to {LAYOUT_VERSION})", l.v));
        }
        l.validate()?;
        Ok(l)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".into())
    }

    /// Structural checks the pack runs on every save (the browser is not trusted).
    pub fn validate(&self) -> Result<(), String> {
        if self.cols == 0 || self.cols > 96 {
            return Err(format!("cols {} out of range 1–96", self.cols));
        }
        if self.title_size > 96 {
            return Err(format!("title size {} out of range (max 96)", self.title_size));
        }
        if self.tiles.len() > 500 {
            return Err("too many tiles (max 500)".into());
        }
        let mut seen = std::collections::BTreeSet::new();
        for t in &self.tiles {
            if !seen.insert(t.id) {
                return Err(format!("duplicate tile id {}", t.id));
            }
            if t.w == 0 || t.h == 0 || t.x + t.w > self.cols || t.h > 1000 || t.y > 100_000 {
                return Err(format!("tile {} has a bad rectangle", t.id));
            }
            if t.series.len() > 8 {
                return Err(format!("tile {} has too many chart series (max 8)", t.id));
            }
        }
        Ok(())
    }
}

/// One widget on the grid; position and size are in cells. Everything past `point` is
/// presentation and optional: a tile with only the basics must always render sensibly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tile {
    pub id: u32,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub widget: Widget,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// Point shown by single-point widgets.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub point: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub units: String,
    /// Decimal places for numbers; `None` = as the point reports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decimals: Option<u8>,
    /// Range for gauge / bar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// Colour zones: value past `warn_*` turns amber, past `alarm_*` red.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warn_lo: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warn_hi: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alarm_lo: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alarm_hi: Option<f64>,
    /// Indicator texts for true / false.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub on_text: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub off_text: String,
    /// Static text of a Label tile.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    /// Chart series (a chart tile ignores `point`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub series: Vec<Series>,
    /// Chart time window in seconds (0 = default 300).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub window_secs: u32,
    /// Numeric tiles draw a faint trend line of recent history unless this is set.
    #[serde(default, skip_serializing_if = "is_false")]
    pub no_trend: bool,
}

fn is_zero(v: &u32) -> bool {
    *v == 0
}

fn is_false(v: &bool) -> bool {
    !*v
}

impl Tile {
    /// A fresh tile of `widget` at the default size, not yet placed (x = y = 0).
    pub fn new(id: u32, widget: Widget) -> Tile {
        let (w, h) = default_size(widget);
        Tile {
            id,
            x: 0,
            y: 0,
            w,
            h,
            widget,
            title: String::new(),
            point: String::new(),
            units: String::new(),
            decimals: None,
            min: None,
            max: None,
            warn_lo: None,
            warn_hi: None,
            alarm_lo: None,
            alarm_hi: None,
            on_text: String::new(),
            off_text: String::new(),
            text: String::new(),
            series: Vec::new(),
            window_secs: 0,
            no_trend: false,
        }
    }
}

/// One line on a chart.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Series {
    pub point: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// `#rrggbb`; empty = next palette colour.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub color: String,
    /// 0 = left axis, 1 = right axis.
    #[serde(default)]
    pub axis: u8,
    /// `line` (default), `step` or `area`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub style: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Widget {
    Numeric,
    Text,
    Gauge,
    Bar,
    Indicator,
    Chart,
    /// Static text (a heading or note); no point.
    Label,
}

impl Widget {
    pub const ALL: [Widget; 7] = [Widget::Numeric, Widget::Gauge, Widget::Bar, Widget::Indicator, Widget::Chart, Widget::Text, Widget::Label];

    pub fn title(self) -> &'static str {
        match self {
            Widget::Numeric => "Numeric",
            Widget::Text => "Text",
            Widget::Gauge => "Gauge",
            Widget::Bar => "Bar",
            Widget::Indicator => "Indicator",
            Widget::Chart => "Chart",
            Widget::Label => "Label",
        }
    }
}

/// The widget that best shows a point, judged from its current value, units and name:
/// booleans and on / off words → indicator, other text → text, percentages and levels → bar,
/// everything else numeric → numeric.
pub fn suggest_widget(v: &serde_json::Value, units: &str, name: &str) -> Widget {
    use serde_json::Value;
    let on_off = |s: &str| {
        matches!(
            s.trim().to_ascii_lowercase().as_str(),
            "true" | "false" | "on" | "off" | "yes" | "no" | "running" | "stopped" | "open" | "closed" | "active" | "inactive"
        )
    };
    match v {
        Value::Bool(_) => Widget::Indicator,
        Value::String(s) if on_off(s) => Widget::Indicator,
        Value::String(s) if s.trim().parse::<f64>().is_err() => Widget::Text,
        _ => {
            let n = name.to_ascii_lowercase();
            if units.trim() == "%" || n.contains("level") || n.contains("percent") || n.ends_with("_pct") {
                Widget::Bar
            } else {
                Widget::Numeric
            }
        }
    }
}

/// Smallest size (cells) a widget stays usable at.
pub fn min_size(w: Widget) -> (u32, u32) {
    match w {
        Widget::Numeric => (2, 2),
        Widget::Text => (2, 1),
        Widget::Gauge => (3, 3),
        Widget::Bar => (3, 2),
        Widget::Indicator => (2, 2),
        Widget::Chart => (4, 3),
        Widget::Label => (2, 1),
    }
}

/// Size a new tile starts at.
pub fn default_size(w: Widget) -> (u32, u32) {
    match w {
        Widget::Numeric => (4, 3),
        Widget::Text => (4, 2),
        Widget::Gauge => (4, 4),
        Widget::Bar => (6, 2),
        Widget::Indicator => (3, 2),
        Widget::Chart => (8, 5),
        Widget::Label => (6, 2),
    }
}

/// Latest state of one point as the browser sees it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PointState {
    /// Value as JSON (number, bool or string); `null` before the first value.
    pub v: serde_json::Value,
    /// 0 = good, otherwise bad / stale (host quality code).
    #[serde(default)]
    pub q: i32,
    /// Unix milliseconds.
    pub ts: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub u: String,
}

/// Server → browser.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ServerMsg {
    /// Sent once on connect: every point the connection exposes.
    Snap { title: String, points: BTreeMap<String, PointState> },
    /// Batched changes at the Output interval.
    Chg { points: BTreeMap<String, PointState> },
}

/// Browser → server. (Control writes arrive with phase 5; reserved so old pages stay valid.)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ClientMsg {
    Hello,
}

/// Dashboard names become file names: letters, digits, `-`, `_`, space; 1–48 chars; no dots.
pub fn valid_dashboard_name(n: &str) -> bool {
    !n.is_empty() && n.chars().count() <= 48 && n == n.trim() && n.chars().all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | ' '))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Layout {
        Layout {
            tiles: vec![Tile { x: 0, y: 0, w: 6, h: 4, title: "Pressure".into(), point: "psi".into(), ..Tile::new(1, Widget::Gauge) }],
            ..Layout::default()
        }
    }

    #[test]
    fn layout_round_trips_and_uses_the_documented_shape() {
        let l = sample();
        let j = l.to_json();
        assert!(j.contains(r#""widget":"gauge""#) && j.contains(r#""v":1"#), "{j}");
        assert_eq!(Layout::from_json(&j).unwrap(), l);
    }

    #[test]
    fn layouts_from_older_builds_still_load() {
        // Fields that were dropped (dashboard "compact", per-tile "color") are ignored.
        let l = Layout::from_json(r##"{"v":1,"compact":true,"tiles":[{"id":1,"x":0,"y":0,"w":4,"h":3,"widget":"numeric","color":"#ff0000"}]}"##).unwrap();
        assert_eq!(l.tiles.len(), 1);
    }

    #[test]
    fn minimal_layout_gets_defaults() {
        let l = Layout::from_json(r#"{"v":1}"#).unwrap();
        assert_eq!((l.cols, l.tiles.len()), (COLS, 0));
    }

    #[test]
    fn bad_layouts_are_rejected() {
        assert!(Layout::from_json(r#"{"v":99}"#).is_err());
        assert!(Layout::from_json("not json").is_err());
        let mut l = sample();
        l.tiles.push(l.tiles[0].clone());
        assert!(l.validate().unwrap_err().contains("duplicate"));
        let mut l = sample();
        l.tiles[0].x = 20; // 20 + 6 > 24
        assert!(l.validate().unwrap_err().contains("bad rectangle"));
        l.tiles[0].w = 0;
        assert!(l.validate().is_err());
    }

    #[test]
    fn wire_messages_are_tagged() {
        let mut points = BTreeMap::new();
        points.insert("psi".to_string(), PointState { v: serde_json::json!(3400), q: 0, ts: 5, u: "psi".into() });
        let m = ServerMsg::Chg { points };
        let j = serde_json::to_string(&m).unwrap();
        assert!(j.starts_with(r#"{"t":"chg""#), "{j}");
        assert_eq!(serde_json::from_str::<ServerMsg>(&j).unwrap(), m);
    }

    #[test]
    fn widgets_are_suggested_from_the_data() {
        use serde_json::json;
        assert_eq!(suggest_widget(&json!(true), "", "pump"), Widget::Indicator);
        assert_eq!(suggest_widget(&json!("Running"), "", "pump"), Widget::Indicator);
        assert_eq!(suggest_widget(&json!("AUTO"), "", "mode"), Widget::Text);
        assert_eq!(suggest_widget(&json!(42.5), "psi", "pressure"), Widget::Numeric);
        assert_eq!(suggest_widget(&json!("42.5"), "", "pressure"), Widget::Numeric);
        assert_eq!(suggest_widget(&json!(60), "%", "valve"), Widget::Bar);
        assert_eq!(suggest_widget(&json!(60), "", "tank_level"), Widget::Bar);
        assert_eq!(suggest_widget(&serde_json::Value::Null, "", "x"), Widget::Numeric);
    }

    #[test]
    fn dashboard_names_cannot_escape_the_directory() {
        for ok in ["Pump overview", "a-b_c", "Zone 1"] {
            assert!(valid_dashboard_name(ok), "{ok}");
        }
        for bad in ["", "..", "a/b", "a\\b", "x.json", " lead", "tail ", &"n".repeat(49), "a\0b"] {
            assert!(!valid_dashboard_name(bad), "{bad:?}");
        }
    }
}
