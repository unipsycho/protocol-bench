//! Shared Points table (headers + columns + live values).
//!
//! Used by the Points panel (all points) and Hub Input binds (this node).

use egui::{Align2, Color32, FontId, Id, Pos2, Rect, Rounding, Sense, Stroke, Vec2};
use pb_engine::PointSnapshot;

use crate::hubs::{HubModel, HubProto};
use crate::theme::{font_ui, tint, ThemePalette};

/// Drag payload: Points bus → Streams emit map, or reorder within that map.
#[derive(Clone, Debug)]
pub enum PointEmitDrag {
    /// Live / bound point id from the Points panel.
    FromPoints(String),
    /// Index into a Streams Output `stream_outputs` list (same-port reorder).
    ReorderEmit(usize),
}

/// Drag payload: Streams Input preview field chip → bind list (`stream_N`).
#[derive(Clone, Debug)]
pub struct StreamScanFieldDrag {
    /// Field index string (`"0"`, `"1"`, …) or empty for whole-line.
    pub field: String,
}

/// Skip placeholder template ids (`mqtt_N`, `stream_N`, …).
pub fn is_draggable_point_id(id: &str) -> bool {
    let id = id.trim();
    !id.is_empty() && !id.ends_with("_N")
}

const ROW_H: f32 = 18.0;
const GAP: f32 = 5.0;
const DOT: f32 = 12.0;
const REMOVE: f32 = 18.0;
/// Breathing room added to each measured column.
const CELL_PAD: f32 = 8.0;
/// Source is the flex column: it absorbs slack and is cropped first.
const SOURCE_MIN: f32 = 48.0;
/// Name only gives ground once Source is at its floor.
const NAME_MIN: f32 = 72.0;
const CELL_FONT: f32 = 12.0;
const HEADER_FONT: f32 = 10.5;

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
    /// Auto-fit columns to their content. Value and Updated always get their
    /// full measured width; Source absorbs slack and is cropped first, then
    /// Name down to [`NAME_MIN`]. If even that does not fit, the row grows
    /// past `avail` (the caller's scroll area handles overflow).
    pub fn fit(ui: &egui::Ui, avail: f32, rows: &[PointsTableRow], with_remove: bool) -> Self {
        let cell = font_ui(CELL_FONT);
        let head = font_ui(HEADER_FONT);
        let measure = |text: &str, font: &FontId| text_width(ui, text, font);
        // Headers measured with a sort mark so widths don't jump on sort.
        let mut name = measure("NAME ▼", &head);
        let mut value = measure("VALUE ▼", &head);
        let mut updated = measure("UPDATED ▼", &head);
        for row in rows {
            name = name.max(measure(&row.id, &cell));
            value = value.max(measure(&flat(&row.value), &cell));
            updated = updated.max(measure(&row.updated, &cell));
        }
        name += CELL_PAD;
        value += CELL_PAD;
        updated += CELL_PAD;

        let remove = if with_remove { REMOVE } else { 0.0 };
        let gaps = GAP * if with_remove { 5.0 } else { 4.0 };
        let avail = avail.max(1.0);
        let mut source = avail - DOT - name - value - updated - remove - gaps;
        if source < SOURCE_MIN {
            let deficit = SOURCE_MIN - source;
            source = SOURCE_MIN;
            let give = deficit.min((name - NAME_MIN).max(0.0));
            name -= give;
        }
        let row_w = DOT + name + value + source + updated + remove + gaps;
        Self {
            row_w: row_w.max(avail),
            gap: GAP,
            dot: DOT,
            name,
            value,
            source: source + (avail - row_w).max(0.0),
            updated,
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
        let value = if point.value.trim().is_empty() {
            "—".into()
        } else {
            point.value.clone()
        };
        Self {
            id: point.name.clone(),
            source: point.source.clone(),
            value,
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
///
/// When `drag_from_bus` is true, each real point row is a drag source
/// ([`PointEmitDrag::FromPoints`]) for Streams Output emit maps.
pub fn paint_points_table(
    ui: &mut egui::Ui,
    rows: &mut [PointsTableRow],
    state: &mut PointsTableState,
    p: &ThemePalette,
    id_salt: impl Into<Id>,
    with_remove: bool,
) -> PointsTableAction {
    paint_points_table_ex(ui, rows, state, p, id_salt, with_remove, false)
}

/// Like [`paint_points_table`], with optional bus → emit drag handles.
pub fn paint_points_table_ex(
    ui: &mut egui::Ui,
    rows: &mut [PointsTableRow],
    state: &mut PointsTableState,
    p: &ThemePalette,
    id_salt: impl Into<Id>,
    with_remove: bool,
    drag_from_bus: bool,
) -> PointsTableAction {
    let id_salt = id_salt.into();
    sort_table_rows(rows, state);

    let cols = PointsCols::fit(ui, ui.available_width(), rows, with_remove);
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
        let can_drag = drag_from_bus && is_draggable_point_id(&row.id);
        let action = if can_drag {
            let point_id = row.id.clone();
            let drag_id = ui.id().with(("pb_pt_drag", point_id.as_str()));
            let inner = ui.dnd_drag_source(
                drag_id,
                PointEmitDrag::FromPoints(point_id),
                |ui| {
                    paint_full_row(
                        ui,
                        cols,
                        PointRowRef::from_table_row(row),
                        p,
                        row.bind_index,
                        true,
                    )
                },
            );
            if !ui.ctx().is_being_dragged(drag_id) {
                inner
                    .response
                    .on_hover_cursor(egui::CursorIcon::Grab)
                    .on_hover_text("Drag onto a Streams Output emit list");
            }
            inner.inner
        } else {
            paint_full_row(
                ui,
                cols,
                PointRowRef::from_table_row(row),
                p,
                row.bind_index,
                false,
            )
        };
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
    let font = font_ui(HEADER_FONT);
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
///
/// When `select_on_hover` is true (Points bus drag sources), wash the whole row
/// under the pointer so the grab target is obvious.
pub fn paint_full_row(
    ui: &mut egui::Ui,
    cols: PointsCols,
    row: PointRowRef<'_>,
    p: &ThemePalette,
    bind_index: Option<usize>,
    select_on_hover: bool,
) -> PointRowAction {
    // Reserve space first; bind-row click is registered last so it wins cell hovers.
    let (rect, _) = ui.allocate_exact_size(Vec2::new(cols.row_w, ROW_H), Sense::hover());
    let (dot_r, name_r, value_r, source_r, updated_r, remove_r) = cols.cell_rects(rect);
    let click_rect = match (bind_index, remove_r) {
        (Some(_), Some(rm)) => Rect::from_min_max(rect.min, Pos2::new(rm.min.x, rect.max.y)),
        (Some(_), None) => rect,
        _ => Rect::NOTHING,
    };
    // Wash under text when the bind row is hot, or when this row is a drag source.
    let hot = if select_on_hover {
        ui.rect_contains_pointer(rect)
    } else {
        bind_index.is_some() && ui.rect_contains_pointer(click_rect)
    };
    if hot {
        let wash = if select_on_hover { rect } else { click_rect };
        ui.painter()
            .rect_filled(wash, Rounding::same(4.0), tint(p.accent, 18));
    }

    if let Some(quality) = row.quality {
        paint_quality_dot(ui, dot_r, quality, row.id, p);
    }

    let font = font_ui(CELL_FONT);
    let fit_w = |r: Rect| (r.width() - CELL_PAD * 0.5).max(1.0);
    let name_shown = fit_text_end(ui, row.id, &font, fit_w(name_r));
    let name_resp = paint_cell_text(ui, name_r, &name_shown, font.clone(), p.text, false);
    if name_shown != row.id {
        name_resp.on_hover_text(row.id);
    }
    if let Some(value) = row.value {
        // Columns are sized so the value is never cropped.
        paint_cell_text(ui, value_r, &flat(value), font.clone(), p.text, false)
            .on_hover_text(value);
    }
    let shown = fit_text_keep_end(ui, row.source, &font, fit_w(source_r));
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
    }
    // Bind rows: click anywhere except × → Manual edit.
    if let Some(bi) = bind_index {
        let resp = ui.interact(
            click_rect,
            ui.id().with(("bind_row", bi, row.id, row.source)),
            Sense::click(),
        );
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() && !action.remove_clicked {
            action.row_clicked = true;
        }
        resp.on_hover_text("Click to edit topic in Manual");
    }
    action
}

// Wire-address formats live in `pb_points::addr` (shared with the service).
pub use pb_points::addr::{
    file_bind_source, http_bind_source, las_bind_source, mqtt_bind_source, snmp_bind_source,
    stream_bind_source,
};

/// Wire address for a Modbus register bind: `modbus/<conn>[/u<unit>]::<area>:<addr>:<type>[:<order>]`.
pub fn modbus_bind_source(conn: &str, bind: &crate::hubs::ModbusBind) -> String {
    pb_points::addr::modbus_bind_source(conn, bind.unit as u64, &bind.area, bind.address as u64, &bind.dtype, &bind.order)
}

/// Global Points panel rows: live bus + Hub Input bind / aggregator placeholders.
///
/// Honest empty only when there are no host points and no Input/agg binds.
pub fn rows_for_points_panel(
    hubs: &HubModel,
    host_points: &[PointSnapshot],
) -> Vec<PointsTableRow> {
    let mut rows: Vec<PointsTableRow> = host_points
        .iter()
        .map(PointsTableRow::from_snapshot)
        .collect();
    let mut seen: std::collections::BTreeSet<String> =
        rows.iter().map(|r| r.id.clone()).collect();

    for hub in &hubs.hubs {
        for port in hub.sources.iter().chain(hub.outputs.iter()) {
            match port.protocol {
                HubProto::Mqtt => {
                    let conn = port.mqtt_connection.as_str();
                    for bind in &port.mqtt_inputs {
                        let topic = bind.topic.trim();
                        if topic.is_empty() {
                            continue;
                        }
                        let source = mqtt_bind_source(conn, topic, bind.path.trim());
                        let id = if bind.point.trim().is_empty() {
                            "mqtt_N".to_string()
                        } else {
                            bind.point.trim().to_string()
                        };
                        if seen.contains(&id) {
                            continue;
                        }
                        seen.insert(id.clone());
                        rows.push(PointsTableRow {
                            id,
                            source,
                            value: "—".into(),
                            quality: "uncertain".into(),
                            updated: "—".into(),
                            updated_ms: 0,
                            bind_index: None,
                            topic: bind.topic.clone(),
                            path: bind.path.clone(),
                        });
                    }
                }
                HubProto::OpcUa => {
                    let conn = port.opcua_connection.trim();
                    for bind in &port.opcua_inputs {
                        let node = bind.node_id.trim();
                        if node.is_empty() {
                            continue;
                        }
                        let source = if conn.is_empty() {
                            format!("opcua/{node}")
                        } else {
                            format!("opcua/{conn}/{node}")
                        };
                        let id = if bind.point.trim().is_empty() {
                            "opc_N".to_string()
                        } else {
                            bind.point.trim().to_string()
                        };
                        if seen.contains(&id) {
                            continue;
                        }
                        seen.insert(id.clone());
                        rows.push(PointsTableRow {
                            id,
                            source,
                            value: "—".into(),
                            quality: "uncertain".into(),
                            updated: "—".into(),
                            updated_ms: 0,
                            bind_index: None,
                            topic: String::new(),
                            path: String::new(),
                        });
                    }
                }
                HubProto::Http => {
                    let conn = port.http_connection.as_str();
                    for bind in &port.http_inputs {
                        let json_path = bind.json_path.trim();
                        if json_path.is_empty() {
                            continue;
                        }
                        let source =
                            http_bind_source(conn, bind.path.trim(), json_path);
                        let id = if bind.point.trim().is_empty() {
                            "http_N".to_string()
                        } else {
                            bind.point.trim().to_string()
                        };
                        if seen.contains(&id) {
                            continue;
                        }
                        seen.insert(id.clone());
                        rows.push(PointsTableRow {
                            id,
                            source,
                            value: "—".into(),
                            quality: "uncertain".into(),
                            updated: "—".into(),
                            updated_ms: 0,
                            bind_index: None,
                            topic: String::new(),
                            path: bind.json_path.clone(),
                        });
                    }
                }
                HubProto::Streams => {
                    let endpoint = port.stream_endpoint.as_str();
                    for bind in &port.stream_inputs {
                        // Whole-line binds may leave field empty — still list them.
                        let source = stream_bind_source(endpoint, bind.field.trim());
                        let id = if bind.point.trim().is_empty() {
                            "stream_N".to_string()
                        } else {
                            bind.point.trim().to_string()
                        };
                        if seen.contains(&id) {
                            continue;
                        }
                        seen.insert(id.clone());
                        rows.push(PointsTableRow {
                            id,
                            source,
                            value: "—".into(),
                            quality: "uncertain".into(),
                            updated: "—".into(),
                            updated_ms: 0,
                            bind_index: None,
                            topic: String::new(),
                            path: bind.field.clone(),
                        });
                    }
                }
                HubProto::Modbus => {
                    let conn = port.modbus_connection.as_str();
                    for bind in &port.modbus_inputs {
                        let source = modbus_bind_source(conn, bind);
                        let id = if bind.point.trim().is_empty() {
                            "modbus_N".to_string()
                        } else {
                            bind.point.trim().to_string()
                        };
                        if seen.contains(&id) {
                            continue;
                        }
                        seen.insert(id.clone());
                        rows.push(PointsTableRow {
                            id,
                            source,
                            value: "—".into(),
                            quality: "uncertain".into(),
                            updated: "—".into(),
                            updated_ms: 0,
                            bind_index: None,
                            topic: String::new(),
                            path: bind.describe(),
                        });
                    }
                }
                HubProto::Files => {
                    let src = port.file_source.as_str();
                    for bind in &port.file_inputs {
                        let column = bind.column.trim();
                        if column.is_empty() {
                            continue;
                        }
                        let id = if bind.point.trim().is_empty() {
                            "file_N".to_string()
                        } else {
                            bind.point.trim().to_string()
                        };
                        if seen.contains(&id) {
                            continue;
                        }
                        seen.insert(id.clone());
                        rows.push(PointsTableRow {
                            id,
                            source: file_bind_source(src, column),
                            value: "—".into(),
                            quality: "uncertain".into(),
                            updated: "—".into(),
                            updated_ms: 0,
                            bind_index: None,
                            topic: String::new(),
                            path: bind.column.clone(),
                        });
                    }
                }
                HubProto::Snmp => {
                    let target = port.snmp_target.as_str();
                    for bind in &port.snmp_inputs {
                        let oid = bind.oid.trim();
                        if oid.is_empty() {
                            continue;
                        }
                        let source = snmp_bind_source(target, oid);
                        let id = if bind.point.trim().is_empty() {
                            "snmp_N".to_string()
                        } else {
                            bind.point.trim().to_string()
                        };
                        if seen.contains(&id) {
                            continue;
                        }
                        seen.insert(id.clone());
                        rows.push(PointsTableRow {
                            id,
                            source,
                            value: "—".into(),
                            quality: "uncertain".into(),
                            updated: "—".into(),
                            updated_ms: 0,
                            bind_index: None,
                            topic: String::new(),
                            path: bind.oid.clone(),
                        });
                    }
                }
                HubProto::Wits | HubProto::WebSocket | HubProto::Sparkplug | HubProto::Dnp3 | HubProto::Can => {
                    let Some(e) = port.protocol.ext() else { continue };
                    for bind in &port.ext_inputs {
                        let Some(source) = crate::ext_proto::bind_source(e, &port.ext_connection, bind) else { continue };
                        let point = crate::ext_proto::text_of(bind, "point");
                        let id = if point.trim().is_empty() { format!("{}_N", e.core().prefix) } else { point.trim().to_string() };
                        if seen.contains(&id) {
                            continue;
                        }
                        seen.insert(id.clone());
                        rows.push(PointsTableRow {
                            id,
                            source,
                            value: "—".into(),
                            quality: "uncertain".into(),
                            updated: "—".into(),
                            updated_ms: 0,
                            bind_index: None,
                            topic: String::new(),
                            path: crate::ext_proto::bind_label(e, bind),
                        });
                    }
                }
                HubProto::Las => {
                    let endpoint = port.las_endpoint.as_str();
                    for bind in &port.las_inputs {
                        let curve = bind.curve.trim();
                        if curve.is_empty() {
                            continue;
                        }
                        let source = las_bind_source(endpoint, curve);
                        let id = if bind.point.trim().is_empty() {
                            "las_N".to_string()
                        } else {
                            bind.point.trim().to_string()
                        };
                        if seen.contains(&id) {
                            continue;
                        }
                        seen.insert(id.clone());
                        rows.push(PointsTableRow {
                            id,
                            source,
                            value: "—".into(),
                            quality: "uncertain".into(),
                            updated: "—".into(),
                            updated_ms: 0,
                            bind_index: None,
                            topic: String::new(),
                            path: bind.curve.clone(),
                        });
                    }
                }
            }
        }
        for step in &hub.aggregators {
            let id = format!("agg_{}", step.id);
            if seen.contains(&id) {
                continue;
            }
            seen.insert(id.clone());
            rows.push(PointsTableRow {
                id,
                source: format!("agg/step/{}", step.id),
                value: "—".into(),
                quality: "uncertain".into(),
                updated: "—".into(),
                updated_ms: 0,
                bind_index: None,
                topic: String::new(),
                path: String::new(),
            });
        }
    }
    rows
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
    (3..=6).contains(&plen) && !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
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

fn flat(value: &str) -> String {
    value.replace('\n', " ")
}

fn text_width(ui: &egui::Ui, text: &str, font: &FontId) -> f32 {
    ui.fonts(|f| {
        f.layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE)
            .size()
            .x
    })
}

/// Pixel-accurate crop with a leading ellipsis (keeps the end of a path).
fn fit_text_keep_end(ui: &egui::Ui, text: &str, font: &FontId, max_w: f32) -> String {
    let flat = flat(text);
    if text_width(ui, &flat, font) <= max_w {
        return flat;
    }
    let chars: Vec<char> = flat.chars().collect();
    // Binary search the longest suffix that fits after "…".
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        let cand: String = std::iter::once('…')
            .chain(chars[chars.len() - mid..].iter().copied())
            .collect();
        if text_width(ui, &cand, font) <= max_w {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    std::iter::once('…')
        .chain(chars[chars.len() - lo..].iter().copied())
        .collect()
}

/// Pixel-accurate crop with a trailing ellipsis.
fn fit_text_end(ui: &egui::Ui, text: &str, font: &FontId, max_w: f32) -> String {
    let flat = flat(text);
    if text_width(ui, &flat, font) <= max_w {
        return flat;
    }
    let chars: Vec<char> = flat.chars().collect();
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        let cand: String = chars[..mid].iter().copied().chain(std::iter::once('…')).collect();
        if text_width(ui, &cand, font) <= max_w {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    chars[..lo].iter().copied().chain(std::iter::once('…')).collect()
}

pub fn format_updated(updated_ms: u64) -> String {
    if updated_ms == 0 {
        return "—".into();
    }
    let age_secs = pb_points::now_ms().saturating_sub(updated_ms) / 1000;

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
