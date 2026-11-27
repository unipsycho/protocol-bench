//! Top-level egui app: the header (dashboard name, tabs, theme, edit tools), the canvas, the
//! welcome screen for an empty dashboard and the tile editor.

use egui::{Align, Color32, Layout as ELayout, Pos2, Rect, Response, RichText, CornerRadius, Sense, Stroke, Ui, Vec2};
use pb_dash_model::valid_dashboard_name;
use std::cell::RefCell;
use std::rc::Rc;

use crate::grid_view::GridView;
use crate::state::Shared;
use crate::theme::{self, bold_id, Pal, THEMES};
use crate::widgets::{ActiveAlarm, Zone};
use std::collections::BTreeMap;

type Backend = Box<dyn FnMut(&mut Shared, f64)>;


pub struct DashApp {
    pub shared: Rc<RefCell<Shared>>,
    /// Runs the network (or the demo data) once per frame: executes queued commands.
    backend: Backend,
    grid: GridView,
    edit: bool,
    kiosk: bool,
    initial: Option<String>,
    dialog: Dialog,
    shown_hash: String,
    /// Palette the egui style was last built from (rebuilt when the theme changes).
    styled: Option<Pal>,
    /// When each point in alarm went into it (UI clock), for the header's "for 2m 10s".
    alarm_since: BTreeMap<String, (Zone, f64)>,
    /// Alarms shown last frame, and ones that cleared and are fading out.
    last_alarms: Vec<ActiveAlarm>,
    fading: Vec<Fading>,
}

#[derive(Default)]
enum Dialog {
    #[default]
    None,
    New { name: String, copy: bool },
    Rename(String),
    Delete,
}

/// Rounded "chip" button; `on` = filled with the accent.
fn chip(ui: &mut Ui, text: impl Into<String>, on: bool, pal: &Pal) -> Response {
    let txt = RichText::new(text.into()).size(13.5).color(if on { pal.bg } else { pal.text });
    let b = egui::Button::new(txt)
        .fill(if on { pal.accent } else { pal.panel2 })
        .stroke(Stroke::new(1.0, if on { pal.accent } else { pal.line }))
        .corner_radius(CornerRadius::from(16.0))
        .min_size(Vec2::new(0.0, 30.0));
    ui.add(b)
}

/// Primary call-to-action button.
fn big_button(ui: &mut Ui, text: &str, primary: bool, pal: &Pal) -> Response {
    let txt = RichText::new(text).size(15.0).color(if primary { pal.bg } else { pal.text });
    ui.add(
        egui::Button::new(txt)
            .fill(if primary { pal.accent } else { pal.panel2 })
            .stroke(Stroke::new(1.0, if primary { pal.accent } else { pal.line }))
            .corner_radius(CornerRadius::from(12.0))
            .min_size(Vec2::new(200.0, 44.0)),
    )
}

/// Two-colour swatch for the theme menu.
fn swatch(ui: &mut Ui, p: &Pal) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(34.0, 20.0), Sense::hover());
    ui.painter().rect_filled(r, 6.0, p.bg);
    ui.painter().rect_filled(Rect::from_min_size(r.min + Vec2::new(4.0, 4.0), Vec2::new(14.0, 12.0)), 3.0, p.panel);
    ui.painter().circle_filled(Pos2::new(r.max.x - 9.0, r.center().y), 5.0, p.accent);
    ui.painter().rect_stroke(r, 6.0, Stroke::new(1.0, p.line), egui::StrokeKind::Inside);
}

/// "12s", "5m 07s", "2h 05m", "3d 4h": how long a point has been in alarm.
pub fn fmt_duration(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m {:02}s", s / 60, s % 60),
        3600..=86_399 => format!("{}h {:02}m", s / 3600, (s / 60) % 60),
        _ => format!("{}d {}h", s / 86_400, (s / 3600) % 24),
    }
}

/// Keep "in alarm since" times in step with the current alarms: new ones start now, ones that
/// cleared are forgotten, and a point that flips from too high to too low starts over.
pub fn track_alarm_since(since: &mut BTreeMap<String, (Zone, f64)>, alarms: &[ActiveAlarm], now: f64) {
    since.retain(|p, (z, _)| alarms.iter().any(|a| a.point == *p && a.zone == *z));
    for a in alarms {
        since.entry(a.point.clone()).or_insert((a.zone, now));
    }
}

const PILL_H_PAD: f32 = 14.0;

/// The small "how long" text: about half the name size, never below 11 px.
fn dur_px(name_px: f32) -> f32 {
    (name_px * 0.5).max(11.0)
}

/// Size of one alarm pill: dot, name at the dashboard's label size, then the small duration.
fn pill_size(ui: &Ui, a: &ActiveAlarm, dur: &str, name_px: f32) -> Vec2 {
    let n = ui.painter().layout_no_wrap(a.name.clone(), bold_id(name_px), Color32::WHITE).size();
    let d = ui.painter().layout_no_wrap(dur.to_string(), egui::FontId::proportional(dur_px(name_px)), Color32::WHITE).size();
    Vec2::new(PILL_H_PAD + 12.0 + 8.0 + n.x + 8.0 + d.x + PILL_H_PAD, n.y + 12.0)
}

/// An alarm as drawn in the header this frame.
pub struct ShownAlarm {
    pub a: ActiveAlarm,
    pub dur: String,
    /// 1 = fully visible; falls to 0 over a second once the alarm clears.
    pub alpha: f32,
    /// Seconds since the alarm came on (drives the arrival flash), `None` while fading out.
    pub age: Option<f64>,
}

/// How long the arrival flash and the fade-out last.
const FLASH_SECS: f64 = 1.0;
const FADE_SECS: f64 = 1.0;

/// Cleared alarms linger here while they fade.
pub struct Fading {
    pub a: ActiveAlarm,
    pub dur: String,
    pub cleared_at: f64,
}

/// Build this frame's header list: current alarms (with their age) followed by ones that just
/// cleared, fading out. Updates `fading` from what was shown last frame.
pub fn shown_alarms(alarms: &[ActiveAlarm], last: &[ActiveAlarm], since: &BTreeMap<String, (Zone, f64)>, fading: &mut Vec<Fading>, now: f64) -> Vec<ShownAlarm> {
    let dur_of = |p: &str| fmt_duration(since.get(p).map(|s| now - s.1).unwrap_or(0.0));
    for old in last {
        if !alarms.iter().any(|a| a.point == old.point && a.zone == old.zone) {
            // Freeze its timer at the moment it cleared (`since` still holds it this frame).
            fading.retain(|f| f.a.point != old.point);
            fading.push(Fading { a: old.clone(), dur: dur_of(&old.point), cleared_at: now });
        }
    }
    fading.retain(|f| now - f.cleared_at < FADE_SECS && !alarms.iter().any(|a| a.point == f.a.point));
    let mut out: Vec<ShownAlarm> = alarms
        .iter()
        .map(|a| ShownAlarm { a: a.clone(), dur: dur_of(&a.point), alpha: 1.0, age: Some(since.get(&a.point).map(|s| now - s.1).unwrap_or(0.0)) })
        .collect();
    out.extend(fading.iter().map(|f| ShownAlarm {
        a: f.a.clone(),
        dur: f.dur.clone(),
        alpha: (1.0 - (now - f.cleared_at) / FADE_SECS).clamp(0.0, 1.0) as f32,
        age: None,
    }));
    out
}

/// One alarm pill: pulsing glow, dot, name and a small "how long" note, all in the zone colour.
/// On arrival it flashes bright and sends out two expanding rings; when cleared it fades away.
fn paint_alarm_pill(ui: &Ui, r: Rect, s: &ShownAlarm, name_px: f32, pal: &Pal, now: f64) {
    let col = s.a.zone.color(pal.alarm, pal);
    let pulse = 0.5 + 0.5 * ((now * 3.2).sin() as f32);
    let mut p = ui.painter().clone();
    // Fade in quickly on arrival, out slowly when cleared.
    let alpha = match s.age {
        Some(age) => (age / 0.15).clamp(0.0, 1.0) as f32,
        None => s.alpha,
    };
    p.set_opacity(alpha);
    let rr = r.height() / 2.0;
    if let Some(age) = s.age.filter(|a| *a < FLASH_SECS) {
        let t = (age / FLASH_SECS) as f32;
        let fx = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("pb_alarm_flash")));
        // Two rings racing outwards, fading as they go.
        for (delay, grow, width) in [(0.0, 34.0, 4.0), (0.25, 22.0, 2.5)] {
            let k = ((t - delay) / (1.0 - delay)).clamp(0.0, 1.0);
            if k > 0.0 && k < 1.0 {
                let e = 1.0 - (1.0 - k).powi(3); // ease out
                let ring = r.expand(3.0 + grow * e);
                fx.rect_stroke(ring, ring.height() / 2.0, Stroke::new(width * (1.0 - k), col.gamma_multiply(1.0 - k)), egui::StrokeKind::Middle);
            }
        }
        // A bright bloom behind the pill.
        let bloom = (1.0 - t).powi(2);
        fx.rect_filled(r.expand(6.0 + 10.0 * t), rr + 8.0, col.gamma_multiply(0.35 * bloom));
    }
    p.rect_filled(r.expand(2.0 + 3.0 * pulse), rr + 4.0, col.gamma_multiply(0.10 + 0.12 * pulse));
    p.rect_filled(r, rr, col.gamma_multiply(0.22));
    // The arrival flash: the pill lights up white-hot in its colour, then settles.
    if let Some(age) = s.age.filter(|a| *a < FLASH_SECS * 0.6) {
        let k = 1.0 - (age / (FLASH_SECS * 0.6)) as f32;
        p.rect_filled(r, rr, col.gamma_multiply(0.7 * k * k));
        p.rect_filled(r.shrink(2.0), rr, Color32::from_white_alpha((90.0 * k * k) as u8));
    }
    p.rect_stroke(r, rr, Stroke::new(1.5, col), egui::StrokeKind::Inside);
    let dot = Pos2::new(r.min.x + PILL_H_PAD + 6.0, r.center().y);
    p.circle_filled(dot, 4.5 + 3.0 * pulse, col.gamma_multiply(0.35));
    p.circle_filled(dot, 4.5, col);
    let name = p.layout_no_wrap(s.a.name.clone(), bold_id(name_px), pal.text);
    let nx = dot.x + 6.0 + 8.0;
    let ny = r.center().y - name.size().y / 2.0;
    let (name_w, name_h) = (name.size().x, name.size().y);
    p.galley(Pos2::new(nx, ny), name, pal.text);
    // Duration sits on the name's baseline, small and quiet.
    let d = p.layout_no_wrap(s.dur.clone(), egui::FontId::proportional(dur_px(name_px)), pal.soft);
    let dy = ny + name_h * 0.78 - d.size().y * 0.8;
    p.galley(Pos2::new(nx + name_w + 8.0, dy), d, pal.soft);
}

/// The alarm pills, centred on the header within `area` (between the name and the buttons). As
/// many as fit; the rest collapse into "+N more". Pills glide when neighbours come and go.
fn alarm_strip(ui: &mut Ui, area: Rect, center_x: f32, shown: &[ShownAlarm], pal: &Pal, now: f64) {
    if area.width() < 80.0 || shown.is_empty() {
        return;
    }
    let name_px = crate::widgets::title_size().min(28.0);
    let gap = 10.0;
    let sizes: Vec<Vec2> = shown.iter().map(|s| pill_size(ui, &s.a, &s.dur, name_px)).collect();
    let more_w = 90.0;
    let mut n = 0;
    let mut total = 0.0;
    while n < shown.len() {
        let w = total + sizes[n].x + if n > 0 { gap } else { 0.0 };
        let need_more = n + 1 < shown.len();
        if w + if need_more { more_w } else { 0.0 } > area.width() && n > 0 {
            break;
        }
        total = w;
        n += 1;
    }
    let rest = shown.len() - n;
    let full = total + if rest > 0 { gap + more_w } else { 0.0 };
    let x0 = (center_x - full / 2.0).clamp(area.min.x, (area.max.x - full).max(area.min.x));
    let mut x = x0;
    for (i, s) in shown[..n].iter().enumerate() {
        let gx = ui.ctx().animate_value_with_time(egui::Id::new(("pb_alarm_x", &s.a.point)), x, 0.25);
        let r = Rect::from_min_size(Pos2::new(gx, area.center().y - sizes[i].y / 2.0), sizes[i]);
        paint_alarm_pill(ui, r, s, name_px, pal, now);
        if s.age.is_some() {
            ui.interact(r, egui::Id::new(("pb_alarm", &s.a.point)), Sense::hover())
                .on_hover_text(format!("{}: {}\nIn alarm for {}", s.a.name, s.a.detail, s.dur));
        }
        x += sizes[i].x + gap;
    }
    if rest > 0 {
        let r = Rect::from_min_size(Pos2::new(x, area.center().y - 14.0), Vec2::new(more_w, 28.0));
        ui.painter().text(r.left_center(), egui::Align2::LEFT_CENTER, format!("+{rest} more"), bold_id(15.0), pal.alarm);
        let tip: Vec<String> = shown[n..].iter().map(|s| format!("{}: {} ({})", s.a.name, s.a.detail, s.dur)).collect();
        ui.interact(r, egui::Id::new("pb_alarm_more"), Sense::hover()).on_hover_text(tip.join("\n"));
    }
}

/// The page background: a soft vertical gradient.
fn paint_background(ui: &Ui, r: Rect, pal: &Pal) {
    let mut m = egui::Mesh::default();
    m.colored_vertex(r.left_top(), pal.bg);
    m.colored_vertex(r.right_top(), pal.bg);
    m.colored_vertex(r.right_bottom(), pal.bg2);
    m.colored_vertex(r.left_bottom(), pal.bg2);
    m.add_triangle(0, 1, 2);
    m.add_triangle(0, 2, 3);
    ui.painter().add(egui::Shape::mesh(m));
}

impl DashApp {
    pub fn new(cc: &eframe::CreationContext<'_>, shared: Rc<RefCell<Shared>>, backend: Backend, kiosk: bool, initial: Option<String>) -> Self {
        theme::install_fonts(&cc.egui_ctx);
        Self { shared, backend, grid: GridView::default(), edit: false, kiosk, initial, dialog: Dialog::None, shown_hash: String::new(), styled: None, alarm_since: BTreeMap::new(), last_alarms: Vec::new(), fading: Vec::new() }
    }

    fn set_edit(&mut self, on: bool, sh: &mut Shared) {
        if self.edit && !on {
            sh.flush_save();
            self.grid.selected = None;
        }
        self.edit = on;
    }

    /// Add a tile for every point not on the dashboard yet, typed by its data, and tidy up.
    fn add_all_points(&mut self, sh: &mut Shared, now: f64) -> usize {
        let before = sh.layout.clone();
        let added = sh.layout.add_points(&sh.point_suggestions()).len();
        if added > 0 {
            sh.edited(before, now);
        }
        added
    }

    fn header(&mut self, ui: &mut Ui, sh: &mut Shared, pal: &Pal, now: f64) {
        egui::Panel::top("pb_dash_top")
            .frame(egui::Frame::NONE.fill(pal.panel).inner_margin(egui::Margin { left: 24, right: 24, top: 16, bottom: 12 }))
            .show(ui, |ui| {
                // One divider under the header (a frame stroke would box the whole page edge).
                let y = ui.max_rect().max.y + 12.0 - 0.5;
                ui.painter().hline(ui.clip_rect().x_range(), y, Stroke::new(1.0, pal.line));
                // Row 1: name + status on the left; clock, theme, menu and Edit on the right.
                let alarms = crate::widgets::active_alarms(&sh.layout, &sh.points);
                // Uses last frame's start times, so call before they are updated.
                let shown = shown_alarms(&alarms, &self.last_alarms, &self.alarm_since, &mut self.fading, now);
                track_alarm_since(&mut self.alarm_since, &alarms, now);
                self.last_alarms = alarms.clone();
                let row = ui.horizontal(|ui| {
                    let left = ui.vertical(|ui| {
                        let name = if sh.current.is_empty() { "Dashboard" } else { sh.current.as_str() };
                        ui.label(RichText::new(name).font(bold_id(30.0)).color(pal.text));
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            let (dot, tip) = if sh.connected { (pal.ok, "Live") } else { (pal.alarm, "Reconnecting…") };
                            let (r, _) = ui.allocate_exact_size(Vec2::new(12.0, 12.0), Sense::hover());
                            let pulse = if sh.connected { 0.5 + 0.5 * ((now * 2.4).sin() as f32) } else { 1.0 };
                            ui.painter().circle_filled(r.center(), 6.0, dot.gamma_multiply(0.25 * pulse));
                            ui.painter().circle_filled(r.center(), 3.5, dot);
                            ui.label(RichText::new(tip).size(13.0).color(dot));
                            let site = if sh.title.is_empty() { "Protocol Bench" } else { sh.title.as_str() };
                            ui.label(RichText::new(format!("·  {site}  ·  {} points  ·  {} tiles", sh.points.len(), sh.layout.tiles.len())).size(13.0).color(pal.muted));
                            if !sh.last_error.is_empty() {
                                ui.label(RichText::new(format!("·  {}", sh.last_error)).size(13.0).color(pal.warn));
                            }
                        });
                    }).response.rect;
                    let right_min = ui.with_layout(ELayout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(crate::net::clock_hm()).font(bold_id(26.0)).color(pal.soft));
                        ui.add_space(10.0);
                        if sh.can_edit {
                            let label = if self.edit { "Done" } else { "Edit" };
                            if chip(ui, label, self.edit, pal).on_hover_text("Arrange, add and style tiles").clicked() {
                                let on = !self.edit;
                                self.set_edit(on, sh);
                            }
                            self.dashboards_menu(ui, sh, pal);
                            self.theme_menu(ui, sh, pal, now);
                        } else if sh.names.len() > 1 {
                            // Read-only viewers can still switch dashboards.
                            self.dashboards_menu(ui, sh, pal);
                        }
                        ui.min_rect().min.x
                    }).inner;
                    (left, right_min)
                });
                // Alarms: centred on the header, in the free space between name and buttons.
                if !shown.is_empty() {
                    let (left, right_min) = row.inner;
                    let r = row.response.rect;
                    let area = Rect::from_min_max(Pos2::new(left.max.x + 24.0, r.min.y), Pos2::new(right_min - 24.0, r.max.y));
                    alarm_strip(ui, area, ui.max_rect().center().x, &shown, pal, now);
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(60)); // pulse + timer
                }
                ui.add_space(8.0);
            });
    }

    /// The editing tools at the top of the sidebar (the header keeps one height in both modes).
    fn edit_tools(&mut self, ui: &mut Ui, sh: &mut Shared, pal: &Pal, now: f64) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Editing").font(bold_id(20.0)).color(pal.text));
            ui.with_layout(ELayout::right_to_left(Align::Center), |ui| {
                if ui.add_enabled(!sh.redo.is_empty(), egui::Button::new("Redo").corner_radius(14.0)).on_hover_text("Redo (Ctrl+Y)").clicked() {
                    sh.redo(now);
                }
                if ui.add_enabled(!sh.undo.is_empty(), egui::Button::new("Undo").corner_radius(14.0)).on_hover_text("Undo (Ctrl+Z)").clicked() {
                    sh.undo(now);
                }
            });
        });
        ui.label(RichText::new("Changes save automatically.").size(13.5).color(pal.muted));
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
            if chip(ui, "+  Tile", true, pal).on_hover_text("Choose a widget to add").clicked() {
                self.grid.pick_new_tile();
            }
            let missing = sh.layout.clone().add_points(&sh.point_suggestions()).len();
            let r = ui.add_enabled_ui(missing > 0, |ui| chip(ui, format!("✨  Add all points ({missing})"), false, pal)).inner;
            if r.on_hover_text("One tile per point not shown yet, with a widget picked from its data type").clicked() {
                self.add_all_points(sh, now);
            }
            if chip(ui, "Auto arrange", false, pal).on_hover_text("Group tiles by type and line them up in tidy rows").clicked() {
                let before = sh.layout.clone();
                sh.layout.auto_arrange();
                sh.edited(before, now);
            }
            self.label_size_menu(ui, sh, pal, now);
        });
    }

    /// Size of the point names on tiles: presets plus a fine-tune slider (per dashboard).
    fn label_size_menu(&mut self, ui: &mut Ui, sh: &mut Shared, pal: &Pal, now: f64) {
        ui.menu_button(RichText::new("Label size"), |ui| {
            ui.label(RichText::new("TILE LABELS").font(bold_id(12.5)).color(pal.muted));
            let cur = if sh.layout.title_size == 0 { 16 } else { sh.layout.title_size };
            let mut pick = None;
            for (name, px) in [("Small", 13), ("Medium (default)", 16), ("Large", 20), ("Extra large", 26), ("Wall screen", 34)] {
                if ui.selectable_label(cur == px, RichText::new(name).size((px as f32).min(22.0))).clicked() {
                    pick = Some(px);
                    ui.close();
                }
            }
            ui.separator();
            let mut v = cur;
            if ui.add(egui::Slider::new(&mut v, 10..=48).suffix(" px")).changed() {
                pick = Some(v);
            }
            if let Some(px) = pick {
                let before = sh.layout.clone();
                sh.layout.title_size = if px == 16 { 0 } else { px };
                sh.edited(before, now);
            }
        })
        .response
        .on_hover_text("How big point names and titles are on the tiles (saved with this dashboard)");
    }

    fn theme_menu(&mut self, ui: &mut Ui, sh: &mut Shared, pal: &Pal, now: f64) {
        ui.menu_button(RichText::new("Theme").size(13.5), |ui| {
            ui.label(RichText::new("THEME").size(11.0).color(pal.muted));
            let system_dark = ui.ctx().theme() == egui::Theme::Dark;
            for t in &THEMES {
                ui.horizontal(|ui| {
                    swatch(ui, &Pal::of(t.key, system_dark));
                    if ui.selectable_label(sh.layout.theme == t.key, t.name).clicked() {
                        let before = sh.layout.clone();
                        sh.layout.theme = t.key.to_string();
                        sh.edited(before, now);
                        ui.close();
                    }
                });
            }
        })
        .response
        .on_hover_text("Colour theme (saved with this dashboard)");
    }

    /// Switch between dashboards, and (for editors) create, duplicate, rename or delete them.
    fn dashboards_menu(&mut self, ui: &mut Ui, sh: &mut Shared, pal: &Pal) {
        ui.menu_button(RichText::new("Dashboards").size(13.5), |ui| {
            ui.set_min_width(220.0);
            ui.label(RichText::new("OPEN").font(bold_id(12.0)).color(pal.muted));
            let names = sh.names.clone();
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                for n in &names {
                    let current = *n == sh.current;
                    let text = if current { RichText::new(n).font(bold_id(14.5)).color(pal.accent) } else { RichText::new(n) };
                    if ui.selectable_label(current, text).clicked() {
                        sh.open(n);
                        self.grid.selected = None;
                        ui.close();
                    }
                }
            });
            if !sh.can_edit {
                return;
            }
            ui.separator();
            ui.label(RichText::new("MANAGE").font(bold_id(12.0)).color(pal.muted));
            if ui.button("New dashboard…").clicked() {
                self.dialog = Dialog::New { name: String::new(), copy: false };
                ui.close();
            }
            if ui.button(format!("Duplicate “{}”…", sh.current)).clicked() {
                self.dialog = Dialog::New { name: format!("{} copy", sh.current), copy: true };
                ui.close();
            }
            if ui.button("Rename…").clicked() {
                self.dialog = Dialog::Rename(sh.current.clone());
                ui.close();
            }
            if ui.button(RichText::new("Delete…").color(pal.alarm)).clicked() {
                self.dialog = Dialog::Delete;
                ui.close();
            }
        })
        .response
        .on_hover_text("Switch dashboards; the link in the address bar opens the current one");
    }

    /// Empty dashboard (view mode): a friendly start screen that builds the page in one click.
    fn welcome(&mut self, ui: &mut Ui, sh: &mut Shared, pal: &Pal, now: f64) {
        let n = sh.points.len();
        let area = ui.max_rect();
        let card = Rect::from_center_size(Pos2::new(area.center().x, area.min.y + (area.height() * 0.42).max(220.0)), Vec2::new(560.0f32.min(area.width() - 40.0), 320.0));
        crate::widgets::card_bg(ui.painter(), card, pal, pal.line);
        ui.scope_builder(egui::UiBuilder::new().max_rect(card.shrink(36.0)), |ui| {
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("✨").size(40.0));
                ui.add_space(4.0);
                ui.label(RichText::new("Let's build this dashboard").font(bold_id(26.0)).color(pal.text));
                ui.add_space(6.0);
                if !sh.can_edit {
                    ui.label(RichText::new("This dashboard is empty.").size(15.0).color(pal.soft));
                    return;
                }
                let msg = if n == 0 {
                    "No points yet — wire points into this Dashboard output in Protocol Bench and they will appear here.".to_string()
                } else {
                    format!("{n} point{} available. Add them all as tiles — numbers, switches and text each get a fitting widget — or lay it out yourself.", if n == 1 { " is" } else { "s are" })
                };
                ui.label(RichText::new(msg).size(15.0).color(pal.soft));
                ui.add_space(22.0);
                ui.horizontal(|ui| {
                    let w = 200.0 * 2.0 + ui.spacing().item_spacing.x;
                    ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
                    if ui.add_enabled_ui(n > 0, |ui| big_button(ui, &format!("✨  Add all {n} points"), true, pal)).inner.clicked() {
                        self.add_all_points(sh, now);
                    }
                    if big_button(ui, "Start from scratch", false, pal).clicked() {
                        self.set_edit(true, sh);
                    }
                });
            });
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context, sh: &mut Shared, pal: &Pal) {
        let mut close = false;
        match &mut self.dialog {
            Dialog::None => {}
            Dialog::New { name, copy } => {
                let ok = valid_dashboard_name(name) && !sh.names.contains(name);
                let title = if *copy { "Duplicate dashboard" } else { "New dashboard" };
                egui::Window::new(title).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO).show(ctx, |ui| {
                    ui.add(egui::TextEdit::singleline(name).hint_text("name").desired_width(260.0));
                    if !name.is_empty() && !ok {
                        ui.label(RichText::new("Letters, digits, space, - and _ (up to 48); must be new.").size(12.0).color(pal.warn));
                    }
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if ui.add_enabled(ok, egui::Button::new(RichText::new("Create").color(pal.bg)).fill(pal.accent)).clicked() {
                            sh.create_dashboard(name, *copy);
                            close = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                });
            }
            Dialog::Rename(name) => {
                let ok = valid_dashboard_name(name) && (!sh.names.contains(name) || *name == sh.current);
                egui::Window::new("Rename dashboard").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO).show(ctx, |ui| {
                    ui.add(egui::TextEdit::singleline(name).desired_width(260.0));
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if ui.add_enabled(ok, egui::Button::new(RichText::new("Rename").color(pal.bg)).fill(pal.accent)).clicked() {
                            sh.rename_dashboard(name);
                            close = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                });
            }
            Dialog::Delete => {
                egui::Window::new("Delete dashboard").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO).show(ctx, |ui| {
                    ui.label(format!("Delete “{}”? This cannot be undone.", sh.current));
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if ui.add(egui::Button::new(RichText::new("Delete").color(Color32::WHITE)).fill(pal.alarm)).clicked() {
                            sh.delete_dashboard();
                            self.grid.selected = None;
                            close = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                });
            }
        }
        if close {
            self.dialog = Dialog::None;
        }
    }
}

impl eframe::App for DashApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = &ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        let shared = self.shared.clone();
        let mut sh = shared.borrow_mut();
        let sh = &mut *sh;
        let pal = Pal::of(&sh.layout.theme, ctx.theme() == egui::Theme::Dark);
        crate::widgets::set_title_size(sh.layout.title_size);
        if self.styled != Some(pal) {
            theme::apply_style(ctx, &pal);
            self.styled = Some(pal);
        }

        // First dashboard: the one in the URL, else the first saved, else a fresh "Main".
        if sh.names_loaded && sh.current.is_empty() {
            let want = self.initial.take().filter(|n| sh.names.contains(n));
            match want.or_else(|| sh.names.first().cloned()) {
                Some(n) => sh.open(&n),
                None => sh.create_dashboard("Main", false),
            }
        }
        if !sh.can_edit {
            self.edit = false;
        }
        let edit = self.edit && sh.can_edit;

        // Keyboard.
        if edit && ctx.memory(|m| m.focused().is_none()) {
            let (undo, redo, del, esc) = ctx.input(|i| {
                (
                    i.modifiers.command && !i.modifiers.shift && i.key_pressed(egui::Key::Z),
                    i.modifiers.command && (i.key_pressed(egui::Key::Y) || (i.modifiers.shift && i.key_pressed(egui::Key::Z))),
                    i.key_pressed(egui::Key::Delete),
                    i.key_pressed(egui::Key::Escape),
                )
            });
            if undo {
                sh.undo(now);
            }
            if redo {
                sh.redo(now);
            }
            if del {
                if let Some(id) = self.grid.selected.take() {
                    let before = sh.layout.clone();
                    sh.layout.remove(id);
                    sh.edited(before, now);
                }
            }
            if esc {
                self.grid.selected = None;
            }
        }

        if !self.kiosk {
            self.header(ui, sh, &pal, now);
        }
        if edit {
            // Sidebar: tools on top, then the selected tile's editor (or how-to help).
            egui::Panel::right("pb_dash_editor")
                .default_size(360.0)
                .min_size(300.0)
                .frame(egui::Frame::NONE.fill(pal.panel).inner_margin(egui::Margin::same(18)))
                .show(ui, |ui| {
                    let x = ui.max_rect().min.x - 18.0 + 0.5;
                    ui.painter().vline(x, ui.clip_rect().y_range(), Stroke::new(1.0, pal.line));
                    self.edit_tools(ui, sh, &pal, now);
                    ui.add_space(12.0);
                    let y = ui.cursor().min.y;
                    ui.painter().hline(ui.max_rect().x_range(), y, Stroke::new(1.0, pal.line));
                    ui.add_space(14.0);
                    match self.grid.selected {
                        Some(id) => {
                            if !crate::editor::show(ui, sh, id, &pal, now) {
                                self.grid.selected = None;
                            }
                        }
                        None => crate::editor::show_help(ui, &pal),
                    }
                });
        }
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(pal.bg)).show(ui, |ui| {
            paint_background(ui, ui.max_rect(), &pal);
            if !sh.layout_loaded {
                ui.centered_and_justified(|ui| {
                    ui.label(RichText::new(if sh.names_loaded { "Loading dashboard…" } else { "Connecting…" }).size(16.0).color(pal.muted));
                });
            } else if sh.layout.tiles.is_empty() && !edit {
                self.welcome(ui, sh, &pal, now);
            } else {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    self.grid.show(ui, sh, &pal, edit, now);
                });
            }
        });
        self.dialogs(ctx, sh, &pal);
        sh.save_when_idle(now);
        // Keep the address bar in step with the dashboard being shown.
        if self.shown_hash != sh.current {
            self.shown_hash = sh.current.clone();
            crate::net::set_hash(&sh.current);
        }
        (self.backend)(sh, now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alarm(point: &str, zone: Zone) -> ActiveAlarm {
        ActiveAlarm { point: point.into(), name: point.into(), zone, detail: String::new() }
    }

    #[test]
    fn durations_read_naturally() {
        assert_eq!(fmt_duration(9.7), "9s");
        assert_eq!(fmt_duration(307.0), "5m 07s");
        assert_eq!(fmt_duration(7500.0), "2h 05m");
        assert_eq!(fmt_duration(3.0 * 86_400.0 + 4.0 * 3600.0), "3d 4h");
        assert_eq!(fmt_duration(-5.0), "0s");
    }

    #[test]
    fn cleared_alarms_fade_out_over_a_second() {
        let mut since = BTreeMap::new();
        let mut fading = Vec::new();
        let on = [alarm("a", Zone::AlarmHi)];
        track_alarm_since(&mut since, &on, 0.0);
        let s = shown_alarms(&on, &on, &since, &mut fading, 65.0);
        assert_eq!((s.len(), s[0].dur.as_str(), s[0].alpha), (1, "1m 05s", 1.0));
        // Clears at t = 70: still shown, timer frozen, fading.
        let s = shown_alarms(&[], &on, &since, &mut fading, 70.0);
        assert_eq!((s.len(), s[0].dur.as_str(), s[0].age), (1, "1m 10s", None));
        track_alarm_since(&mut since, &[], 70.0);
        let s = shown_alarms(&[], &[], &since, &mut fading, 70.5);
        assert!((s[0].alpha - 0.5).abs() < 1e-3, "half faded");
        assert_eq!(s[0].dur, "1m 10s");
        assert!(shown_alarms(&[], &[], &since, &mut fading, 71.1).is_empty(), "gone after a second");
        // Coming back while fading replaces the ghost with the live alarm.
        let _ = shown_alarms(&[], &on, &since, &mut fading, 80.0);
        track_alarm_since(&mut since, &on, 80.2);
        let s = shown_alarms(&on, &[], &since, &mut fading, 80.2);
        assert_eq!(s.len(), 1);
        assert!(s[0].age.is_some());
    }

    #[test]
    fn alarm_timers_start_clear_and_restart() {
        let mut since = BTreeMap::new();
        track_alarm_since(&mut since, &[alarm("a", Zone::AlarmHi)], 10.0);
        track_alarm_since(&mut since, &[alarm("a", Zone::AlarmHi), alarm("b", Zone::AlarmLo)], 20.0);
        assert_eq!((since["a"].1, since["b"].1), (10.0, 20.0), "a keeps its start time");
        track_alarm_since(&mut since, &[alarm("a", Zone::AlarmLo)], 30.0);
        assert_eq!(since["a"].1, 30.0, "too high → too low starts over");
        assert!(!since.contains_key("b"), "cleared alarms are forgotten");
        track_alarm_since(&mut since, &[], 40.0);
        assert!(since.is_empty());
    }
}
