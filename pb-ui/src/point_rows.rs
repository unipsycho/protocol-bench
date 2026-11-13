//! Shared full-width point list rows.
//!
//! Used by the Points panel (all points) and Hub Input binds (this node).

use egui::{Align2, Color32, FontId, Pos2, Rect, Rounding, Sense, Stroke, Vec2};
use pb_engine::PointSnapshot;

use crate::theme::{font_ui, tint, ThemePalette};

const ROW_H: f32 = 18.0;
const GAP: f32 = 5.0;
const DOT: f32 = 12.0;
const NAME: f32 = 78.0;
const VALUE: f32 = 78.0;
const UPDATED: f32 = 110.0;
const REMOVE: f32 = 18.0;

/// Column geometry for the full Points table.
#[derive(Clone, Copy)]
pub struct PointsCols {
    pub row_w: f32,
    gap: f32,
    dot: f32,
    name: f32,
    value: f32,
    source: f32,
    updated: f32,
}

impl PointsCols {
    pub fn from_width(avail: f32) -> Self {
        let row_w = avail.max(1.0);
        let source = (row_w - DOT - NAME - VALUE - UPDATED - GAP * 4.0).max(48.0);
        Self {
            row_w,
            gap: GAP,
            dot: DOT,
            name: NAME,
            value: VALUE,
            source,
            updated: UPDATED,
        }
    }

    pub fn cell_rects(&self, row: Rect) -> (Rect, Rect, Rect, Rect, Rect) {
        let y0 = row.min.y;
        let y1 = row.max.y;
        let mut x = row.min.x;
        let dot = Rect::from_min_max(Pos2::new(x, y0), Pos2::new(x + self.dot, y1));
        x += self.dot + self.gap;
        let name = Rect::from_min_max(Pos2::new(x, y0), Pos2::new(x + self.name, y1));
        x += self.name + self.gap;
        let value = Rect::from_min_max(Pos2::new(x, y0), Pos2::new(x + self.value, y1));
        x += self.value + self.gap;
        let source = Rect::from_min_max(Pos2::new(x, y0), Pos2::new(x + self.source, y1));
        x += self.source + self.gap;
        let updated = Rect::from_min_max(Pos2::new(x, y0), Pos2::new(x + self.updated, y1));
        (dot, name, value, source, updated)
    }
}

/// Compact id | source row (Hub Input binds, optional Points subset).
#[derive(Clone, Copy)]
pub struct IdSourceCols {
    pub row_w: f32,
    id: f32,
    source: f32,
    remove: f32,
}

impl IdSourceCols {
    pub fn from_width(avail: f32, with_remove: bool) -> Self {
        let row_w = avail.max(1.0);
        let remove = if with_remove { REMOVE } else { 0.0 };
        let id = NAME.max(64.0);
        let source = (row_w - id - GAP - remove).max(48.0);
        Self {
            row_w,
            id,
            source,
            remove,
        }
    }
}

/// One row's display fields (Points panel or Input bind).
#[derive(Clone, Copy)]
pub struct PointRowRef<'a> {
    pub id: &'a str,
    pub source: &'a str,
    pub value: Option<&'a str>,
    pub quality: Option<&'a str>,
    pub updated: Option<&'a str>,
}

impl<'a> PointRowRef<'a> {
    pub fn from_snapshot(point: &'a PointSnapshot, updated: &'a str) -> Self {
        Self {
            id: point.name.as_str(),
            source: point.source.as_str(),
            value: Some(point.value.as_str()),
            quality: Some(point.quality.as_str()),
            updated: Some(updated),
        }
    }
}

#[derive(Clone, Copy, Default)]
pub struct PointRowAction {
    pub remove_clicked: bool,
}

/// Full Points table data row (quality · id · value · source · updated).
pub fn paint_full_row(
    ui: &mut egui::Ui,
    cols: PointsCols,
    row: PointRowRef<'_>,
    p: &ThemePalette,
) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(cols.row_w, ROW_H), Sense::hover());
    let (dot_r, name_r, value_r, source_r, updated_r) = cols.cell_rects(rect);

    if let Some(quality) = row.quality {
        paint_quality_dot(ui, dot_r, quality, row.id, p);
    }

    let font = font_ui(12.0);
    paint_cell_text(ui, name_r, row.id, font.clone(), p.text, false);
    if let Some(value) = row.value {
        paint_cell_text(
            ui,
            value_r,
            &truncate_chars(value, 14),
            font.clone(),
            p.text,
            false,
        );
    }
    let max_chars = ((cols.source / 7.0) as usize).max(6);
    let shown = truncate_keep_end(row.source, max_chars);
    paint_cell_text(ui, source_r, &shown, font.clone(), p.muted, false)
        .on_hover_text(row.source);
    if let Some(updated) = row.updated {
        paint_cell_text(ui, updated_r, updated, font, p.muted, true);
    }
}

/// Full-width id (left) + source path (right); optional trailing × for binds.
pub fn paint_id_source_row(
    ui: &mut egui::Ui,
    cols: IdSourceCols,
    row: PointRowRef<'_>,
    p: &ThemePalette,
) -> PointRowAction {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(cols.row_w, ROW_H), Sense::hover());
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, Rounding::same(4.0), tint(p.accent, 12));
    }

    let y0 = rect.min.y;
    let y1 = rect.max.y;
    let mut x = rect.min.x;
    let id_r = Rect::from_min_max(Pos2::new(x, y0), Pos2::new(x + cols.id, y1));
    x += cols.id + GAP;
    let source_r = Rect::from_min_max(Pos2::new(x, y0), Pos2::new(x + cols.source, y1));
    x += cols.source;
    let remove_r = if cols.remove > 0.0 {
        Some(Rect::from_min_max(
            Pos2::new(x, y0),
            Pos2::new(x + cols.remove, y1),
        ))
    } else {
        None
    };

    let font = font_ui(12.0);
    paint_cell_text(ui, id_r, row.id, font.clone(), p.text, false);
    let max_chars = ((cols.source / 7.0) as usize).max(6);
    let shown = truncate_keep_end(row.source, max_chars);
    paint_cell_text(ui, source_r, &shown, font, p.muted, false).on_hover_text(row.source);

    let mut action = PointRowAction::default();
    if let Some(r) = remove_r {
        let click = ui.interact(r, ui.id().with(("rm", row.id, row.source)), Sense::click());
        let fill = if click.hovered() {
            tint(p.accent, 20)
        } else {
            Color32::TRANSPARENT
        };
        if fill.a() > 0 {
            ui.painter().rect_filled(r, Rounding::same(4.0), fill);
        }
        ui.painter().text(
            r.center(),
            Align2::CENTER_CENTER,
            "×",
            font_ui(13.0),
            if click.hovered() { p.accent } else { p.muted },
        );
        if click.clicked() {
            action.remove_clicked = true;
        }
        click.on_hover_cursor(egui::CursorIcon::PointingHand);
    }
    action
}

/// MQTT wire address for a Hub Input bind: `mqtt/<conn>/<topic>[::<path>]`.
pub fn mqtt_bind_source(conn: &str, topic: &str, path: &str) -> String {
    let conn = conn.trim();
    let topic = topic.trim();
    let path = path.trim();
    let base = if conn.is_empty() {
        format!("mqtt/{topic}")
    } else {
        format!("mqtt/{conn}/{topic}")
    };
    if path.is_empty() {
        base
    } else {
        format!("{base}::{path}")
    }
}

/// Resolve display id + source for one Input bind against live host points.
pub fn resolve_input_bind_row<'a>(
    bind_point: &'a str,
    source: &'a str,
    host_points: &'a [PointSnapshot],
) -> (String, &'a str) {
    let live = host_points.iter().find(|pt| {
        pt.source == source
            || (!bind_point.trim().is_empty() && pt.name == bind_point)
            || pt.name == source
    });
    let id = if let Some(pt) = live {
        if is_short_point_id(&pt.name) {
            pt.name.clone()
        } else if !bind_point.trim().is_empty() {
            bind_point.to_string()
        } else {
            "mqtt_N".into()
        }
    } else if !bind_point.trim().is_empty() {
        bind_point.to_string()
    } else {
        "mqtt_N".into()
    };
    let shown_source = live
        .map(|pt| {
            if pt.source.starts_with("mqtt/") {
                pt.source.as_str()
            } else {
                source
            }
        })
        .unwrap_or(source);
    (id, shown_source)
}

fn is_short_point_id(name: &str) -> bool {
    // mqtt_1 / opc_12 — proto + '_' + digits
    let Some((proto, rest)) = name.split_once('_') else {
        return false;
    };
    let plen = proto.len();
    (3..=4).contains(&plen) && !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
}

pub fn paint_quality_dot(
    ui: &mut egui::Ui,
    rect: Rect,
    quality: &str,
    id_salt: &str,
    p: &ThemePalette,
) {
    let color = match quality {
        "good" => Color32::from_rgb(0x3d, 0xb8, 0x6b),
        "bad" => Color32::from_rgb(0xe0, 0x55, 0x55),
        _ => Color32::from_rgb(0xd4, 0xa0, 0x3c),
    };
    let r = 3.0;
    ui.painter().circle_filled(rect.center(), r, color);
    ui.painter()
        .circle_stroke(rect.center(), r, Stroke::new(1.0, tint(p.text, 40)));
    ui.interact(rect, ui.id().with(("q", id_salt)), Sense::hover())
        .on_hover_text(quality);
}

pub fn paint_cell_text(
    ui: &mut egui::Ui,
    rect: Rect,
    text: &str,
    font: FontId,
    color: Color32,
    right: bool,
) -> egui::Response {
    let resp = ui.interact(
        rect,
        ui.id().with(("cell", rect.min.x.to_bits(), rect.min.y.to_bits(), text)),
        Sense::hover(),
    );
    let painter = ui.painter().with_clip_rect(rect);
    let align = if right {
        Align2::RIGHT_CENTER
    } else {
        Align2::LEFT_CENTER
    };
    let pos = if right {
        Pos2::new(rect.max.x, rect.center().y)
    } else {
        Pos2::new(rect.min.x, rect.center().y)
    };
    painter.text(pos, align, text, font, color);
    resp
}

/// Ellipsis at the start so the end of a long path stays visible.
pub fn truncate_keep_end(value: &str, max_chars: usize) -> String {
    let flat = value.replace('\n', " ");
    let count = flat.chars().count();
    if count <= max_chars {
        return flat;
    }
    let keep = max_chars.saturating_sub(1);
    let end: String = flat.chars().skip(count.saturating_sub(keep)).collect();
    format!("…{end}")
}

pub fn truncate_chars(value: &str, max_chars: usize) -> String {
    let flat = value.replace('\n', " ");
    if flat.chars().count() <= max_chars {
        flat
    } else {
        let mut out: String = flat.chars().take(max_chars.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}
