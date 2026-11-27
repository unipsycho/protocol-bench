//! The dashboard canvas: draws the layout on its snapping grid and, in edit mode, handles every
//! gesture — move by the grip, resize by any corner or edge, drop on a tile's centre to swap or
//! on its edge to split, drag on empty space to draw a new tile.
//!
//! All the rules (collisions, snapping, swap / split) live in `pb_dash_model::grid`; this file
//! only turns pointer movement into cells and previews the result live.

use egui::{Align2, Color32, CursorIcon, FontId, Id, Pos2, Rect, CornerRadius, Sense, Shape, Stroke, Ui, Vec2};
use pb_dash_model::grid::{drop_zone, snap_cells, Edge, Rect as Cells};
use pb_dash_model::{min_size, Layout, Tile, Widget};

use crate::chart;
use crate::state::Shared;
use crate::theme::bold_id;
use crate::widgets::{self, Pal};

const MARGIN: f32 = 22.0;
/// Space between neighbouring tiles.
const GAP: f32 = 16.0;
const GRIP_H: f32 = 32.0;
/// Seconds a tile takes to glide to a new spot.
const GLIDE: f32 = 0.16;
/// Below this page width the grid stacks into one column (phones).
const STACK_BELOW: f32 = 640.0;

const L: u8 = 1;
const R: u8 = 2;
const T: u8 = 4;
const B: u8 = 8;

#[derive(Clone)]
enum Kind {
    Move,
    Resize(u8),
}

#[derive(Clone)]
struct Drag {
    id: u32,
    kind: Kind,
    base: Layout,
    start_ptr: Pos2,
    start: Cells,
}

/// What dropping the dragged tile right now would do (for the highlight).
#[derive(Clone, Copy, PartialEq, Debug)]
enum Drop {
    None,
    Swap(u32),
    Split(u32, Edge),
}

#[derive(Default)]
pub struct GridView {
    drag: Option<Drag>,
    /// Cell rectangle being drawn on empty space (start cell, current cell).
    draw: Option<((i32, i32), (i32, i32))>,
    /// A rectangle was drawn: choose the widget for it.
    /// The new-tile picker is open: `Some(Some(rect))` for a drawn rectangle, `Some(None)` for
    /// "first free spot" (the toolbar's + Tile).
    pending: Option<Option<Cells>>,
    pub selected: Option<u32>,
}

struct Geo {
    origin: Pos2,
    cell: f32,
}

impl Geo {
    fn rectf(&self, x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(self.origin + Vec2::new(x * self.cell, y * self.cell), Vec2::new(w * self.cell, h * self.cell))
    }
    fn rect(&self, x: u32, y: u32, w: u32, h: u32) -> Rect {
        self.rectf(x as f32, y as f32, w as f32, h as f32)
    }
    fn tile(&self, t: &Tile) -> Rect {
        self.rect(t.x, t.y, t.w, t.h)
    }
    fn cell_at(&self, p: Pos2, cols: u32) -> (i32, i32) {
        let v = (p - self.origin) / self.cell;
        ((v.x.floor() as i32).clamp(0, cols as i32 - 1), (v.y.floor() as i32).max(0))
    }
    /// A tile's on-screen rectangle, gliding from where it was to where it now belongs.
    /// Animated in cell units so scrolling and window resizes never lag.
    fn glide(&self, ctx: &egui::Context, t: &Tile) -> Rect {
        let a = |k: u8, v: u32| ctx.animate_value_with_time(Id::new(("pb_glide", t.id, k)), v as f32, GLIDE);
        self.rectf(a(0, t.x), a(1, t.y), a(2, t.w), a(3, t.h)).shrink(GAP / 2.0)
    }
}

fn paint_tile(ui: &mut Ui, rect: Rect, t: &Tile, sh: &Shared, pal: &Pal, hover_ok: bool) {
    if t.widget == Widget::Chart {
        chart::paint(ui, rect, t, sh, pal, hover_ok);
    } else {
        let hist = sh.history.get(&t.point).map(|h| h.as_slice());
        widgets::paint(&ui.painter_at(rect.expand(28.0)), rect, t, sh.points.get(&t.point), hist, sh.server_now, pal);
    }
}

/// The dragged tile as it follows the pointer (charts as a plain card: no hover / legend).
fn paint_ghost(p: &egui::Painter, rect: Rect, t: &Tile, sh: &Shared, pal: &Pal) {
    if t.widget == Widget::Chart {
        let inner = widgets::card(p, rect, t, pal, false);
        p.text(inner.center(), Align2::CENTER_CENTER, "Chart", FontId::proportional(14.0), pal.muted);
    } else {
        let hist = sh.history.get(&t.point).map(|h| h.as_slice());
        widgets::paint(p, rect, t, sh.points.get(&t.point), hist, sh.server_now, pal);
    }
}

/// Rounded rectangle drawn with a dashed outline (drop targets).
fn dashed_rect(p: &egui::Painter, r: Rect, col: Color32, phase: f32) {
    let pts = vec![r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    let dash = 9.0;
    let gap = 6.0;
    p.extend(Shape::dashed_line_with_offset(&pts, Stroke::new(2.0, col), &[dash], &[gap], phase));
}

/// Small rounded label (sizes, "Swap with …").
fn pill(p: &egui::Painter, at: Pos2, text: &str, fg: Color32, bg: Color32) -> Rect {
    let g = p.layout_no_wrap(text.to_string(), bold_id(13.0), fg);
    let r = Rect::from_center_size(at, g.size() + Vec2::new(20.0, 10.0));
    p.add(egui::epaint::Shadow { offset: [0, 4], blur: 12, spread: 0, color: Color32::from_black_alpha(60) }.as_shape(r, CornerRadius::from(r.height() / 2.0)));
    p.rect_filled(r, r.height() / 2.0, bg);
    p.galley(r.min + Vec2::new(10.0, 5.0), g, fg);
    r
}

impl GridView {
    pub fn show(&mut self, ui: &mut Ui, sh: &mut Shared, pal: &Pal, edit: bool, now: f64) {
        let width = ui.available_width();
        if width < STACK_BELOW && !edit {
            self.stacked(ui, sh, pal);
            return;
        }
        let cols = sh.layout.cols.max(1);
        let cell = ((width - 2.0 * MARGIN + GAP) / cols as f32).max(8.0);
        let view_h = ui.available_height();
        let rows = sh.layout.rows() + if edit { 6 } else { 1 };
        let height = (rows as f32 * cell + 2.0 * MARGIN).max(view_h);
        let (canvas, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
        // Tiles are inset by GAP / 2, so start half a gap early to keep the outer margin exact.
        let geo = Geo { origin: canvas.min + Vec2::splat(MARGIN - GAP / 2.0), cell };
        let ptr = ui.ctx().pointer_interact_pos();
        let before = sh.layout.clone();
        let ctx = ui.ctx().clone();

        // Background: draw a new tile by dragging on empty space.
        if edit {
            let bg = ui.interact(canvas, Id::new("pb_dash_bg"), Sense::click_and_drag());
            if bg.drag_started() {
                self.draw = ptr.map(|p| (geo.cell_at(p, cols), geo.cell_at(p, cols)));
                self.selected = None;
            }
            if bg.dragged() {
                if let (Some((s, _)), Some(p)) = (self.draw, ptr) {
                    self.draw = Some((s, geo.cell_at(p, cols)));
                }
            }
            if bg.drag_stopped() {
                if let Some((a, b)) = self.draw.take() {
                    let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
                    let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
                    self.pending = Some(Some(Cells { x: x0 as u32, y: y0 as u32, w: (x1 - x0 + 1) as u32, h: (y1 - y0 + 1) as u32 }));
                }
            } else if bg.clicked() {
                self.selected = None;
            }
            draw_grid_dots(ui, canvas, &geo, cols, rows, pal);
            if sh.layout.tiles.is_empty() && self.draw.is_none() {
                let c = Pos2::new(canvas.center().x, canvas.min.y + 160.0);
                ui.painter().text(c, Align2::CENTER_CENTER, "Drag on the grid to draw a tile", bold_id(20.0), pal.soft);
                ui.painter().text(c + Vec2::new(0.0, 30.0), Align2::CENTER_CENTER, "…or use  + Tile  /  Add all points  in the toolbar", FontId::proportional(14.0), pal.muted);
            }
        }

        // While dragging, show the previewed layout (others shift live); otherwise the real one.
        let (layout, drop) = match (&self.drag, ptr) {
            (Some(d), Some(p)) => preview(d, p, &geo, cols),
            _ => (sh.layout.clone(), Drop::None),
        };
        let moving = self.drag.as_ref().filter(|d| matches!(d.kind, Kind::Move)).map(|d| d.id);
        let swap_partner = match drop {
            Drop::Swap(id) => Some(id),
            _ => None,
        };

        let mut start_drag: Option<Drag> = None;
        let mut delete: Option<u32> = None;
        let mut select: Option<u32> = None;
        for t in &layout.tiles {
            let rect = geo.glide(&ctx, t);
            if moving == Some(t.id) {
                continue; // drawn as the floating ghost + its drop placeholder below
            }
            let hover_ok = self.drag.is_none() && !edit;
            paint_tile(ui, rect, t, sh, pal, hover_ok);
            if !edit {
                continue;
            }
            let selected = self.selected == Some(t.id);
            let resizing = self.drag.as_ref().is_some_and(|d| d.id == t.id);
            let painter = ui.painter_at(rect.expand(8.0));
            if selected || resizing {
                painter.rect_stroke(rect.expand(1.0), widgets::CARD_R + 1.0, Stroke::new(2.0, pal.accent), egui::StrokeKind::Inside);
            }
            if swap_partner == Some(t.id) {
                // The tile that will trade places, shown already sliding into the vacated slot.
                painter.rect_filled(rect, widgets::CARD_R, pal.accent.gamma_multiply(0.10));
                painter.rect_stroke(rect.expand(1.0), widgets::CARD_R + 1.0, Stroke::new(2.0, pal.accent), egui::StrokeKind::Inside);
                pill(&painter, rect.center(), "moves here", pal.bg, pal.accent);
            }
            // Body click selects (opens the editor).
            let body = ui.interact(rect, Id::new(("pb_dash_body", t.id)), Sense::click());
            if body.clicked() {
                select = Some(t.id);
            }
            // Short tiles (headings) only show their chrome when hovered, so the content stays visible.
            let near = ptr.is_some_and(|p| rect.expand(10.0).contains(p));
            if rect.height() < 1.6 * GRIP_H && !(near || selected || resizing) {
                continue;
            }
            // Header strip: grip, title, pencil and ×.
            // The header replaces the card's title band exactly (labels have no band).
            let band = if t.widget == Widget::Label { GRIP_H } else { widgets::title_band(rect).max(GRIP_H).min(rect.height()) };
            let strip = Rect::from_min_max(rect.min, Pos2::new(rect.max.x, rect.min.y + band));
            // Narrow tiles keep only ×; clicking the body opens the editor just the same.
            let narrow = rect.width() < 170.0;
            let grip = Rect::from_min_max(rect.min, Pos2::new(rect.max.x - if narrow { 34.0 } else { 64.0 }, strip.max.y));
            let top = CornerRadius { nw: widgets::CARD_R as u8, ne: widgets::CARD_R as u8, sw: 0, se: 0 };
            painter.rect_filled(strip, top, pal.panel2);
            painter.line_segment([strip.left_bottom(), strip.right_bottom()], Stroke::new(1.0, pal.line));
            let g = ui
                .interact(grip, Id::new(("pb_dash_grip", t.id)), Sense::drag())
                .on_hover_text(format!("{} · {}×{}\nDrag to move. Drop on the middle of another tile to swap, on its edge to share its space.", t.widget.title(), t.w, t.h));
            let dot = if g.hovered() { pal.accent } else { pal.muted };
            for i in 0..3 {
                for j in 0..2 {
                    painter.circle_filled(Pos2::new(rect.min.x + 13.0 + j as f32 * 5.0, strip.center().y - 5.0 + i as f32 * 5.0), 1.5, dot);
                }
            }
            let name = widgets::tile_title(t);
            let label = if name.is_empty() { t.widget.title().to_string() } else { name };
            let room = (grip.width() - 34.0).max(10.0);
            let lg = painter.layout_no_wrap(widgets::ellipsize(&painter, &label, widgets::title_size(), room), FontId::proportional(widgets::title_size()), pal.text);
            painter.galley(Pos2::new(rect.min.x + 30.0, strip.center().y - lg.size().y / 2.0), lg, pal.text);
            if g.hovered() {
                ctx.set_cursor_icon(CursorIcon::Grab);
            }
            if g.drag_started() {
                select = Some(t.id);
                if let Some(p) = ptr {
                    start_drag = Some(Drag { id: t.id, kind: Kind::Move, base: sh.layout.clone(), start_ptr: p, start: Cells::of(t) });
                }
            }
            let pencil = Rect::from_center_size(Pos2::new(rect.max.x - 44.0, strip.center().y), Vec2::splat(24.0));
            let cross = Rect::from_center_size(Pos2::new(rect.max.x - 17.0, strip.center().y), Vec2::splat(24.0));
            let pr = if narrow { ui.interact(Rect::NOTHING, Id::new(("pb_dash_pencil", t.id)), Sense::hover()) } else { ui.interact(pencil, Id::new(("pb_dash_pencil", t.id)), Sense::click()).on_hover_text("Edit this tile") };
            let xr = ui.interact(cross, Id::new(("pb_dash_del", t.id)), Sense::click()).on_hover_text("Remove this tile");
            if pr.hovered() {
                painter.circle_filled(pencil.center(), 12.0, pal.accent.gamma_multiply(0.18));
            }
            if xr.hovered() {
                painter.circle_filled(cross.center(), 12.0, pal.alarm.gamma_multiply(0.18));
            }
            if !narrow {
                draw_pencil(&painter, pencil, if pr.hovered() || selected { pal.accent } else { pal.soft });
            }
            draw_cross(&painter, cross, if xr.hovered() { pal.alarm } else { pal.soft });
            if pr.clicked() {
                select = Some(t.id);
            }
            if xr.clicked() {
                delete = Some(t.id);
            }
            // Resize handles: four corners and four edges.
            for (flags, hr, icon) in handle_rects(rect) {
                let hid = Id::new(("pb_dash_handle", t.id, flags));
                let hresp = ui.interact(hr, hid, Sense::drag());
                if hresp.hovered() || (resizing && matches!(&self.drag, Some(Drag { kind: Kind::Resize(f), .. }) if *f == flags)) {
                    ctx.set_cursor_icon(icon);
                }
                if hresp.drag_started() {
                    select = Some(t.id);
                    if let Some(p) = ptr {
                        start_drag = Some(Drag { id: t.id, kind: Kind::Resize(flags), base: sh.layout.clone(), start_ptr: p, start: Cells::of(t) });
                    }
                }
            }
            if resizing {
                pill(&painter, rect.center(), &format!("{} × {}", t.w, t.h), pal.bg, pal.accent);
            }
        }

        // The tile being moved: a placeholder exactly where it will land, and the tile itself
        // floating under the pointer.
        if let (Some(d), Some(id), Some(p)) = (&self.drag, moving, ptr) {
            let fg = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::new("pb_dash_drag")));
            let phase = (now as f32 * 30.0) % 15.0;
            let mut label: Option<(Pos2, String)> = None;
            if let Some(dest) = layout.tile(id) {
                let r = geo.glide(&ctx, dest);
                fg.rect_filled(r, widgets::CARD_R, pal.accent.gamma_multiply(0.12));
                dashed_rect(&fg, r.shrink(1.0), pal.accent, phase);
                let msg = match drop {
                    Drop::Swap(other) => format!("Swap with {}", d.base.tile(other).map(tile_name).unwrap_or_default()),
                    Drop::Split(_, edge) => format!("Dock {}", match edge { Edge::Left => "left", Edge::Right => "right", Edge::Top => "above", Edge::Bottom => "below" }),
                    Drop::None => format!("Move here  {} × {}", dest.w, dest.h),
                };
                label = Some((Pos2::new(r.center().x, r.max.y - 22.0), msg));
            }
            if let Some(t) = d.base.tile(id) {
                let r = geo.tile(t).shrink(GAP / 2.0).translate(p - d.start_ptr);
                let mut ghost = fg.clone();
                ghost.set_opacity(0.85);
                ghost.add(egui::epaint::Shadow { offset: [0, 18], blur: 40, spread: 0, color: Color32::from_black_alpha(120) }.as_shape(r, CornerRadius::from(widgets::CARD_R)));
                paint_ghost(&ghost, r, t, sh, pal);
                ghost.rect_stroke(r, widgets::CARD_R, Stroke::new(2.0, pal.accent), egui::StrokeKind::Inside);
            }
            // The label goes on top of everything, so the floating tile never hides it.
            if let Some((at, msg)) = label {
                let top = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, Id::new("pb_dash_drag_label")));
                pill(&top, at, &msg, pal.bg, pal.accent);
            }
            ctx.set_cursor_icon(CursorIcon::Grabbing);
            ctx.request_repaint();
        }
        // The rectangle being drawn on empty space.
        if let Some((a, b)) = self.draw {
            let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
            let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
            let r = geo.rect(x0 as u32, y0 as u32, (x1 - x0 + 1) as u32, (y1 - y0 + 1) as u32).shrink(GAP / 2.0);
            ui.painter().rect_filled(r, widgets::CARD_R, pal.accent.gamma_multiply(0.12));
            dashed_rect(ui.painter(), r, pal.accent, 0.0);
            pill(ui.painter(), r.center(), &format!("{} × {}", x1 - x0 + 1, y1 - y0 + 1), pal.bg, pal.accent);
        }

        // Apply this frame's results. A drag ends when the button comes up, wherever it is.
        let released = self.drag.is_some() && !ui.input(|i| i.pointer.primary_down());
        if released {
            self.drag = None;
            if ptr.is_some() {
                sh.layout = layout;
            }
        }
        if let Some(d) = start_drag {
            self.drag = Some(d);
        }
        if let Some(id) = select {
            self.selected = Some(id);
        }
        if let Some(id) = delete {
            sh.layout.remove(id);
            if self.selected == Some(id) {
                self.selected = None;
            }
        }
        if let Some(spot) = self.pending {
            // Keep the drawn rectangle on screen while choosing what goes in it.
            if let Some(r) = spot {
                let rr = geo.rect(r.x, r.y, r.w, r.h).shrink(GAP / 2.0);
                ui.painter().rect_filled(rr, widgets::CARD_R, pal.accent.gamma_multiply(0.12));
                dashed_rect(ui.painter(), rr, pal.accent, 0.0);
            }
            if let Some(w) = self.widget_picker(ui, pal, spot) {
                let id = match spot {
                    Some(r) => sh.layout.add_rect(w, r),
                    None => sh.layout.add(w, None),
                };
                self.selected = Some(id);
                self.pending = None;
            }
        }
        if sh.layout != before {
            sh.edited(before, now);
        }
    }

    /// Open the new-tile picker for the first free spot.
    pub fn pick_new_tile(&mut self) {
        self.pending = Some(None);
    }

    /// The new-tile picker: a card per widget with a small preview and what it is for.
    fn widget_picker(&mut self, ui: &Ui, pal: &Pal, spot: Option<Cells>) -> Option<Widget> {
        let mut chosen = None;
        let mut close = false;
        let frame = egui::Frame::NONE
            .fill(pal.panel)
            .stroke(Stroke::new(1.0, pal.line))
            .corner_radius(CornerRadius::same(18))
            .inner_margin(egui::Margin::same(24))
            .shadow(egui::epaint::Shadow { offset: [0, 18], blur: 48, spread: 0, color: Color32::from_black_alpha(140) });
        let resp = egui::Modal::new(Id::new("pb_new_tile"))
            .frame(frame)
            .backdrop_color(Color32::from_black_alpha(110))
            .show(ui.ctx(), |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("Add a tile").font(bold_id(22.0)).color(pal.text));
                        let sub = match spot {
                            Some(r) => format!("What should this {} × {} space show?", r.w, r.h),
                            None => "Pick a widget; it goes in the first free spot.".to_string(),
                        };
                        ui.label(egui::RichText::new(sub).size(15.0).color(pal.soft));
                    });
                });
                ui.add_space(16.0);
                let card = Vec2::new(168.0, 120.0);
                egui::Grid::new("pb_new_tile_grid").spacing(Vec2::splat(12.0)).show(ui, |ui| {
                    for (i, w) in Widget::ALL.into_iter().enumerate() {
                        if widget_card(ui, w, card, pal).clicked() {
                            chosen = Some(w);
                        }
                        if i % 4 == 3 {
                            ui.end_row();
                        }
                    }
                });
                ui.add_space(14.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::Button::new(egui::RichText::new("Cancel").size(15.0)).corner_radius(14.0)).clicked() {
                        close = true;
                    }
                    ui.label(egui::RichText::new("Esc or click outside to cancel").size(13.0).color(pal.muted));
                });
            });
        // Esc / click on the backdrop (but not the click that picked a card).
        if resp.should_close() && chosen.is_none() {
            close = true;
        }
        if close {
            self.pending = None;
        }
        chosen
    }

    /// Narrow screens: one column in reading order, no editing.
    fn stacked(&mut self, ui: &mut Ui, sh: &Shared, pal: &Pal) {
        let width = ui.available_width() - 2.0 * MARGIN;
        let row = 30.0;
        ui.add_space(MARGIN);
        for id in sh.layout.reading_order() {
            let Some(t) = sh.layout.tile(id) else { continue };
            let (rect, _) = ui.allocate_exact_size(Vec2::new(width, (t.h as f32 * row).max(80.0)), Sense::hover());
            let rect = rect.translate(Vec2::new(MARGIN, 0.0));
            paint_tile(ui, rect, t, sh, pal, true);
            ui.add_space(GAP);
        }
    }
}

/// What each widget is for, shown in the picker.
fn widget_blurb(w: Widget) -> &'static str {
    match w {
        Widget::Numeric => "Live number + trend",
        Widget::Gauge => "Dial with zones",
        Widget::Bar => "Level bar with zones",
        Widget::Indicator => "On / off lamp",
        Widget::Chart => "Points over time",
        Widget::Text => "Text or status",
        Widget::Label => "Section heading",
    }
}

/// One clickable card in the new-tile picker: a drawn preview, the name and a blurb.
fn widget_card(ui: &mut Ui, w: Widget, size: Vec2, pal: &Pal) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(size, Sense::click());
    let hot = resp.hovered();
    let t = ui.ctx().animate_bool(resp.id, hot);
    let r = r.translate(Vec2::new(0.0, -2.0 * t));
    let p = ui.painter();
    if hot {
        p.add(egui::epaint::Shadow { offset: [0, 8], blur: 20, spread: 0, color: pal.accent.gamma_multiply(0.25) }.as_shape(r, CornerRadius::same(14)));
    }
    p.rect_filled(r, 14.0, if hot { pal.panel2 } else { pal.bg.lerp_to_gamma(pal.panel, 0.5) });
    p.rect_stroke(r, 14.0, Stroke::new(if hot { 2.0 } else { 1.0 }, if hot { pal.accent } else { pal.line }), egui::StrokeKind::Inside);
    let icon = Rect::from_center_size(Pos2::new(r.center().x, r.min.y + 38.0), Vec2::new(72.0, 44.0));
    draw_widget_icon(p, icon, w, pal, hot);
    p.text(Pos2::new(r.center().x, r.min.y + 78.0), Align2::CENTER_CENTER, w.title(), bold_id(15.5), pal.text);
    p.text(Pos2::new(r.center().x, r.min.y + 100.0), Align2::CENTER_CENTER, widget_blurb(w), FontId::proportional(13.0), pal.muted);
    if hot {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    resp
}

/// Tiny preview of a widget for the picker.
fn draw_widget_icon(p: &egui::Painter, r: Rect, w: Widget, pal: &Pal, hot: bool) {
    let a = if hot { pal.accent } else { pal.accent.gamma_multiply(0.85) };
    let c = r.center();
    match w {
        Widget::Numeric => {
            p.text(c - Vec2::new(0.0, 6.0), Align2::CENTER_CENTER, "42.7", bold_id(22.0), a);
            let pts: Vec<Pos2> = (0..=12).map(|i| Pos2::new(r.min.x + 6.0 + i as f32 * 5.0, r.max.y - 6.0 - 4.0 * ((i as f32) * 0.9).sin())).collect();
            p.add(Shape::line(pts, Stroke::new(1.8, a.gamma_multiply(0.7))));
        }
        Widget::Gauge => {
            use std::f32::consts::PI;
            let arc = |a0: f32, a1: f32, col: Color32| {
                let pts: Vec<Pos2> = (0..=24).map(|i| {
                    let t = a0 + (a1 - a0) * i as f32 / 24.0;
                    Pos2::new(c.x + 19.0 * t.cos(), c.y + 3.0 + 19.0 * t.sin())
                }).collect();
                p.add(Shape::line(pts, Stroke::new(5.0, col)));
            };
            arc(PI * 0.75, PI * 2.25, pal.line);
            arc(PI * 0.75, PI * 1.7, a);
        }
        Widget::Bar => {
            let track = Rect::from_center_size(c + Vec2::new(0.0, 6.0), Vec2::new(64.0, 10.0));
            p.rect_filled(track, 5.0, pal.line);
            p.rect_filled(Rect::from_min_max(track.min, Pos2::new(track.min.x + 42.0, track.max.y)), 5.0, a);
            p.text(Pos2::new(track.min.x, track.min.y - 10.0), Align2::LEFT_CENTER, "68 %", bold_id(13.0), pal.text);
        }
        Widget::Indicator => {
            for (k, al) in [(1.7, 0.08), (1.35, 0.16)] {
                p.circle_filled(c, 11.0 * k, pal.ok.gamma_multiply(al));
            }
            p.circle_filled(c, 11.0, pal.ok);
            p.circle_filled(c + Vec2::new(-3.5, -3.5), 3.0, Color32::from_white_alpha(80));
        }
        Widget::Chart => {
            let l1: Vec<Pos2> = (0..=14).map(|i| Pos2::new(r.min.x + 4.0 + i as f32 * 4.6, c.y + 10.0 * ((i as f32) * 0.7).sin())).collect();
            let l2: Vec<Pos2> = (0..=14).map(|i| Pos2::new(r.min.x + 4.0 + i as f32 * 4.6, c.y + 4.0 + 8.0 * ((i as f32) * 0.45 + 2.0).cos())).collect();
            p.line_segment([Pos2::new(r.min.x + 2.0, r.max.y - 2.0), Pos2::new(r.max.x - 2.0, r.max.y - 2.0)], Stroke::new(1.0, pal.line));
            p.add(Shape::line(l2, Stroke::new(2.0, pal.warn)));
            p.add(Shape::line(l1, Stroke::new(2.0, a)));
        }
        Widget::Text => {
            p.text(c, Align2::CENTER_CENTER, "AUTO", bold_id(20.0), pal.text);
        }
        Widget::Label => {
            p.text(Pos2::new(r.min.x + 4.0, c.y - 2.0), Align2::LEFT_CENTER, "Heading", bold_id(17.0), pal.text);
            p.rect_filled(Rect::from_min_size(Pos2::new(r.min.x + 4.0, c.y + 11.0), Vec2::new(24.0, 3.0)), 2.0, a);
        }
    }
}

fn tile_name(t: &Tile) -> String {
    let n = widgets::tile_title(t);
    if n.is_empty() { t.widget.title().to_string() } else { n }
}

/// Where the dragged tile would end up if released at `ptr`, and what kind of drop it is.
fn preview(d: &Drag, ptr: Pos2, geo: &Geo, cols: u32) -> (Layout, Drop) {
    let mut l = d.base.clone();
    let delta = ptr - d.start_ptr;
    let (dx, dy) = (snap_cells(delta.x, geo.cell), snap_cells(delta.y, geo.cell));
    match d.kind {
        Kind::Move => {
            // Pointer over another tile: swap (centre) or split (edge).
            let (cx, cy) = geo.cell_at(ptr, cols);
            let over = d.base.tiles.iter().find(|t| {
                t.id != d.id && (cx as u32) >= t.x && (cx as u32) < t.x + t.w && (cy as u32) >= t.y && (cy as u32) < t.y + t.h
            });
            if let Some(t) = over {
                let r = geo.tile(t);
                let (fx, fy) = ((ptr.x - r.min.x) / r.width(), (ptr.y - r.min.y) / r.height());
                match drop_zone(fx, fy) {
                    Some(edge) => {
                        let mut probe = d.base.clone();
                        if probe.split_drop(d.id, t.id, edge) {
                            return (probe, Drop::Split(t.id, edge));
                        }
                    }
                    None => {
                        let mut probe = d.base.clone();
                        if probe.swap(d.id, t.id) {
                            return (probe, Drop::Swap(t.id));
                        }
                    }
                }
            }
            let x = (d.start.x as i32 + dx).clamp(0, (cols - d.start.w) as i32) as u32;
            let y = (d.start.y as i32 + dy).max(0) as u32;
            l.move_to(d.id, x, y);
            (l, Drop::None)
        }
        Kind::Resize(flags) => {
            let (mw, mh) = d.base.tile(d.id).map(|t| min_size(t.widget)).unwrap_or((1, 1));
            let (mut x0, mut x1) = (d.start.x as i32, (d.start.x + d.start.w) as i32);
            let (mut y0, mut y1) = (d.start.y as i32, (d.start.y + d.start.h) as i32);
            if flags & L != 0 {
                x0 = (x0 + dx).clamp(0, x1 - mw as i32);
            }
            if flags & R != 0 {
                x1 = (x1 + dx).clamp(x0 + mw as i32, cols as i32);
            }
            if flags & T != 0 {
                y0 = (y0 + dy).clamp(0, y1 - mh as i32);
            }
            if flags & B != 0 {
                y1 = (y1 + dy).max(y0 + mh as i32);
            }
            l.resize_to(d.id, Cells { x: x0 as u32, y: y0 as u32, w: (x1 - x0) as u32, h: (y1 - y0) as u32 });
            (l, Drop::None)
        }
    }
}

/// The eight resize zones of a tile: (flags, rect, cursor).
fn handle_rects(r: Rect) -> Vec<(u8, Rect, CursorIcon)> {
    let c = 14.0; // corner size
    let e = 7.0; // edge thickness
    let mut v = Vec::new();
    let corner = |p: Pos2| Rect::from_center_size(p, Vec2::splat(c));
    v.push((L | T, corner(r.left_top()), CursorIcon::ResizeNwSe));
    v.push((R | T, corner(r.right_top()), CursorIcon::ResizeNeSw));
    v.push((L | B, corner(r.left_bottom()), CursorIcon::ResizeNeSw));
    v.push((R | B, corner(r.right_bottom()), CursorIcon::ResizeNwSe));
    v.push((L, Rect::from_min_max(Pos2::new(r.min.x - e / 2.0, r.min.y + c / 2.0), Pos2::new(r.min.x + e / 2.0, r.max.y - c / 2.0)), CursorIcon::ResizeHorizontal));
    v.push((R, Rect::from_min_max(Pos2::new(r.max.x - e / 2.0, r.min.y + c / 2.0), Pos2::new(r.max.x + e / 2.0, r.max.y - c / 2.0)), CursorIcon::ResizeHorizontal));
    v.push((T, Rect::from_min_max(Pos2::new(r.min.x + c / 2.0, r.min.y - e / 2.0), Pos2::new(r.max.x - c / 2.0, r.min.y + e / 2.0)), CursorIcon::ResizeVertical));
    v.push((B, Rect::from_min_max(Pos2::new(r.min.x + c / 2.0, r.max.y - e / 2.0), Pos2::new(r.max.x - c / 2.0, r.max.y + e / 2.0)), CursorIcon::ResizeVertical));
    v
}

fn draw_grid_dots(ui: &Ui, canvas: Rect, geo: &Geo, cols: u32, rows: u32, pal: &Pal) {
    let painter = ui.painter_at(canvas);
    let visible = ui.clip_rect();
    for y in 0..=rows {
        let py = geo.origin.y + y as f32 * geo.cell;
        if py < visible.min.y - geo.cell || py > visible.max.y + geo.cell {
            continue;
        }
        for x in 0..=cols {
            painter.circle_filled(Pos2::new(geo.origin.x + x as f32 * geo.cell, py), 1.4, pal.line);
        }
    }
}

fn draw_pencil(p: &egui::Painter, r: Rect, col: Color32) {
    let s = Stroke::new(1.6, col);
    let c = r.center();
    p.line_segment([c + Vec2::new(-5.0, 5.0), c + Vec2::new(4.0, -4.0)], Stroke::new(3.0, col));
    p.line_segment([c + Vec2::new(-6.0, 6.5), c + Vec2::new(-3.0, 6.0)], s);
    p.line_segment([c + Vec2::new(-6.0, 6.5), c + Vec2::new(-5.5, 3.5)], s);
}

fn draw_cross(p: &egui::Painter, r: Rect, col: Color32) {
    let s = Stroke::new(1.6, col);
    let c = r.center();
    p.line_segment([c + Vec2::new(-4.0, -4.0), c + Vec2::new(4.0, 4.0)], s);
    p.line_segment([c + Vec2::new(-4.0, 4.0), c + Vec2::new(4.0, -4.0)], s);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geo() -> Geo {
        Geo { origin: Pos2::new(10.0, 10.0), cell: 20.0 }
    }

    fn base() -> (Layout, u32, u32) {
        let mut l = Layout::default();
        let a = l.add(Widget::Numeric, Some((0, 0)));
        let b = l.add(Widget::Numeric, Some((8, 0)));
        (l, a, b)
    }

    fn drag(l: &Layout, id: u32, kind: Kind, from: Pos2) -> Drag {
        Drag { id, kind, base: l.clone(), start_ptr: from, start: Cells::of(l.tile(id).unwrap()) }
    }

    #[test]
    fn moving_by_whole_cells_snaps() {
        let (l, a, _) = base();
        let d = drag(&l, a, Kind::Move, Pos2::new(30.0, 30.0));
        // 52 px right = 2.6 cells → 3 cells; 18 px down = 0.9 → 1 cell. Pointer lands in a free cell.
        let (out, drop) = preview(&d, Pos2::new(30.0 + 52.0, 30.0 + 18.0), &geo(), 24);
        assert_eq!(drop, Drop::None);
        let t = out.tile(a).unwrap();
        assert_eq!((t.x, t.y), (3, 1));
    }

    #[test]
    fn corner_drag_resizes_and_keeps_the_minimum() {
        let (l, a, _) = base();
        let d = drag(&l, a, Kind::Resize(R | B), Pos2::new(90.0, 70.0));
        let (out, _) = preview(&d, Pos2::new(90.0 + 40.0, 70.0 + 20.0), &geo(), 24);
        let t = out.tile(a).unwrap();
        assert_eq!((t.x, t.y, t.w, t.h), (0, 0, 6, 4));
        let (out, _) = preview(&d, Pos2::new(-500.0, -500.0), &geo(), 24);
        let t = out.tile(a).unwrap();
        assert_eq!((t.w, t.h), (2, 2), "numeric stays at its 2×2 minimum");
        // Left edge drag moves x and shrinks w together.
        let d = drag(&l, a, Kind::Resize(L), Pos2::new(10.0, 30.0));
        let (out, _) = preview(&d, Pos2::new(10.0 + 20.0, 30.0), &geo(), 24);
        let t = out.tile(a).unwrap();
        assert_eq!((t.x, t.w), (1, 3));
    }

    #[test]
    fn dropping_on_a_tile_centre_swaps_and_on_its_edge_splits() {
        let (l, a, b) = base();
        // Tile b spans cells x 8..12 → px 170..250, y 10..70.
        let d = drag(&l, a, Kind::Move, Pos2::new(30.0, 30.0));
        let (out, drop) = preview(&d, Pos2::new(210.0, 40.0), &geo(), 24);
        assert_eq!(drop, Drop::Swap(b));
        assert_eq!(out.tile(a).unwrap().x, 8);
        let (out, drop) = preview(&d, Pos2::new(176.0, 40.0), &geo(), 24);
        assert_eq!(drop, Drop::Split(b, Edge::Left));
        let (ta, tb) = (out.tile(a).unwrap(), out.tile(b).unwrap());
        assert_eq!((ta.x, ta.w, tb.x, tb.w), (8, 2, 10, 2));
    }

    #[test]
    fn too_small_targets_fall_back_to_a_plain_move() {
        let mut l = Layout::default();
        let g = l.add(Widget::Gauge, Some((0, 0))); // 4×4
        let n = l.add(Widget::Numeric, Some((10, 0)));
        let d = drag(&l, n, Kind::Move, Pos2::new(220.0, 30.0));
        // Left edge of the gauge: halves would be 2 wide, gauge needs 3 → no split, plain move.
        let (_, drop) = preview(&d, Pos2::new(14.0, 30.0), &geo(), 24);
        assert!(matches!(drop, Drop::None) || matches!(drop, Drop::Swap(_)), "never a split here");
        let _ = g;
    }

    #[test]
    fn handles_cover_corners_and_edges() {
        let h = handle_rects(Rect::from_min_size(Pos2::ZERO, Vec2::new(100.0, 60.0)));
        assert_eq!(h.len(), 8);
        let flags: Vec<u8> = h.iter().map(|x| x.0).collect();
        for f in [L | T, R | T, L | B, R | B, L, R, T, B] {
            assert!(flags.contains(&f));
        }
    }
}
