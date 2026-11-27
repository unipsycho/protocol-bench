//! Time-series chart tile: several points on one chart, up to two Y axes, hover readout,
//! clickable legend. Drawn by hand (no plotting crate) so dual axes, step lanes and the tile's
//! own look need no compromises.

use egui::{Align2, Color32, FontId, Id, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2};
use pb_dash_model::{Series, Tile};

use crate::state::Shared;
use crate::widgets::{card, fmt_auto, parse_color, Pal, SERIES};

/// Colour of series `i`.
pub fn series_color(s: &Series, i: usize) -> Color32 {
    parse_color(&s.color).unwrap_or_else(|| crate::widgets::rgb(SERIES[i % SERIES.len()]))
}

/// "Nice" tick values covering `lo..=hi` (about `target` of them).
pub fn nice_ticks(lo: f64, hi: f64, target: usize) -> (Vec<f64>, usize) {
    let span = (hi - lo).max(1e-12);
    let raw = span / target.max(1) as f64;
    let mag = 10f64.powf(raw.log10().floor());
    let norm = raw / mag;
    let step = mag * if norm < 1.5 { 1.0 } else if norm < 3.5 { 2.0 } else if norm < 7.5 { 5.0 } else { 10.0 };
    let mut v = Vec::new();
    let mut x = (lo / step).ceil() * step;
    while x <= hi + step * 1e-9 && v.len() < 20 {
        v.push(x);
        x += step;
    }
    let decimals = (-step.log10().floor()).max(0.0) as usize;
    (v, decimals)
}

/// "-4m", "-90s", "now".
pub fn rel_label(secs_ago: f64) -> String {
    let s = secs_ago.round() as i64;
    if s <= 0 {
        "now".into()
    } else if s % 3600 == 0 {
        format!("-{}h", s / 3600)
    } else if s % 60 == 0 {
        format!("-{}m", s / 60)
    } else if s < 90 {
        format!("-{s}s")
    } else {
        format!("-{}m{}s", s / 60, s % 60)
    }
}

/// Value of a series at time `t` (the last sample at or before it: values hold until they change).
pub fn value_at(rows: &[(u64, f64)], t: u64) -> Option<f64> {
    let i = rows.partition_point(|r| r.0 <= t);
    if i == 0 { None } else { Some(rows[i - 1].1) }
}

struct Line<'a> {
    rows: &'a [(u64, f64)],
    color: Color32,
    axis: usize,
    style: &'a str,
    label: String,
}

pub fn paint(ui: &mut Ui, rect: Rect, t: &Tile, sh: &Shared, pal: &Pal, hover_ok: bool) {
    let painter = ui.painter_at(rect.expand(28.0));
    let inner = card(&painter, rect, t, pal, false);
    let window = if t.window_secs == 0 { 300 } else { t.window_secs } as u64 * 1000;
    let now = sh.server_now.max(1);
    let t0 = now.saturating_sub(window);
    let empty: Vec<(u64, f64)> = Vec::new();
    let lines: Vec<Line> = t
        .series
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.point.is_empty())
        .map(|(i, s)| Line {
            rows: sh.history.get(&s.point).unwrap_or(&empty),
            color: series_color(s, i),
            axis: (s.axis.min(1)) as usize,
            style: s.style.as_str(),
            label: if s.label.is_empty() { crate::widgets::pretty_name(&s.point) } else { s.label.clone() },
        })
        .collect();
    if lines.is_empty() {
        painter.text(inner.center(), Align2::CENTER_CENTER, "Add points in the tile editor", FontId::proportional(12.0), pal.muted);
        return;
    }
    // Legend (click to hide / show a series).
    let hid_id = Id::new(("pb_chart_hidden", t.id));
    let mut hidden: Vec<bool> = ui.data_mut(|d| d.get_temp(hid_id)).unwrap_or_default();
    hidden.resize(lines.len(), false);
    // Legend chips: colour swatch, name, latest value. They wrap onto more rows when the tile
    // is narrow (never more than a third of its height), so every series stays identifiable.
    let chip_h = 24.0;
    let (mut x, mut y) = (inner.min.x, inner.min.y);
    let max_y = inner.min.y + (inner.height() / 3.0).max(chip_h);
    for (i, l) in lines.iter().enumerate() {
        let cur = l.rows.last().map(|r| fmt_auto((r.1 * 100.0).round() / 100.0)).unwrap_or_else(|| "—".into());
        let off = hidden[i];
        let name = painter.layout_no_wrap(l.label.clone(), FontId::proportional(13.5), if off { pal.muted } else { pal.text });
        let val = painter.layout_no_wrap(cur, crate::theme::bold_id(13.5), if off { pal.muted } else { l.color });
        let w = 10.0 + 16.0 + 6.0 + name.size().x + 8.0 + val.size().x + 10.0;
        if x + w > inner.max.x && x > inner.min.x {
            x = inner.min.x;
            y += chip_h + 4.0;
        }
        if y + chip_h > max_y + 0.5 {
            break;
        }
        let item = Rect::from_min_size(Pos2::new(x, y), Vec2::new(w.min(inner.width()), chip_h));
        let resp = ui.interact(item, hid_id.with(i), Sense::click()).on_hover_text(if off { "Show this series" } else { "Hide this series" });
        painter.rect_filled(item, chip_h / 2.0, if resp.hovered() { pal.line } else { pal.panel2 });
        let sw = Pos2::new(item.min.x + 10.0, item.center().y);
        painter.line_segment([sw, sw + Vec2::new(16.0, 0.0)], Stroke::new(4.0, if off { pal.line } else { l.color }));
        painter.galley(Pos2::new(sw.x + 22.0, item.center().y - name.size().y / 2.0), name.clone(), pal.text);
        painter.galley(Pos2::new(sw.x + 22.0 + name.size().x + 8.0, item.center().y - val.size().y / 2.0), val, pal.text);
        if resp.clicked() {
            hidden[i] = !hidden[i];
        }
        x = item.max.x + 6.0;
    }
    let legend_h = y + chip_h - inner.min.y;
    ui.data_mut(|d| d.insert_temp(hid_id, hidden.clone()));

    let right_axis = lines.iter().any(|l| l.axis == 1);
    let plot = Rect::from_min_max(
        Pos2::new(inner.min.x + 42.0, inner.min.y + legend_h + 10.0),
        Pos2::new(inner.max.x - if right_axis { 42.0 } else { 6.0 }, inner.max.y - 16.0),
    );
    if plot.width() < 30.0 || plot.height() < 20.0 {
        return;
    }
    // Y ranges per axis from the visible samples.
    let mut yr = [(f64::MAX, f64::MIN); 2];
    for (i, l) in lines.iter().enumerate().filter(|(i, _)| !hidden[*i]) {
        let _ = i;
        for &(ts, v) in l.rows.iter().filter(|r| r.0 >= t0) {
            let _ = ts;
            yr[l.axis].0 = yr[l.axis].0.min(v);
            yr[l.axis].1 = yr[l.axis].1.max(v);
        }
        // The value held from before the window start is on screen too.
        if let Some(v) = value_at(l.rows, t0) {
            yr[l.axis].0 = yr[l.axis].0.min(v);
            yr[l.axis].1 = yr[l.axis].1.max(v);
        }
    }
    for (a, r) in yr.iter_mut().enumerate() {
        if r.0 > r.1 {
            *r = (0.0, 1.0);
        }
        if a == 0 {
            if let (Some(lo), Some(hi)) = (t.min, t.max) {
                if hi > lo {
                    *r = (lo, hi);
                    continue;
                }
            }
        }
        if (r.1 - r.0).abs() < 1e-9 {
            r.0 -= 1.0;
            r.1 += 1.0;
        } else {
            let pad = (r.1 - r.0) * 0.08;
            r.0 -= pad;
            r.1 += pad;
        }
    }
    let to_x = |ts: u64| plot.min.x + plot.width() * ((ts.saturating_sub(t0)) as f32 / window as f32).clamp(0.0, 1.0);
    let to_y = |a: usize, v: f64| plot.max.y - plot.height() * (((v - yr[a].0) / (yr[a].1 - yr[a].0)) as f32).clamp(-0.05, 1.05);
    // Grid + axis labels.
    for a in 0..=(right_axis as usize) {
        let (ticks, dec) = nice_ticks(yr[a].0, yr[a].1, 4);
        for v in ticks {
            let y = to_y(a, v);
            if y < plot.min.y - 1.0 || y > plot.max.y + 1.0 {
                continue;
            }
            if a == 0 {
                painter.line_segment([Pos2::new(plot.min.x, y), Pos2::new(plot.max.x, y)], Stroke::new(1.0, pal.line.gamma_multiply(0.55)));
                painter.text(Pos2::new(plot.min.x - 4.0, y), Align2::RIGHT_CENTER, format!("{v:.dec$}"), FontId::proportional(10.0), pal.muted);
            } else {
                painter.text(Pos2::new(plot.max.x + 4.0, y), Align2::LEFT_CENTER, format!("{v:.dec$}"), FontId::proportional(10.0), pal.muted);
            }
        }
    }
    // Time axis: as many labels as fit (at least the two ends), evenly spaced.
    let slots = ((plot.width() / 78.0) as usize).clamp(1, 4);
    for k in 0..=slots {
        let f = k as f32 / slots as f32;
        let x = plot.min.x + plot.width() * f;
        painter.line_segment([Pos2::new(x, plot.min.y), Pos2::new(x, plot.max.y)], Stroke::new(1.0, pal.line.gamma_multiply(0.3)));
        let secs_ago = (window as f64 / 1000.0) * (1.0 - f as f64);
        let anchor = if k == 0 { Align2::LEFT_TOP } else if k == slots { Align2::RIGHT_TOP } else { Align2::CENTER_TOP };
        painter.text(Pos2::new(x, plot.max.y + 2.0), anchor, rel_label(secs_ago), FontId::proportional(10.0), pal.muted);
    }
    // Lines.
    let clip = painter.with_clip_rect(plot.expand(1.0));
    let mut any = false;
    for (i, l) in lines.iter().enumerate().filter(|(i, _)| !hidden[*i]) {
        let _ = i;
        let mut pts: Vec<Pos2> = Vec::new();
        let hold = value_at(l.rows, t0);
        if let Some(v) = hold {
            pts.push(Pos2::new(plot.min.x, to_y(l.axis, v)));
        }
        let mut last_y = pts.last().map(|p| p.y);
        for &(ts, v) in l.rows.iter().filter(|r| r.0 >= t0) {
            let (x, y) = (to_x(ts), to_y(l.axis, v));
            if l.style == "step" {
                if let Some(ly) = last_y {
                    pts.push(Pos2::new(x, ly));
                }
            }
            pts.push(Pos2::new(x, y));
            last_y = Some(y);
        }
        // Values hold until they change: extend to "now".
        if let Some(y) = last_y {
            pts.push(Pos2::new(plot.max.x, y));
        }
        if pts.len() < 2 {
            continue;
        }
        any = true;
        if l.style == "area" {
            clip.add(crate::widgets::area_mesh(&pts, plot.max.y, l.color.gamma_multiply(0.3)));
        }
        clip.add(Shape::line(pts.clone(), Stroke::new(5.0, l.color.gamma_multiply(0.10))));
        clip.add(Shape::line(pts, Stroke::new(2.0, l.color)));
    }
    if !any {
        painter.text(plot.center(), Align2::CENTER_CENTER, "No data yet", FontId::proportional(12.0), pal.muted);
    }
    // Hover readout.
    if hover_ok {
        if let Some(pos) = ui.input(|i| i.pointer.hover_pos()).filter(|p| plot.contains(*p)) {
            let ts = t0 + (window as f64 * ((pos.x - plot.min.x) / plot.width()) as f64) as u64;
            painter.line_segment([Pos2::new(pos.x, plot.min.y), Pos2::new(pos.x, plot.max.y)], Stroke::new(1.0, pal.muted));
            let rows: Vec<(String, Color32, Option<f64>)> = lines.iter().enumerate().filter(|(i, _)| !hidden[*i]).map(|(_, l)| (l.label.clone(), l.color, value_at(l.rows, ts))).collect();
            let text_h = 14.0;
            let w = rows.iter().map(|(n, _, v)| painter.layout_no_wrap(format!("{n}  {}", v.map(fmt_auto).unwrap_or_else(|| "—".into())), FontId::proportional(11.0), pal.text).size().x).fold(60.0f32, f32::max) + 22.0;
            let h = rows.len() as f32 * text_h + 14.0;
            let mut o = Pos2::new(pos.x + 10.0, plot.min.y + 4.0);
            if o.x + w > plot.max.x {
                o.x = pos.x - 10.0 - w;
            }
            let b = Rect::from_min_size(o, Vec2::new(w, h));
            painter.rect_filled(b, 8.0, pal.panel2);
            painter.rect_stroke(b, 8.0, Stroke::new(1.0, pal.line), egui::StrokeKind::Inside);
            painter.text(Pos2::new(b.min.x + 6.0, b.min.y + 3.0), Align2::LEFT_TOP, rel_label((now.saturating_sub(ts)) as f64 / 1000.0), FontId::proportional(10.0), pal.muted);
            for (k, (n, c, v)) in rows.iter().enumerate() {
                let y = b.min.y + 14.0 + k as f32 * text_h + text_h / 2.0;
                painter.circle_filled(Pos2::new(b.min.x + 9.0, y), 3.5, *c);
                painter.text(Pos2::new(b.min.x + 17.0, y), Align2::LEFT_CENTER, format!("{n}  {}", v.map(fmt_auto).unwrap_or_else(|| "—".into())), FontId::proportional(11.0), pal.text);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_are_nice() {
        let (t, d) = nice_ticks(0.0, 100.0, 4);
        assert_eq!(t, vec![0.0, 20.0, 40.0, 60.0, 80.0, 100.0]);
        assert_eq!(d, 0);
        let (t, d) = nice_ticks(2.9, 3.4, 4);
        assert_eq!(d, 1);
        assert!(t.first().unwrap() >= &2.9 && t.last().unwrap() <= &3.4001, "{t:?}");
        assert!(t.windows(2).all(|w| ((w[1] - w[0]) - (t[1] - t[0])).abs() < 1e-9));
        let (t, _) = nice_ticks(-5.0, 5.0, 4);
        assert!(t.contains(&0.0));
        assert!(nice_ticks(1.0, 1.0, 4).0.len() <= 20, "flat range can't loop forever");
    }

    #[test]
    fn relative_time_labels() {
        assert_eq!(rel_label(0.0), "now");
        assert_eq!(rel_label(45.0), "-45s");
        assert_eq!(rel_label(300.0), "-5m");
        assert_eq!(rel_label(3600.0), "-1h");
        assert_eq!(rel_label(150.0), "-2m30s");
    }

    #[test]
    fn values_hold_until_they_change() {
        let rows = vec![(1000, 1.0), (2000, 5.0), (4000, 2.0)];
        assert_eq!(value_at(&rows, 500), None);
        assert_eq!(value_at(&rows, 1000), Some(1.0));
        assert_eq!(value_at(&rows, 3999), Some(5.0));
        assert_eq!(value_at(&rows, 99_999), Some(2.0));
        assert_eq!(value_at(&[], 5), None);
    }
}
