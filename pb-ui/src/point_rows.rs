//! Shared Points table (headers + columns + live values).
//!
//! Used by the Points panel (all points) and Hub Input binds (this node).

use egui::{Align2, Color32, FontId, Id, Pos2, Rect, Rounding, Sense, Stroke, Vec2};
use pb_engine::PointSnapshot;

use crate::theme::{font_ui, tint, ThemePalette};

const ROW_H: f32 = 18.0;
const GAP: f32 = 5.0;
const DOT: f32 = 12.0;
const NAME: f32 = 78.0;
const VALUE: f32 = 78.0;
const UPDATED: f32 = 110.0;
const REMOVE: f32 = 18.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PointsSort {
    #[default]
    Name,
    Value,
    Source,
    Updated,
}

/// Sort state for a Points table instance (panel or Input binds).
#[derive(Debug, Clone)]
pub struct PointsTableState {
    pub sort: PointsSort,
    pub ascending: bool,
}

impl Default for PointsTableState {
    fn default() -> Self {
        Self {
            sort: PointsSort::Name,
            ascending: true,
        }
    }
}

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
    remove: f32,
}

impl PointsCols {
    pub fn from_width(avail: f32, with_remove: bool) -> Self {
        let remove = if with_remove { REMOVE } else { 0.0 };
        let row_w = avail.max(1.0);
        let source = (row_w - DOT - NAME - VALUE - UPDATED - remove - GAP * 4.0
            - if with_remove { GAP } else { 0.0 })
        .max(40.0);
        Self {
            row_w,
            gap: GAP,
            dot: DOT,
            name: NAME,
            value: VALUE,
            source,
            updated: UPDATED,
            remove,
        }
    }

    pub fn cell_rects(&self, row: Rect) -> (Rect, Rect, Rect, Rect, Rect, Option<Rect>) {
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
        x += self.updated;
        let remove = if self.remove > 0.0 {
            x += self.gap;
            Some(Rect::from_min_max(
                Pos2::new(x, y0),
                Pos2::new(x + self.remove, y1),
            ))
        } else {
            None
        };
        (dot, name, value, source, updated, remove)
    }
}

/// One painted row (live host point or pending Input bind).
#[derive(Clone, Debug)]
pub struct PointsTableRow {
    pub id: String,
    pub source: String,
    pub value: String,
    pub quality: String,
    pub updated: String,
    pub updated_ms: u64,
    /// When set, × removes / click edits this bind index on the Input node.
    pub bind_index: Option<usize>,
    /// Topic / JSON path for Manual edit fill (Input binds only).
    pub topic: String,
    pub path: String,
}

impl PointsTableRow {
    pub fn from_snapshot(point: &PointSnapshot) -> Self {
        Self {
            id: point.name.clone(),
            source: point.source.clone(),
            value: point.value.clone(),
            quality: point.quality.clone(),
            updated: format_updated(point.updated_ms),
            updated_ms: point.updated_ms,
            bind_index: None,
            topic: String::new(),
            path: String::new(),
        }
    }
}

/// One row's display fields for painters that borrow.
#[derive(Clone, Copy)]
pub struct PointRowRef<'a> {
    pub id: &'a str,
    pub source: &'a str,
    pub value: Option<&'a str>,
    pub quality: Option<&'a str>,
    pub updated: Option<&'a str>,
}

impl<'a> PointRowRef<'a> {
    pub fn from_table_row(row: &'a PointsTableRow) -> Self {
        Self {
            id: row.id.as_str(),
            source: row.source.as_str(),
            value: Some(row.value.as_str()),
            quality: Some(row.quality.as_str()),
            updated: Some(row.updated.as_str()),
        }
    }
}

#[derive(Clone, Copy, Default)]
pub struct PointRowAction {
    pub remove_clicked: bool,
    pub row_clicked: bool,
}

#[derive(Clone, Copy, Default)]
pub struct PointsTableAction {
    pub remove: Option<usize>,
    /// Bind index when a data row (not ×) is clicked — Input Manual edit.
    pub row_clicked: Option<usize>,
}

/// Sort + paint the shared Points table.
pub fn paint_points_table(
    ui: &mut egui::Ui,
    rows: &mut [PointsTableRow],
    state: &mut PointsTableState,
    p: &ThemePalette,
    id_salt: impl Into<Id>,
    with_remove: bool,
) -> PointsTableAction {
    let id_salt = id_salt.into();
    sort_table_rows(rows, state);

    let cols = PointsCols::from_width(ui.available_width(), with_remove);
    ui.set_min_width(cols.row_w);

    if let Some(col) = paint_points_header(ui, cols, state, p, id_salt) {
        if state.sort == col {
            state.ascending = !state.ascending;
        } else {
            state.sort = col;
            state.ascending = true;
        }
        sort_table_rows(rows, state);
    }
    ui.add_space(2.0);
    ui.separator();
    ui.add_space(1.0);

    let mut out = PointsTableAction::default();
    for row in rows.iter() {
        let action = paint_full_row(ui, cols, PointRowRef::from_table_row(row), p, row.bind_index);
        if action.remove_clicked {
            out.remove = row.bind_index;
        } else if action.row_clicked {
            out.row_clicked = row.bind_index;
        }
    }
    out
}

/// Load/store sort state in egui memory (Hub Input inspector).
pub fn table_state_for(ui: &egui::Ui, id_salt: impl Into<Id>) -> PointsTableState {
    let id = Id::new(("pb_pts_tbl", id_salt.into()));
    ui.ctx()
        .data(|d| d.get_temp::<PointsTableState>(id))
        .unwrap_or_default()
}

pub fn store_table_state(ui: &egui::Ui, id_salt: impl Into<Id>, state: PointsTableState) {
    let id = Id::new(("pb_pts_tbl", id_salt.into()));
    ui.ctx().data_mut(|d| d.insert_temp(id, state));
}

fn sort_table_rows(rows: &mut [PointsTableRow], state: &PointsTableState) {
    rows.sort_by(|a, b| {
        let ord = match state.sort {
            PointsSort::Name => a.id.cmp(&b.id),
            PointsSort::Value => a.value.cmp(&b.value),
            PointsSort::Source => a.source.cmp(&b.source),
            PointsSort::Updated => a.updated_ms.cmp(&b.updated_ms),
        };
        if state.ascending {
            ord
        } else {
            ord.reverse()
        }
    });
}

fn sort_mark(state: &PointsTableState, col: PointsSort) -> &'static str {
    if state.sort != col {
        return "";
    }
    if state.ascending {
        " ▲"
    } else {
        " ▼"
    }
}

fn paint_points_header(
    ui: &mut egui::Ui,
    cols: PointsCols,
    state: &PointsTableState,
    p: &ThemePalette,
    id_salt: Id,
) -> Option<PointsSort> {
    let (row, _) = ui.allocate_exact_size(Vec2::new(cols.row_w, ROW_H), Sense::hover());
    let (_dot, name_r, value_r, source_r, updated_r, _rm) = cols.cell_rects(row);
    let font = font_ui(10.5);
    let mut clicked = None;

    if header_label(
        ui,
        name_r,
        &format!("NAME{}", sort_mark(state, PointsSort::Name)),
        &font,
        p,
        false,
        id_salt.with("name"),
    )
    .clicked()
    {
        clicked = Some(PointsSort::Name);
    }
    if header_label(
        ui,
        value_r,
        &format!("VALUE{}", sort_mark(state, PointsSort::Value)),
        &font,
        p,
        false,
        id_salt.with("value"),
    )
    .clicked()
    {
        clicked = Some(PointsSort::Value);
    }
    if header_label(
        ui,
        source_r,
        &format!("SOURCE{}", sort_mark(state, PointsSort::Source)),
        &font,
        p,
        false,
        id_salt.with("source"),
    )
    .clicked()
    {
        clicked = Some(PointsSort::Source);
    }
    if header_label(
        ui,
        updated_r,
        &format!("UPDATED{}", sort_mark(state, PointsSort::Updated)),
        &font,
        p,
        true,
        id_salt.with("updated"),
    )
    .clicked()
    {
        clicked = Some(PointsSort::Updated);
    }
    clicked
}

fn header_label(
    ui: &mut egui::Ui,
    rect: Rect,
    text: &str,
    font: &FontId,
    p: &ThemePalette,
    right: bool,
    id: Id,
) -> egui::Response {
    let resp = ui.interact(rect, id, Sense::click());
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
    let color = if resp.hovered() { p.accent } else { p.muted };
    ui.painter().text(pos, align, text, font.clone(), color);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Full Points table data row (quality · id · value · source · updated [· ×]).
pub fn paint_full_row(
    ui: &mut egui::Ui,
    cols: PointsCols,
    row: PointRowRef<'_>,
    p: &ThemePalette,
    bind_index: Option<usize>,
) -> PointRowAction {
    let sense = if bind_index.is_some() {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(cols.row_w, ROW_H), sense);
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, Rounding::same(4.0), tint(p.accent, 12));
        if bind_index.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }
    let (dot_r, name_r, value_r, source_r, updated_r, remove_r) = cols.cell_rects(rect);

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
        )
        .on_hover_text(value);
    }
    let max_chars = ((cols.source / 7.0) as usize).max(6);
    let shown = truncate_keep_end(row.source, max_chars);
    paint_cell_text(ui, source_r, &shown, font.clone(), p.muted, false).on_hover_text(row.source);
    if let Some(updated) = row.updated {
        paint_cell_text(ui, updated_r, updated, font, p.muted, true);
    }

    let mut action = PointRowAction::default();
    if let (Some(r), Some(bi)) = (remove_r, bind_index) {
        let click = ui.interact(r, ui.id().with(("rm", bi, row.id)), Sense::click());
        if click.hovered() {
            ui.painter()
                .rect_filled(r, Rounding::same(4.0), tint(p.accent, 20));
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
    } else if bind_index.is_some() && resp.clicked() {
        action.row_clicked = true;
        resp.clone().on_hover_text("Edit topic in Manual");
    }
    if bind_index.is_some() && !action.remove_clicked {
        resp.on_hover_text("Click to edit topic in Manual");
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

/// Build table rows for one Input node's binds (live host points + pending placeholders).
pub fn rows_for_input_binds(
    conn: &str,
    binds: &[(String, String, String)],
    host_points: &[PointSnapshot],
) -> Vec<PointsTableRow> {
    // binds: (point, topic, path)
    let mut out = Vec::with_capacity(binds.len());
    for (i, (point, topic, path)) in binds.iter().enumerate() {
        let source = mqtt_bind_source(conn, topic, path);
        if let Some(pt) = find_live_point(point, &source, host_points) {
            let mut row = PointsTableRow::from_snapshot(pt);
            row.bind_index = Some(i);
            row.topic = topic.clone();
            row.path = path.clone();
            // Prefer short id when host already remapped.
            if !is_short_point_id(&row.id) && !point.trim().is_empty() {
                row.id = point.clone();
            } else if !is_short_point_id(&row.id) {
                row.id = "mqtt_N".into();
            }
            if !row.source.starts_with("mqtt/") {
                row.source = source;
            }
            out.push(row);
        } else {
            out.push(PointsTableRow {
                id: if point.trim().is_empty() {
                    "mqtt_N".into()
                } else {
                    point.clone()
                },
                source,
                value: "—".into(),
                quality: "uncertain".into(),
                updated: "—".into(),
                updated_ms: 0,
                bind_index: Some(i),
                topic: topic.clone(),
                path: path.clone(),
            });
        }
    }
    out
}

fn find_live_point<'a>(
    bind_point: &str,
    source: &str,
    host_points: &'a [PointSnapshot],
) -> Option<&'a PointSnapshot> {
    host_points.iter().find(|pt| {
        pt.source == source
            || (!bind_point.trim().is_empty() && pt.name == bind_point)
            || pt.name == source
    })
}

fn is_short_point_id(name: &str) -> bool {
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
        ui.id()
            .with(("cell", rect.min.x.to_bits(), rect.min.y.to_bits(), text)),
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

pub fn format_updated(updated_ms: u64) -> String {
    if updated_ms == 0 {
        return "—".into();
    }
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(updated_ms);
    let age_secs = now_ms.saturating_sub(updated_ms) / 1000;

    // Fresh samples read as live.
    if age_secs < 5 {
        return "just now".into();
    }

    const WEEK_SECS: u64 = 7 * 24 * 3600;
    if age_secs < WEEK_SECS {
        if age_secs < 60 {
            return format!(
                "{} second{} ago",
                age_secs,
                if age_secs == 1 { "" } else { "s" }
            );
        }
        let minutes = age_secs / 60;
        if minutes < 60 {
            return format!(
                "{} minute{} ago",
                minutes,
                if minutes == 1 { "" } else { "s" }
            );
        }
        // ≥1h and <1 week: hours only (including multi-day ages).
        let hours = age_secs / 3600;
        return format!(
            "{} hour{} ago",
            hours,
            if hours == 1 { "" } else { "s" }
        );
    }

    // Older than a week: local date + time to the minute.
    let secs = (updated_ms / 1000) as i64;
    match chrono::DateTime::from_timestamp(secs, 0) {
        Some(utc) => utc
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M")
            .to_string(),
        None => "—".into(),
    }
}
