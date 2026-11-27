//! Painting of the single-point widgets (numeric, text, gauge, bar, indicator, label).
//! Everything is drawn with the painter at whatever size the tile has: fonts are fitted to the
//! tile, so a numeric can be huge on a wall screen and still fit a small tile.

use egui::{Align2, Color32, FontFamily, FontId, Painter, Pos2, Rect, CornerRadius, Shape, Stroke, Vec2};
use egui::epaint::Shadow;
use pb_dash_model::{PointState, Tile, Widget};
use serde_json::Value;

use crate::state::as_f64;

pub use crate::theme::{bold, rgb, Pal};

/// `#rrggbb` → colour.
pub fn parse_color(s: &str) -> Option<Color32> {
    let h = s.trim().strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    u32::from_str_radix(h, 16).ok().map(rgb)
}

/// Series / accent palette (colour-blind friendly order).
pub const SERIES: [u32; 8] = [0x38bdf8, 0xfbbf24, 0x34d399, 0xf472b6, 0xa78bfa, 0xfb923c, 0xf87171, 0x5eead4];

/// A value as text: `decimals` when set; otherwise whole numbers as-is and fractions trimmed.
pub fn fmt_value(v: &Value, decimals: Option<u8>) -> String {
    match v {
        Value::Null => "—".into(),
        Value::Bool(b) => if *b { "ON" } else { "OFF" }.into(),
        Value::String(s) => s.clone(),
        Value::Number(n) => match (decimals, n.as_f64()) {
            (Some(d), Some(x)) => format!("{x:.*}", d as usize),
            (None, Some(x)) => fmt_auto(x),
            _ => n.to_string(),
        },
        other => other.to_string(),
    }
}

pub fn fmt_auto(x: f64) -> String {
    if x.fract() == 0.0 && x.abs() < 1e15 {
        return format!("{}", x as i64);
    }
    let s = format!("{x:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Largest font size (within `min..=max`) at which `text` fits `w × h`.
pub fn fit_font(p: &Painter, text: &str, w: f32, h: f32, family: FontFamily, min: f32, max: f32) -> f32 {
    if text.is_empty() || w <= 1.0 || h <= 1.0 {
        return min;
    }
    let g = p.layout_no_wrap(text.to_string(), FontId::new(100.0, family), Color32::WHITE);
    let s = g.size();
    (100.0 * (w / s.x.max(1.0)).min(h / s.y.max(1.0))).clamp(min, max)
}

/// Accent for a value: red / amber past the tile's thresholds, else the tile colour / default.
pub fn zone_color(t: &Tile, v: f64, base: Color32, pal: &Pal) -> Color32 {
    zone_of(t, v).color(base, pal)
}

/// Which band of the tile's limits a value is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    Normal,
    /// Past a high limit (amber / red).
    WarnHi,
    AlarmHi,
    /// Past a low limit (blue / purple), so too-low never looks like too-high.
    WarnLo,
    AlarmLo,
}

impl Zone {
    pub fn color(self, base: Color32, pal: &Pal) -> Color32 {
        match self {
            Zone::Normal => base,
            Zone::WarnHi => pal.warn,
            Zone::AlarmHi => pal.alarm,
            Zone::WarnLo => pal.warn_lo,
            Zone::AlarmLo => pal.alarm_lo,
        }
    }
    pub fn is_alarm(self) -> bool {
        matches!(self, Zone::AlarmHi | Zone::AlarmLo)
    }
}

/// A point currently past an alarm limit, as listed in the header.
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveAlarm {
    pub point: String,
    /// The name shown: the tile's title, else the readable point name.
    pub name: String,
    pub zone: Zone,
    /// e.g. "3388.1 psi — above the alarm limit 3380".
    pub detail: String,
}

/// Every point in alarm on this dashboard, once each, in reading order. Only number, gauge and
/// bar tiles have limits; values with BAD quality are not judged.
pub fn active_alarms(layout: &pb_dash_model::Layout, points: &std::collections::BTreeMap<String, PointState>) -> Vec<ActiveAlarm> {
    let mut out: Vec<ActiveAlarm> = Vec::new();
    for id in layout.reading_order() {
        let Some(t) = layout.tile(id) else { continue };
        if !matches!(t.widget, Widget::Numeric | Widget::Gauge | Widget::Bar) || t.point.is_empty() || out.iter().any(|a| a.point == t.point) {
            continue;
        }
        let Some(st) = points.get(&t.point).filter(|s| s.q == 0) else { continue };
        let Some(v) = as_f64(&st.v) else { continue };
        let zone = zone_of(t, v);
        if !zone.is_alarm() {
            continue;
        }
        let units = if t.units.is_empty() { st.u.clone() } else { t.units.clone() };
        let (side, limit) = if zone == Zone::AlarmHi { ("above", t.alarm_hi) } else { ("below", t.alarm_lo) };
        let value = fmt_value(&st.v, t.decimals);
        let value = if units.is_empty() { value } else { format!("{value} {units}") };
        let detail = format!("{value} — {side} the alarm limit {}", limit.map(fmt_auto).unwrap_or_default());
        out.push(ActiveAlarm { point: t.point.clone(), name: tile_title(t), zone, detail });
    }
    out
}

pub fn zone_of(t: &Tile, v: f64) -> Zone {
    let above = |h: Option<f64>| h.is_some_and(|h| v >= h);
    let below = |l: Option<f64>| l.is_some_and(|l| v <= l);
    if above(t.alarm_hi) {
        Zone::AlarmHi
    } else if below(t.alarm_lo) {
        Zone::AlarmLo
    } else if above(t.warn_hi) {
        Zone::WarnHi
    } else if below(t.warn_lo) {
        Zone::WarnLo
    } else {
        Zone::Normal
    }
}

/// A tile's normal colour: the theme accent (themes own the colours; tiles only change colour
/// through their warn / alarm zones).
pub fn tile_color(_t: &Tile, pal: &Pal) -> Color32 {
    pal.accent
}

/// A point name made pleasant to read: `tank_level` → `Tank level`, `pump-2.run` → `Pump 2.run`.
/// Names that already look written by a person (spaces or mixed case) are kept as they are.
pub fn pretty_name(n: &str) -> String {
    if n.contains(' ') || (n.chars().any(|c| c.is_lowercase()) && n.chars().any(|c| c.is_uppercase())) {
        return n.to_string();
    }
    let spaced: String = n.chars().map(|c| if c == '_' || c == '-' { ' ' } else { c }).collect();
    let spaced = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut c = spaced.chars();
    match c.next() {
        Some(f) if spaced.chars().any(|c| c.is_lowercase()) => f.to_uppercase().collect::<String>() + c.as_str(),
        _ => spaced,
    }
}

/// Title shown on a tile: its own, else the point name made readable.
pub fn tile_title(t: &Tile) -> String {
    if !t.title.is_empty() {
        t.title.clone()
    } else if t.widget == Widget::Chart {
        String::new()
    } else {
        pretty_name(&t.point)
    }
}

pub const CARD_R: f32 = 14.0;
/// Default size of tile titles (point names). One size per dashboard so it reads evenly.
pub const TITLE_SIZE: f32 = 16.0;

thread_local! {
    static TITLE_PX: std::cell::Cell<f32> = const { std::cell::Cell::new(TITLE_SIZE) };
}

/// Set the dashboard's title size for this frame (`0` = default).
pub fn set_title_size(px: u32) {
    TITLE_PX.with(|c| c.set(if px == 0 { TITLE_SIZE } else { (px as f32).clamp(9.0, 96.0) }));
}

/// Current tile title size in points.
pub fn title_size() -> f32 {
    TITLE_PX.with(|c| c.get())
}

/// The tile background: soft drop shadow, card fill, hairline border.
pub fn card_bg(p: &Painter, rect: Rect, pal: &Pal, border: Color32) {
    let r = CornerRadius::from(CARD_R);
    p.add(Shadow { offset: [0, 6], blur: 22, spread: 0, color: pal.shadow }.as_shape(rect, r));
    p.rect_filled(rect, r, pal.panel);
    p.rect_stroke(rect, r, Stroke::new(1.0, border), egui::StrokeKind::Inside);
}

/// Filled area under a polyline, fading towards `base` (one mesh, so no seams between slices).
pub fn area_mesh(line: &[Pos2], base: f32, col: Color32) -> Shape {
    let mut m = egui::Mesh::default();
    for p in line {
        m.colored_vertex(*p, col);
        m.colored_vertex(Pos2::new(p.x, base), col.gamma_multiply(0.1));
    }
    for i in 0..line.len().saturating_sub(1) {
        let k = (i * 2) as u32;
        m.add_triangle(k, k + 1, k + 2);
        m.add_triangle(k + 1, k + 3, k + 2);
    }
    Shape::mesh(m)
}

fn card_pad(rect: Rect) -> f32 {
    (rect.height() * 0.08).clamp(10.0, 18.0)
}

/// Height from the card's top edge to where its content starts below the title. The edit-mode
/// header covers exactly this band, so no title peeks out from under it.
pub fn title_band(rect: Rect) -> f32 {
    let pad = card_pad(rect);
    pad + title_size() * 1.4 + pad * 0.4
}

/// Draw the tile card and its title; returns the content rect below / inside it.
pub fn card(p: &Painter, rect: Rect, t: &Tile, pal: &Pal, bad: bool) -> Rect {
    card_in_zone(p, rect, t, pal, bad, Zone::Normal)
}

/// [`card`] for a value in `zone`: a warning gets an amber border, an alarm a red border and
/// a faint red wash over the card, so a problem is visible from across the room.
pub fn card_in_zone(p: &Painter, rect: Rect, t: &Tile, pal: &Pal, bad: bool, zone: Zone) -> Rect {
    if t.widget == Widget::Label {
        // Headings sit on the page itself, no card.
        return rect;
    }
    card_bg(p, rect, pal, if bad { pal.warn.gamma_multiply(0.8) } else { pal.line });
    if zone != Zone::Normal {
        let col = zone.color(pal.line, pal);
        if zone.is_alarm() {
            p.rect_filled(rect, CARD_R, col.gamma_multiply(0.10));
        }
        p.rect_stroke(rect.shrink(1.0), CARD_R - 1.0, Stroke::new(if zone.is_alarm() { 2.5 } else { 2.0 }, col), egui::StrokeKind::Inside);
    }
    let title = tile_title(t);
    let pad = card_pad(rect);
    let mut inner = rect.shrink2(Vec2::new(pad + 2.0, pad));
    if !title.is_empty() {
        let size = title_size();
        // Accent pip in the tile's colour, then the title.
        let pip = Rect::from_min_size(inner.left_top() + Vec2::new(0.0, size * 0.18), Vec2::new(4.0, size * 1.0));
        p.rect_filled(pip, 2.0, tile_color(t, pal));
        let max_w = inner.width() - 14.0 - if bad { 44.0 } else { 0.0 };
        let g = p.layout_no_wrap(ellipsize(p, &title, size, max_w.max(20.0)), FontId::proportional(size), pal.soft);
        p.galley(inner.left_top() + Vec2::new(12.0, 0.0), g, pal.soft);
        inner.min.y = rect.min.y + title_band(rect);
    }
    if bad {
        let b = p.layout_no_wrap("BAD".into(), crate::theme::bold_id(10.5), pal.bg);
        let pill = Rect::from_min_size(Pos2::new(rect.max.x - pad - b.size().x - 12.0, rect.min.y + pad - 1.0), b.size() + Vec2::new(12.0, 4.0));
        p.rect_filled(pill, 8.0, pal.warn);
        p.galley(pill.min + Vec2::new(6.0, 2.0), b, pal.bg);
    }
    inner
}

/// `s`, shortened with … to fit `max_w` at `size`.
pub fn ellipsize(p: &Painter, s: &str, size: f32, max_w: f32) -> String {
    if p.layout_no_wrap(s.to_string(), FontId::proportional(size), Color32::WHITE).size().x <= max_w {
        return s.to_string();
    }
    let mut chars: Vec<char> = s.chars().collect();
    while chars.len() > 1 {
        chars.pop();
        let t: String = chars.iter().collect::<String>() + "…";
        if p.layout_no_wrap(t.clone(), FontId::proportional(size), Color32::WHITE).size().x <= max_w {
            return t;
        }
    }
    "…".to_string()
}

/// Paint one single-point widget into `rect` (the whole tile). `hist` is the point's recent
/// numeric history (for the numeric tile's trend line) and `now` the server clock in ms.
pub fn paint(p: &Painter, rect: Rect, t: &Tile, st: Option<&PointState>, hist: Option<&[(u64, f64)]>, now: u64, pal: &Pal) {
    let bad = st.is_some_and(|s| s.q != 0);
    // Limits only mean something for numeric widgets with a number to compare.
    let zone = match t.widget {
        Widget::Numeric | Widget::Gauge | Widget::Bar => st.filter(|s| s.q == 0).and_then(|s| as_f64(&s.v)).map(|v| zone_of(t, v)).unwrap_or(Zone::Normal),
        _ => Zone::Normal,
    };
    let inner = card_in_zone(p, rect, t, pal, bad, zone);
    let accent = tile_color(t, pal);
    let dim = |c: Color32| if bad { c.gamma_multiply(0.55) } else { c };
    match t.widget {
        Widget::Label => label(p, rect, t, pal),
        Widget::Numeric => numeric(p, inner, t, st, hist, now, pal, dim(accent)),
        Widget::Text => text(p, inner, t, st, pal),
        Widget::Gauge => gauge(p, inner, t, st, pal, dim(accent)),
        Widget::Bar => bar(p, inner, t, st, pal, dim(accent)),
        Widget::Indicator => indicator(p, inner, t, st, pal, dim(accent)),
        Widget::Chart => {} // drawn by chart.rs (needs history)
    }
}

fn no_data(p: &Painter, inner: Rect, pal: &Pal) {
    let s = fit_font(p, "—", inner.width() * 0.3, inner.height() * 0.5, bold(), 12.0, 60.0);
    p.text(inner.center(), Align2::CENTER_CENTER, "—", FontId::new(s, bold()), pal.muted);
}

fn label(p: &Painter, rect: Rect, t: &Tile, pal: &Pal) {
    let txt = if t.text.is_empty() { &t.title } else { &t.text };
    let inner = rect.shrink2(Vec2::new(6.0, 4.0));
    let s = fit_font(p, txt, inner.width(), inner.height() * 0.8, bold(), 10.0, 64.0);
    let r = p.text(inner.left_center(), Align2::LEFT_CENTER, txt, FontId::new(s, bold()), pal.text);
    // Underline accent so a heading reads as a section start.
    let y = (r.max.y + 4.0).min(rect.max.y - 1.0);
    p.rect_filled(Rect::from_min_size(Pos2::new(inner.min.x, y), Vec2::new((r.width() * 0.25).clamp(24.0, 80.0), 3.0)), 2.0, tile_color(t, pal));
}


/// Window of history the numeric tile's trend line shows.
const TREND_MS: u64 = 300_000;

/// Faint area + line of recent values along the bottom of `r`.
/// Thresholds set on a tile (warn / alarm, high and low).
fn thresholds(t: &Tile) -> Vec<f64> {
    let mut v: Vec<f64> = [t.warn_lo, t.warn_hi, t.alarm_lo, t.alarm_hi].into_iter().flatten().filter(|x| x.is_finite()).collect();
    v.sort_by(|a, b| a.total_cmp(b));
    v.dedup();
    v
}

/// Cut a polyline of `(x, value)` points wherever it crosses a threshold, so each piece lies in
/// a single zone and can be coloured by it. Returns pieces of `[(x0, v0), (x1, v1)]`.
pub fn split_at_thresholds(pts: &[(f32, f64)], th: &[f64]) -> Vec<[(f32, f64); 2]> {
    let mut out = Vec::new();
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (lo, hi) = (a.1.min(b.1), a.1.max(b.1));
        let mut cuts: Vec<f64> = th.iter().copied().filter(|&c| c > lo && c < hi).collect();
        if b.1 < a.1 {
            cuts.reverse();
        }
        let mut prev = a;
        for c in cuts {
            let f = ((c - a.1) / (b.1 - a.1)) as f32;
            let at = (a.0 + (b.0 - a.0) * f, c);
            out.push([prev, at]);
            prev = at;
        }
        out.push([prev, b]);
    }
    out
}

/// Faint area + line of recent values along the bottom of `r`. The line keeps the tile's normal
/// colour and turns amber / red only where the value was in a warn / alarm zone; thresholds
/// inside the visible range show as faint dashed guides.
fn sparkline(p: &Painter, r: Rect, rows: &[(u64, f64)], now: u64, t: &Tile, base: Color32, pal: &Pal) {
    let t0 = now.saturating_sub(TREND_MS);
    let mut pts: Vec<(f32, f64)> = rows
        .iter()
        .filter(|x| x.0 >= t0)
        .map(|x| ((x.0.saturating_sub(t0)) as f32 / TREND_MS as f32, x.1))
        .collect();
    if pts.len() < 2 {
        return;
    }
    if let Some(&(_, v)) = pts.last() {
        pts.push((1.0, v)); // the value holds until "now"
    }
    let (lo, hi) = pts.iter().fold((f64::MAX, f64::MIN), |a, x| (a.0.min(x.1), a.1.max(x.1)));
    let span = if hi - lo < 1e-9 { 1.0 } else { hi - lo };
    let to = |x: f32, v: f64| Pos2::new(r.min.x + r.width() * x, r.max.y - r.height() * (0.1 + 0.8 * ((v - lo) / span) as f32));
    let th = thresholds(t);
    for (c, col) in [(t.warn_lo, pal.warn_lo), (t.warn_hi, pal.warn), (t.alarm_lo, pal.alarm_lo), (t.alarm_hi, pal.alarm)] {
        let Some(c) = c.filter(|c| *c > lo && *c < hi) else { continue };
        let y = to(0.0, c).y;
        p.extend(Shape::dashed_line(&[Pos2::new(r.min.x, y), Pos2::new(r.max.x, y)], Stroke::new(1.0, col.gamma_multiply(0.45)), 4.0, 4.0));
    }
    let pieces = split_at_thresholds(&pts, &th);
    // One mesh for the fill (no seams), coloured piece by piece.
    let mut m = egui::Mesh::default();
    for [a, b] in &pieces {
        let col = zone_color(t, (a.1 + b.1) / 2.0, base, pal).gamma_multiply(0.22);
        let k = m.vertices.len() as u32;
        let (pa, pb) = (to(a.0, a.1), to(b.0, b.1));
        m.colored_vertex(pa, col);
        m.colored_vertex(Pos2::new(pa.x, r.max.y), col.gamma_multiply(0.1));
        m.colored_vertex(pb, col);
        m.colored_vertex(Pos2::new(pb.x, r.max.y), col.gamma_multiply(0.1));
        m.add_triangle(k, k + 1, k + 2);
        m.add_triangle(k + 1, k + 3, k + 2);
    }
    p.add(Shape::mesh(m));
    // Line: consecutive pieces of the same colour joined into one stroke (clean joins).
    let mut run: Vec<Pos2> = Vec::new();
    let mut run_col: Option<Color32> = None;
    for [a, b] in &pieces {
        let col = zone_color(t, (a.1 + b.1) / 2.0, base, pal);
        if run_col != Some(col) {
            if let Some(c) = run_col {
                p.add(Shape::line(std::mem::take(&mut run), Stroke::new(1.8, c.gamma_multiply(0.85))));
            }
            run_col = Some(col);
            run.push(to(a.0, a.1));
        }
        run.push(to(b.0, b.1));
    }
    if let Some(c) = run_col {
        p.add(Shape::line(run, Stroke::new(1.8, c.gamma_multiply(0.85))));
    }
    if let Some(&(x, v)) = pts.last() {
        p.circle_filled(to(x, v), 3.0, zone_color(t, v, base, pal));
    }
}

#[allow(clippy::too_many_arguments)]
fn numeric(p: &Painter, inner: Rect, t: &Tile, st: Option<&PointState>, hist: Option<&[(u64, f64)]>, now: u64, pal: &Pal, accent: Color32) {
    let Some(st) = st.filter(|s| !s.v.is_null()) else { return no_data(p, inner, pal) };
    let value = fmt_value(&st.v, t.decimals);
    let color = as_f64(&st.v).map(|v| zone_color(t, v, accent, pal)).unwrap_or(accent);
    let units = if t.units.is_empty() { st.u.clone() } else { t.units.clone() };
    // Tall enough: the trend line takes the bottom third and the number sits above it.
    let trend = !t.no_trend && inner.height() > 64.0 && hist.is_some_and(|h| h.len() > 1);
    let (vbox, sbox) = if trend {
        let split = inner.min.y + inner.height() * 0.64;
        (Rect::from_min_max(inner.min, Pos2::new(inner.max.x, split)), Some(Rect::from_min_max(Pos2::new(inner.min.x, split), inner.max)))
    } else {
        (inner, None)
    };
    if let (Some(r), Some(h)) = (sbox, hist) {
        sparkline(p, r.expand2(Vec2::new(4.0, 0.0)), h, now, t, accent, pal);
    }
    // Value and units share one line; the value takes the lion's share of the width.
    let uw = if units.is_empty() { 0.0 } else { vbox.width() * 0.26 };
    let vs = fit_font(p, &value, vbox.width() - uw - 6.0, vbox.height() * 0.95, bold(), 12.0, 400.0);
    let vg = p.layout_no_wrap(value.clone(), FontId::new(vs, bold()), color);
    let us = if units.is_empty() { 0.0 } else { (vs * 0.36).clamp(11.0, 56.0) };
    let ug = (!units.is_empty()).then(|| p.layout_no_wrap(units.clone(), FontId::proportional(us), pal.soft));
    let total = vg.size().x + ug.as_ref().map(|g| g.size().x + 8.0).unwrap_or(0.0);
    let x0 = if trend { vbox.min.x } else { vbox.center().x - total / 2.0 };
    let y = vbox.center().y;
    p.galley(Pos2::new(x0, y - vg.size().y / 2.0), vg.clone(), color);
    if let Some(g) = ug {
        p.galley(Pos2::new(x0 + vg.size().x + 8.0, y + vg.size().y / 2.0 - g.size().y - vs * 0.1), g, pal.soft);
    }
}

fn text(p: &Painter, inner: Rect, t: &Tile, st: Option<&PointState>, pal: &Pal) {
    let Some(st) = st.filter(|s| !s.v.is_null()) else { return no_data(p, inner, pal) };
    let value = fmt_value(&st.v, t.decimals);
    let s = fit_font(p, &value, inner.width(), inner.height() * 0.85, bold(), 11.0, 120.0);
    p.text(inner.center(), Align2::CENTER_CENTER, value, FontId::new(s, bold()), pal.text);
}

/// Range of a gauge / bar: the tile's, else 0..100.
pub fn range_of(t: &Tile) -> (f64, f64) {
    let lo = t.min.unwrap_or(0.0);
    let hi = t.max.unwrap_or(100.0);
    if hi > lo { (lo, hi) } else { (lo, lo + 1.0) }
}

fn arc_points(c: Pos2, r: f32, a0: f32, a1: f32) -> Vec<Pos2> {
    let n = (((a1 - a0).abs() * r / 4.0) as usize).clamp(8, 160);
    (0..=n).map(|i| {
        let a = a0 + (a1 - a0) * i as f32 / n as f32;
        Pos2::new(c.x + r * a.cos(), c.y + r * a.sin())
    }).collect()
}

/// A thick arc with round ends.
fn arc(p: &Painter, c: Pos2, r: f32, a0: f32, a1: f32, w: f32, col: Color32) {
    p.add(Shape::line(arc_points(c, r, a0, a1), Stroke::new(w, col)));
    for a in [a0, a1] {
        p.circle_filled(Pos2::new(c.x + r * a.cos(), c.y + r * a.sin()), w / 2.0, col);
    }
}

fn gauge(p: &Painter, inner: Rect, t: &Tile, st: Option<&PointState>, pal: &Pal, accent: Color32) {
    use std::f32::consts::PI;
    let (lo, hi) = range_of(t);
    let side = inner.width().min(inner.height() * 1.08);
    let r = (side / 2.0 - 6.0).max(10.0);
    let c = Pos2::new(inner.center().x, inner.min.y + r + 6.0 + (inner.height() - side).max(0.0) * 0.3);
    let w = (r * 0.17).clamp(5.0, 26.0);
    let (a0, a1) = (PI * 0.75, PI * 2.25); // 270° sweep, gap at the bottom
    let frac = |v: f64| (((v - lo) / (hi - lo)).clamp(0.0, 1.0)) as f32;
    let at = |v: f64| a0 + (a1 - a0) * frac(v);
    arc(p, c, r - w / 2.0, a0, a1, w, pal.line.gamma_multiply(0.9));
    // Threshold zones as a thin outer band.
    let band = |from: f64, to: f64, col: Color32| {
        if to > from {
            p.add(Shape::line(arc_points(c, r + 3.0, at(from), at(to)), Stroke::new(3.0, col.gamma_multiply(0.85))));
        }
    };
    if let Some(v) = t.warn_hi { band(v, hi, pal.warn) }
    if let Some(v) = t.warn_lo { band(lo, v, pal.warn_lo) }
    if let Some(v) = t.alarm_hi { band(v, hi, pal.alarm) }
    if let Some(v) = t.alarm_lo { band(lo, v, pal.alarm_lo) }
    let Some(v) = st.and_then(|s| as_f64(&s.v)) else { return no_data(p, inner, pal) };
    let col = zone_color(t, v, accent, pal);
    if frac(v) > 0.0 {
        // Soft glow under the value arc, then the arc itself.
        p.add(Shape::line(arc_points(c, r - w / 2.0, a0, at(v)), Stroke::new(w * 1.9, col.gamma_multiply(0.12))));
        arc(p, c, r - w / 2.0, a0, at(v), w, col);
        let tip = Pos2::new(c.x + (r - w / 2.0) * at(v).cos(), c.y + (r - w / 2.0) * at(v).sin());
        p.circle_filled(tip, w * 0.32, pal.panel);
    }
    let st = st.unwrap();
    let value = fmt_value(&st.v, t.decimals);
    let units = if t.units.is_empty() { st.u.clone() } else { t.units.clone() };
    let vs = fit_font(p, &value, r * 1.25, r * 0.62, bold(), 11.0, 200.0);
    p.text(Pos2::new(c.x, c.y), Align2::CENTER_CENTER, value, FontId::new(vs, bold()), pal.text);
    if !units.is_empty() {
        p.text(Pos2::new(c.x, c.y + vs * 0.6), Align2::CENTER_TOP, units, FontId::proportional((vs * 0.34).clamp(10.0, 40.0)), pal.soft);
    }
    let small = FontId::proportional((r * 0.14).clamp(9.0, 14.0));
    p.text(Pos2::new(c.x - r * 0.62, c.y + r * 0.88), Align2::CENTER_CENTER, fmt_auto(lo), small.clone(), pal.muted);
    p.text(Pos2::new(c.x + r * 0.62, c.y + r * 0.88), Align2::CENTER_CENTER, fmt_auto(hi), small, pal.muted);
}

fn bar(p: &Painter, inner: Rect, t: &Tile, st: Option<&PointState>, pal: &Pal, accent: Color32) {
    let (lo, hi) = range_of(t);
    let Some(v) = st.and_then(|s| as_f64(&s.v)) else { return no_data(p, inner, pal) };
    let st = st.unwrap();
    let col = zone_color(t, v, accent, pal);
    let frac = (((v - lo) / (hi - lo)).clamp(0.0, 1.0)) as f32;
    let vertical = inner.height() > inner.width() * 1.2;
    let value = fmt_value(&st.v, t.decimals);
    let units = if t.units.is_empty() { st.u.clone() } else { t.units.clone() };
    let track_col = pal.line.gamma_multiply(0.8);
    if vertical {
        let text_h = (inner.height() * 0.16).clamp(16.0, 48.0);
        let label = if units.is_empty() { value } else { format!("{value} {units}") };
        let tw = (inner.width() * 0.5).clamp(10.0, 70.0);
        let track = Rect::from_min_max(Pos2::new(inner.center().x - tw / 2.0, inner.min.y + text_h + 8.0), Pos2::new(inner.center().x + tw / 2.0, inner.max.y));
        let r = CornerRadius::from((tw / 2.0).min(12.0));
        p.rect_filled(track, r, track_col);
        let fill = Rect::from_min_max(Pos2::new(track.min.x, track.max.y - track.height() * frac), track.max);
        if fill.height() > 1.0 {
            p.rect_filled(fill.expand(3.0), r, col.gamma_multiply(0.12));
            p.rect_filled(fill, r, col);
        }
        let s = fit_font(p, &label, inner.width(), text_h, bold(), 10.0, 60.0);
        p.text(Pos2::new(inner.center().x, inner.min.y + text_h / 2.0), Align2::CENTER_CENTER, label, FontId::new(s, bold()), pal.text);
    } else {
        let h = (inner.height() * 0.32).clamp(8.0, 30.0);
        let text_h = (inner.height() - h - 8.0).clamp(12.0, 90.0);
        let vs = fit_font(p, &value, inner.width() * 0.6, text_h, bold(), 11.0, 90.0);
        let vg = p.layout_no_wrap(value, FontId::new(vs, bold()), pal.text);
        let ty = inner.min.y + text_h / 2.0;
        let vsize = vg.size();
        p.galley(Pos2::new(inner.min.x, ty - vsize.y / 2.0), vg, pal.text);
        if !units.is_empty() {
            let ug = p.layout_no_wrap(units, FontId::proportional((vs * 0.42).clamp(10.0, 40.0)), pal.soft);
            p.galley(Pos2::new(inner.min.x + vsize.x + 6.0, ty + vsize.y / 2.0 - ug.size().y - vs * 0.1), ug, pal.soft);
        }
        let pct = format!("{:.0}%", frac * 100.0);
        p.text(Pos2::new(inner.max.x, ty), Align2::RIGHT_CENTER, pct, FontId::proportional((vs * 0.4).clamp(10.0, 28.0)), pal.muted);
        let track = Rect::from_min_max(Pos2::new(inner.min.x, inner.max.y - h), inner.max);
        let r = CornerRadius::from(h / 2.0);
        p.rect_filled(track, r, track_col);
        let fill = Rect::from_min_max(track.min, Pos2::new(track.min.x + track.width() * frac, track.max.y));
        if fill.width() > 1.0 {
            p.rect_filled(fill.expand(3.0), CornerRadius::from(h / 2.0 + 3.0), col.gamma_multiply(0.12));
            p.rect_filled(Rect::from_min_max(fill.min, Pos2::new(fill.max.x.max(fill.min.x + h), fill.max.y)), r, col);
        }
        // Threshold ticks under the track.
        for (v, c) in [(t.warn_lo, pal.warn_lo), (t.warn_hi, pal.warn), (t.alarm_lo, pal.alarm_lo), (t.alarm_hi, pal.alarm)] {
            if let Some(v) = v {
                let x = track.min.x + track.width() * (((v - lo) / (hi - lo)).clamp(0.0, 1.0)) as f32;
                p.line_segment([Pos2::new(x, track.min.y - 3.0), Pos2::new(x, track.max.y + 3.0)], Stroke::new(2.0, c));
            }
        }
    }
}

/// Truthiness of a point value for the indicator.
pub fn is_on(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::Number(n) => n.as_f64().map(|x| x != 0.0),
        Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "on" | "1" | "yes" | "running" | "open" | "active" => Some(true),
            "false" | "off" | "0" | "no" | "stopped" | "closed" | "inactive" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn indicator(p: &Painter, inner: Rect, t: &Tile, st: Option<&PointState>, pal: &Pal, accent: Color32) {
    let Some(on) = st.and_then(|s| is_on(&s.v)) else { return no_data(p, inner, pal) };
    let _ = accent;
    let on_col = pal.ok;
    let col = if on { on_col } else { pal.muted.gamma_multiply(0.5) };
    let label = if on { if t.on_text.is_empty() { "ON" } else { &t.on_text } } else if t.off_text.is_empty() { "OFF" } else { &t.off_text };
    let wide = inner.width() > inner.height() * 1.4;
    let r = (inner.height().min(inner.width()) * if wide { 0.3 } else { 0.24 }).max(6.0);
    let (lamp, text_rect) = if wide {
        let c = Pos2::new(inner.min.x + r * 1.5 + 2.0, inner.center().y);
        (c, Rect::from_min_max(Pos2::new(c.x + r * 1.6 + 8.0, inner.min.y), inner.max))
    } else {
        let c = Pos2::new(inner.center().x, inner.min.y + r * 1.5);
        (c, Rect::from_min_max(Pos2::new(inner.min.x, c.y + r * 1.5 + 2.0), inner.max))
    };
    if on {
        // Glow rings fading out.
        for (k, a) in [(1.6, 0.06), (1.35, 0.12), (1.15, 0.22)] {
            p.circle_filled(lamp, r * k, col.gamma_multiply(a));
        }
    } else {
        p.circle_stroke(lamp, r * 1.15, Stroke::new(1.0, pal.line));
    }
    p.circle_filled(lamp, r, col);
    // Specular highlight.
    p.circle_filled(lamp + Vec2::new(-r * 0.3, -r * 0.3), r * 0.28, Color32::from_white_alpha(if on { 70 } else { 25 }));
    let s = fit_font(p, label, text_rect.width(), text_rect.height() * 0.8, bold(), 10.0, 90.0);
    let anchor = if wide { (Align2::LEFT_CENTER, text_rect.left_center()) } else { (Align2::CENTER_CENTER, text_rect.center()) };
    p.text(anchor.1, anchor.0, label, FontId::new(s, bold()), if on { pal.text } else { pal.muted });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn values_format_sensibly() {
        assert_eq!(fmt_value(&json!(3400), None), "3400");
        assert_eq!(fmt_value(&json!(3.14159), None), "3.142");
        assert_eq!(fmt_value(&json!(2.5), None), "2.5");
        assert_eq!(fmt_value(&json!(2.0), None), "2");
        assert_eq!(fmt_value(&json!(2.0), Some(1)), "2.0");
        assert_eq!(fmt_value(&json!(true), None), "ON");
        assert_eq!(fmt_value(&json!("auto"), None), "auto");
        assert_eq!(fmt_value(&Value::Null, None), "—");
    }

    #[test]
    fn zones_and_colours() {
        let pal = Pal::of("", true);
        let mut t = Tile::new(1, Widget::Gauge);
        t.warn_hi = Some(80.0);
        t.alarm_hi = Some(95.0);
        t.warn_lo = Some(10.0);
        assert_eq!(zone_color(&t, 50.0, pal.accent, &pal), pal.accent);
        assert_eq!(zone_color(&t, 85.0, pal.accent, &pal), pal.warn);
        assert_eq!(zone_color(&t, 99.0, pal.accent, &pal), pal.alarm);
        assert_eq!(zone_color(&t, 5.0, pal.accent, &pal), pal.warn_lo, "too low has its own colour");
        assert_eq!(parse_color("#ff0080"), Some(Color32::from_rgb(255, 0, 128)));
        assert_eq!(parse_color("ff0080"), None);
        assert_eq!(parse_color("#fff"), None);
    }

    #[test]
    fn values_fall_into_zones() {
        let mut t = Tile::new(1, Widget::Numeric);
        assert_eq!(zone_of(&t, 1e9), Zone::Normal, "no limits, always normal");
        t.warn_hi = Some(80.0);
        t.alarm_hi = Some(95.0);
        t.alarm_lo = Some(5.0);
        assert_eq!(zone_of(&t, 50.0), Zone::Normal);
        assert_eq!(zone_of(&t, 80.0), Zone::WarnHi, "the limit itself counts");
        assert_eq!(zone_of(&t, 96.0), Zone::AlarmHi);
        assert_eq!(zone_of(&t, 2.0), Zone::AlarmLo);
        t.warn_lo = Some(10.0);
        assert_eq!(zone_of(&t, 8.0), Zone::WarnLo);
    }

    #[test]
    fn alarms_list_each_point_once_and_skip_bad_quality() {
        use std::collections::BTreeMap;
        let mut l = pb_dash_model::Layout::default();
        for (i, (point, x)) in [("psi", 0), ("psi", 4), ("lvl", 8), ("ok", 12)].into_iter().enumerate() {
            let mut t = Tile::new(i as u32 + 1, Widget::Numeric);
            t.point = point.into();
            t.x = x;
            t.alarm_hi = Some(100.0);
            t.alarm_lo = Some(10.0);
            l.tiles.push(t);
        }
        let mut pts = BTreeMap::new();
        let st = |v: f64, q: i32| PointState { v: json!(v), q, ts: 0, u: "psi".into() };
        pts.insert("psi".to_string(), st(150.0, 0));
        pts.insert("lvl".to_string(), st(5.0, 0));
        pts.insert("ok".to_string(), st(50.0, 0));
        let a = active_alarms(&l, &pts);
        assert_eq!(a.iter().map(|a| (a.point.as_str(), a.zone)).collect::<Vec<_>>(), vec![("psi", Zone::AlarmHi), ("lvl", Zone::AlarmLo)]);
        assert_eq!(a[0].detail, "150 psi — above the alarm limit 100");
        pts.insert("psi".to_string(), st(150.0, 1));
        assert_eq!(active_alarms(&l, &pts).len(), 1, "BAD quality isn't an alarm");
    }

    #[test]
    fn indicator_understands_common_on_off_words() {
        assert_eq!(is_on(&json!(true)), Some(true));
        assert_eq!(is_on(&json!(0)), Some(false));
        assert_eq!(is_on(&json!("Running")), Some(true));
        assert_eq!(is_on(&json!("stopped")), Some(false));
        assert_eq!(is_on(&json!("mystery")), None);
        assert_eq!(is_on(&Value::Null), None);
    }

    #[test]
    fn trend_lines_split_where_they_cross_a_threshold() {
        // Rising 0 → 100 through thresholds 50 and 80, then falling back to 60 through 80.
        let pieces = split_at_thresholds(&[(0.0, 0.0), (1.0, 100.0), (2.0, 60.0)], &[50.0, 80.0]);
        let ends: Vec<(f32, f64)> = pieces.iter().map(|p| p[1]).collect();
        assert_eq!(ends, vec![(0.5, 50.0), (0.8, 80.0), (1.0, 100.0), (1.5, 80.0), (2.0, 60.0)]);
        assert!(pieces.windows(2).all(|w| w[0][1] == w[1][0]), "pieces join up");
        assert_eq!(split_at_thresholds(&[(0.0, 1.0), (1.0, 2.0)], &[]).len(), 1);
    }

    #[test]
    fn point_names_read_nicely() {
        assert_eq!(pretty_name("tank_level"), "Tank level");
        assert_eq!(pretty_name("pump-2__run"), "Pump 2 run");
        assert_eq!(pretty_name("Discharge pressure"), "Discharge pressure");
        assert_eq!(pretty_name("TankLevel"), "TankLevel");
        assert_eq!(pretty_name("HR40001"), "HR40001");
        assert_eq!(pretty_name("PUMP_RUN"), "PUMP RUN");
    }

    #[test]
    fn range_defaults_and_guards() {
        let mut t = Tile::new(1, Widget::Gauge);
        assert_eq!(range_of(&t), (0.0, 100.0));
        t.min = Some(5.0);
        t.max = Some(5.0);
        let (lo, hi) = range_of(&t);
        assert!(hi > lo, "a flat range can't divide by zero");
    }
}
