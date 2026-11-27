//! Tile editor (side panel): flip a tile to edit mode, connect it to a point, tune how it looks.

use egui::{Color32, RichText, Ui};
use pb_dash_model::{Series, Tile, Widget};

use crate::chart::series_color;
use crate::state::Shared;
use crate::widgets::Pal;

/// Sidebar content in edit mode when no tile is selected: how editing works. (The app owns the
/// sidebar panel, so the canvas never changes width when a tile is picked.)
pub fn show_help(ui: &mut egui::Ui, pal: &Pal) {
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.label(RichText::new("HOW IT WORKS").font(crate::theme::bold_id(12.5)).color(pal.muted));
            ui.add_space(8.0);
            for (icon, head, line) in [
                ("1", "Quick start", "Add all points (above) builds a tile for every point, typed by its data. Auto arrange lines everything up."),
                ("2", "Edit a tile", "Click a tile (or its pencil) to pick its point, widget and alarm limits."),
                ("3", "Move", "Drag a tile by its header. The dashed outline shows where it will land."),
                ("4", "Swap", "Drop on the middle of another tile to trade places; the other tile slides into the gap."),
                ("5", "Share space", "Drop on another tile's edge to split its space between the two."),
                ("6", "Resize", "Drag any corner or edge; sizes snap to the grid."),
                ("7", "Draw", "Drag on empty grid space to draw a tile exactly where you want it."),
                ("8", "Keys", "Ctrl+Z undo · Ctrl+Y redo · Delete removes the selected tile."),
            ] {
                ui.horizontal_top(|ui| {
                    let (r, _) = ui.allocate_exact_size(egui::vec2(22.0, 20.0), egui::Sense::hover());
                    ui.painter().circle_filled(r.center(), 10.0, pal.accent.gamma_multiply(0.18));
                    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, icon, crate::theme::bold_id(12.0), pal.accent);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(head).size(15.0).color(pal.text));
                        ui.label(RichText::new(line).size(14.0).color(pal.soft));
                    });
                });
                ui.add_space(6.0);
            }
        });
}

/// Sidebar content for the selected tile. Returns false when the user closed it.
pub fn show(ui: &mut egui::Ui, sh: &mut Shared, selected: u32, pal: &Pal, now: f64) -> bool {
    let Some(tile) = sh.layout.tile(selected).cloned() else { return false };
    let names: Vec<String> = sh.points.keys().cloned().collect();
    let mut t = tile.clone();
    let mut open = true;
    let mut delete = false;
    {
            ui.horizontal(|ui| {
                ui.label(RichText::new(tile.widget.title()).font(crate::theme::bold_id(20.0)).color(pal.text));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::Button::new(RichText::new("Done").color(pal.bg)).fill(pal.accent).corner_radius(14.0)).clicked() {
                        open = false;
                    }
                });
            });
            ui.add_space(6.0);
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                edit_tile(ui, &mut t, &names, pal);
                ui.add_space(14.0);
                if ui.add(egui::Button::new(RichText::new("Remove this tile").color(pal.alarm))).clicked() {
                    delete = true;
                }
            });
    }
    if delete {
        let before = sh.layout.clone();
        sh.layout.remove(selected);
        sh.edited(before, now);
        return false;
    }
    if t != tile {
        let before = sh.layout.clone();
        if let Some(slot) = sh.layout.tile_mut(selected) {
            *slot = t;
        }
        sh.layout.normalize();
        sh.edited(before, now);
    }
    open
}

fn field(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        // Fixed-width label column, left-aligned, full-strength colour (faded text blurs on dark).
        let (r, _) = ui.allocate_exact_size(egui::vec2(104.0, 30.0), egui::Sense::hover());
        let col = ui.visuals().widgets.noninteractive.fg_stroke.color;
        ui.painter().text(r.left_center(), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(15.0), col);
        add(ui);
    });
}

fn edit_tile(ui: &mut Ui, t: &mut Tile, names: &[String], pal: &Pal) {
    ui.spacing_mut().item_spacing.y = 8.0;
    field(ui, "Widget", |ui| {
        egui::ComboBox::from_id_salt("pb_ed_widget").selected_text(t.widget.title()).width(160.0).show_ui(ui, |ui| {
            for w in Widget::ALL {
                ui.selectable_value(&mut t.widget, w, w.title());
            }
        });
    });
    if t.widget != Widget::Label {
        field(ui, "Title", |ui| {
            ui.add(egui::TextEdit::singleline(&mut t.title).desired_width(200.0).hint_text("optional"));
        });
    }
    match t.widget {
        Widget::Label => {
            field(ui, "Text", |ui| {
                ui.add(egui::TextEdit::multiline(&mut t.text).desired_width(200.0).desired_rows(2));
            });
        }
        Widget::Chart => chart_fields(ui, t, names, pal),
        _ => {
            field(ui, "Point", |ui| {
                point_combo(ui, "pb_ed_point", &mut t.point, names);
            });
        }
    }
    match t.widget {
        Widget::Numeric => {
            field(ui, "Units", |ui| {
                ui.add(egui::TextEdit::singleline(&mut t.units).desired_width(90.0).hint_text("psi"));
            });
            decimals(ui, t);
            field(ui, "Trend line", |ui| {
                let mut on = !t.no_trend;
                if ui.checkbox(&mut on, "last 5 minutes").changed() {
                    t.no_trend = !on;
                }
            });
            zones(ui, t, pal);
        }
        Widget::Text => decimals(ui, t),
        Widget::Gauge | Widget::Bar => {
            field(ui, "Units", |ui| {
                ui.add(egui::TextEdit::singleline(&mut t.units).desired_width(90.0).hint_text("psi"));
            });
            field(ui, "Range", |ui| {
                let (mut lo, mut hi) = (t.min.unwrap_or(0.0), t.max.unwrap_or(100.0));
                let a = ui.add(egui::DragValue::new(&mut lo).speed(1.0)).changed();
                ui.label("to");
                let b = ui.add(egui::DragValue::new(&mut hi).speed(1.0)).changed();
                if a || b || t.min.is_none() {
                    t.min = Some(lo);
                    t.max = Some(hi);
                }
            });
            decimals(ui, t);
            zones(ui, t, pal);
        }
        Widget::Indicator => {
            field(ui, "On text", |ui| {
                ui.add(egui::TextEdit::singleline(&mut t.on_text).desired_width(200.0).hint_text("ON"));
            });
            field(ui, "Off text", |ui| {
                ui.add(egui::TextEdit::singleline(&mut t.off_text).desired_width(200.0).hint_text("OFF"));
            });
        }
        Widget::Chart | Widget::Label => {}
    }
}

/// Warn / alarm limits. Past them the value (and its stretch of trend line) turns amber or red;
/// between them the tile keeps its normal colour.
fn zones(ui: &mut Ui, t: &mut Tile, pal: &Pal) {
    ui.add_space(4.0);
    ui.label(RichText::new("ALARM ZONES").font(crate::theme::bold_id(12.5)).color(pal.muted));
    ui.label(RichText::new("Too high turns amber / red, too low blue / purple; in between the tile uses the theme colour.").size(13.5).color(pal.muted));
    zone_num(ui, "Alarm above", pal.alarm, &mut t.alarm_hi);
    zone_num(ui, "Warn above", pal.warn, &mut t.warn_hi);
    zone_num(ui, "Warn below", pal.warn_lo, &mut t.warn_lo);
    zone_num(ui, "Alarm below", pal.alarm_lo, &mut t.alarm_lo);
}

fn zone_num(ui: &mut Ui, label: &str, col: Color32, v: &mut Option<f64>) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(egui::vec2(104.0, 30.0), egui::Sense::hover());
        ui.painter().circle_filled(egui::pos2(r.min.x + 5.0, r.center().y), 5.0, col);
        let fg = ui.visuals().widgets.noninteractive.fg_stroke.color;
        ui.painter().text(egui::pos2(r.min.x + 16.0, r.center().y), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(15.0), fg);
        let mut on = v.is_some();
        if ui.checkbox(&mut on, "").changed() {
            *v = if on { Some(0.0) } else { None };
        }
        if let Some(x) = v {
            ui.add(egui::DragValue::new(x).speed(1.0));
        }
    });
}

fn hex(c: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}

fn decimals(ui: &mut Ui, t: &mut Tile) {
    field(ui, "Decimals", |ui| {
        let mut auto = t.decimals.is_none();
        if ui.checkbox(&mut auto, "auto").changed() {
            t.decimals = if auto { None } else { Some(1) };
        }
        if let Some(d) = &mut t.decimals {
            let mut v = *d as u32;
            if ui.add(egui::DragValue::new(&mut v).range(0..=6)).changed() {
                *d = v as u8;
            }
        }
    });
}


/// Searchable point list; returns true when the choice changed.
pub fn point_combo(ui: &mut Ui, id: &str, value: &mut String, names: &[String]) -> bool {
    let before = value.clone();
    let shown = if value.is_empty() { "(pick a point)".to_string() } else { value.clone() };
    let filter_id = egui::Id::new((id, "filter"));
    egui::ComboBox::from_id_salt(id).selected_text(shown).width(190.0).height(320.0).show_ui(ui, |ui| {
        let mut filter: String = ui.data_mut(|d| d.get_temp(filter_id)).unwrap_or_default();
        ui.add(egui::TextEdit::singleline(&mut filter).hint_text("search").desired_width(170.0));
        let f = filter.to_lowercase();
        ui.data_mut(|d| d.insert_temp(filter_id, filter));
        ui.separator();
        if names.is_empty() {
            ui.label(RichText::new("No points yet — wire some into the Dashboard node.").weak());
        }
        for n in names.iter().filter(|n| f.is_empty() || n.to_lowercase().contains(&f)) {
            let pretty = crate::widgets::pretty_name(n);
            let label = if pretty == *n { n.clone() } else { format!("{pretty}   ({n})") };
            ui.selectable_value(value, n.clone(), label);
        }
    });
    *value != before
}

fn chart_fields(ui: &mut Ui, t: &mut Tile, names: &[String], pal: &Pal) {
    field(ui, "Time window", |ui| {
        let opts = [(60, "1 minute"), (300, "5 minutes"), (900, "15 minutes"), (3600, "1 hour"), (21600, "6 hours"), (86400, "24 hours")];
        let cur = if t.window_secs == 0 { 300 } else { t.window_secs };
        let shown = opts.iter().find(|o| o.0 == cur).map(|o| o.1.to_string()).unwrap_or_else(|| format!("{cur} s"));
        egui::ComboBox::from_id_salt("pb_ed_window").selected_text(shown).width(130.0).show_ui(ui, |ui| {
            for (secs, name) in opts {
                if ui.selectable_label(cur == secs, name).clicked() {
                    t.window_secs = secs;
                }
            }
        });
    });
    field(ui, "Left axis", |ui| {
        let mut fixed = t.min.is_some() && t.max.is_some();
        if ui.checkbox(&mut fixed, "fixed range").changed() {
            if fixed {
                t.min = Some(0.0);
                t.max = Some(100.0);
            } else {
                t.min = None;
                t.max = None;
            }
        }
        if let (Some(lo), Some(hi)) = (&mut t.min, &mut t.max) {
            ui.add(egui::DragValue::new(lo).speed(1.0));
            ui.label("to");
            ui.add(egui::DragValue::new(hi).speed(1.0));
        }
    });
    ui.add_space(2.0);
    ui.label(RichText::new(format!("SERIES  ·  {}", t.series.len())).font(crate::theme::bold_id(12.5)).color(pal.muted));
    let mut remove = None;
    for i in 0..t.series.len() {
        let s = &mut t.series[i];
        ui.group(|ui| {
            ui.horizontal(|ui| {
                point_combo(ui, &format!("pb_ed_series_{i}"), &mut s.point, names);
                if ui.small_button("×").on_hover_text("Remove this series").clicked() {
                    remove = Some(i);
                }
            });
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut s.label).desired_width(110.0).hint_text("label"));
                let mut c = series_color(s, i);
                if ui.color_edit_button_srgba(&mut c).changed() {
                    s.color = hex(c);
                }
                let mut right = s.axis == 1;
                if ui.checkbox(&mut right, "right axis").changed() {
                    s.axis = right as u8;
                }
            });
            ui.horizontal(|ui| {
                for (key, name) in [("", "line"), ("step", "step"), ("area", "area")] {
                    if ui.selectable_label(s.style == key, name).clicked() {
                        s.style = key.to_string();
                    }
                }
            });
        });
    }
    if let Some(i) = remove {
        t.series.remove(i);
    }
    if t.series.len() < 8 && ui.button("+ Add point").clicked() {
        t.series.push(Series { point: String::new(), label: String::new(), color: String::new(), axis: 0, style: String::new() });
    }
}
