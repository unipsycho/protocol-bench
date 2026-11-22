//! Selectable connection list rows with in-row delete + confirm (MQTT / OPC UA).

use egui::{Align2, Color32, Layout, Pos2, Rect, Rounding, Sense, Stroke, Vec2};

use crate::theme::{font_semi, font_ui, space, tint, ThemePalette};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowAction {
    Select,
    Delete,
}

/// Section title with a right-justified Add control on the same row (PB-105).
pub fn heading_with_add(
    ui: &mut egui::Ui,
    title: &str,
    add_label: &str,
    title_color: Color32,
    p: &ThemePalette,
) -> bool {
    let mut add = false;
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(title)
                .font(font_semi(15.0))
                .color(title_color),
        );
        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            let resp = ui.add(
                egui::Button::new(
                    egui::RichText::new(add_label)
                        .font(font_ui(12.5))
                        .color(Color32::from_rgb(0x12, 0x14, 0x18)),
                )
                .fill(p.live)
                .stroke(Stroke::new(1.0, p.line))
                .min_size(Vec2::new(72.0, 26.0)),
            );
            if resp.clicked() {
                add = true;
            }
        });
    });
    add
}

/// One full-width row: identity on the left, × on the same row when `show_delete`.
pub fn selectable_row(
    ui: &mut egui::Ui,
    label: &str,
    subtitle: &str,
    selected: bool,
    show_delete: bool,
    p: &ThemePalette,
) -> Option<RowAction> {
    let row_h = if subtitle.is_empty() { 30.0 } else { 40.0 };
    let close_slot = 28.0;
    let pad_x = 10.0;
    let avail_w = ui.available_width().max(120.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(avail_w, row_h), Sense::hover());
    let hovered = ui.rect_contains_pointer(rect);

    // Idle rows use solid control fill so they read against the pane (PB-105).
    let (fill, stroke) = if selected {
        (
            tint(p.accent, if p.is_light { 56 } else { 96 }),
            Stroke::new(1.5, p.accent),
        )
    } else if hovered {
        (
            tint(p.control, if p.is_light { 220 } else { 255 }),
            Stroke::new(1.0, tint(p.accent, 140)),
        )
    } else {
        (p.control, Stroke::new(1.0, p.line))
    };
    ui.painter()
        .rect_filled(rect, Rounding::same(4.0), fill);
    ui.painter()
        .rect_stroke(rect, Rounding::same(4.0), stroke);

    let text_right = if show_delete {
        rect.max.x - pad_x - close_slot
    } else {
        rect.max.x - pad_x
    };
    let text_rect = Rect::from_min_max(
        Pos2::new(rect.min.x + pad_x, rect.min.y),
        Pos2::new(text_right, rect.max.y),
    );

    let title = ui.fonts(|f| f.layout_no_wrap(label.to_owned(), font_ui(13.0), p.text));
    if subtitle.is_empty() {
        ui.painter().galley(
            Pos2::new(
                text_rect.min.x,
                text_rect.center().y - title.size().y * 0.5,
            ),
            title,
            p.text,
        );
    } else {
        let sub = ui.fonts(|f| f.layout_no_wrap(subtitle.to_owned(), font_ui(11.0), p.muted));
        let title_h = title.size().y;
        let block_h = title_h + 2.0 + sub.size().y;
        let top = text_rect.center().y - block_h * 0.5;
        ui.painter()
            .galley(Pos2::new(text_rect.min.x, top), title, p.text);
        ui.painter().galley(
            Pos2::new(text_rect.min.x, top + title_h + 2.0),
            sub,
            p.muted,
        );
    }

    let label_id = ui.id().with(("pb_conn_row", label, subtitle));
    let label_resp = ui.interact(text_rect, label_id, Sense::click());
    if label_resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    let mut action = None;
    if label_resp.clicked() {
        action = Some(RowAction::Select);
    }

    if show_delete {
        let close_rect = Rect::from_center_size(
            Pos2::new(rect.max.x - pad_x - close_slot * 0.5, rect.center().y),
            Vec2::splat(close_slot),
        );
        let close_id = ui.id().with(("pb_conn_x", label, subtitle));
        let close_resp = ui
            .interact(close_rect, close_id, Sense::click())
            .on_hover_text(format!("Delete {label}"));
        if close_resp.hovered() {
            ui.painter().rect_filled(
                close_rect.shrink(3.0),
                Rounding::same(3.0),
                tint(p.err, if p.is_light { 36 } else { 48 }),
            );
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        ui.painter().text(
            close_rect.center(),
            Align2::CENTER_CENTER,
            "×",
            font_ui(15.0),
            if close_resp.hovered() { p.err } else { p.muted },
        );
        if close_resp.clicked() {
            action = Some(RowAction::Delete);
        }
    }

    action
}

/// Modal confirm before removing a connection. Returns index when Delete is chosen.
pub fn confirm_delete(
    ctx: &egui::Context,
    pending: &mut Option<(usize, String)>,
    p: &ThemePalette,
) -> Option<usize> {
    let Some((idx, name)) = pending.clone() else {
        return None;
    };
    let mut confirmed = None;
    let mut open = true;
    egui::Window::new("Confirm delete")
        .id(egui::Id::new(("pb_conn_confirm", idx, &name)))
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .order(egui::Order::Foreground)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.set_min_width(280.0);
            ui.label(
                egui::RichText::new(format!("Delete {name}?"))
                    .font(font_ui(14.0))
                    .color(p.text),
            );
            ui.add_space(space::AFTER_BLURB);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = space::BUTTON;
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("Cancel")
                                .font(font_ui(12.5))
                                .color(p.text),
                        )
                        .fill(p.control)
                        .stroke(Stroke::new(1.0, p.line)),
                    )
                    .clicked()
                {
                    *pending = None;
                }
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("Delete")
                                .font(font_ui(12.5))
                                .color(Color32::WHITE),
                        )
                        .fill(Color32::from_rgb(180, 70, 60))
                        .stroke(Stroke::new(1.0, p.line)),
                    )
                    .clicked()
                {
                    confirmed = Some(idx);
                    *pending = None;
                }
            });
        });
    if !open {
        *pending = None;
    }
    confirmed
}

/// Traffic-light for a status line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dot {
    Ok,
    Warn,
    Err,
    Idle,
}

/// Coloured dot + state text, and the last error underneath.
pub fn status_line(ui: &mut egui::Ui, p: &ThemePalette, dot: Dot, text: &str, error: &str) {
    let color = match dot {
        Dot::Ok => p.live,
        Dot::Warn => p.warn,
        Dot::Err => p.err,
        Dot::Idle => p.muted,
    };
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = space::ITEM_X;
        ui.label(egui::RichText::new("●").font(font_ui(12.0)).color(color));
        ui.label(egui::RichText::new(text).font(font_ui(12.0)).color(p.text));
    });
    if !error.is_empty() {
        ui.label(egui::RichText::new(error).font(font_ui(11.5)).color(p.warn));
    }
}

/// Clickable `(name, sample)` chips for column / curve pickers. Names for which `taken` is true
/// are shown but disabled. Returns the name that was clicked.
pub fn picker_chips(ui: &mut egui::Ui, p: &ThemePalette, items: &[(String, String)], taken: &dyn Fn(&str) -> bool) -> Option<String> {
    let mut clicked = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 4.0);
        for (name, sample) in items {
            let used = taken(name);
            let label = if sample.is_empty() { name.clone() } else { format!("{name}  {}", sample.chars().take(14).collect::<String>()) };
            let resp = ui.add_enabled(
                !used,
                egui::Button::new(egui::RichText::new(label).font(font_ui(11.5)).color(if used { p.muted } else { p.text }))
                    .fill(p.control)
                    .stroke(egui::Stroke::new(1.0, p.line)),
            );
            let tip = if sample.is_empty() { name.to_string() } else { format!("{name} = {sample}") };
            let resp = resp.on_hover_text(if used { format!("{tip}  (already bound)") } else { format!("{tip}  — click to bind") });
            if resp.clicked() {
                clicked = Some(name.clone());
            }
        }
    });
    clicked
}

/// A free id for a new connection: `base`, else `base-2`, `base-3`, … — the first one `taken` rejects.
/// Every connection panel (HTTP, Streams, Modbus, SNMP, LAS, Files, …) numbers new rows this way.
pub fn unique_id(base: &str, taken: impl Fn(&str) -> bool) -> String {
    let mut id = base.to_string();
    let mut n = 2;
    while taken(&id) {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

/// Small form button; `selected` paints it in the accent colour (toggle look).
pub fn text_btn(ui: &mut egui::Ui, label: &str, selected: bool, p: &ThemePalette) -> egui::Response {
    let fill = if selected { p.accent } else { p.control };
    let text = if selected { Color32::from_rgb(0x12, 0x14, 0x18) } else { p.text };
    ui.add(
        egui::Button::new(egui::RichText::new(label).font(font_ui(12.0)).color(text))
            .fill(fill)
            .stroke(Stroke::new(1.0, p.line)),
    )
}

/// `label  [single-line edit]` form row; returns true when the text changed. `label_w` is the
/// label column width so a panel's rows line up.
pub fn labeled_field(ui: &mut egui::Ui, label_w: f32, label: &str, value: &mut String, p: &ThemePalette) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::ITEM_X;
        ui.add_sized([label_w, 20.0], egui::Label::new(egui::RichText::new(label).font(font_ui(12.0)).color(p.muted)));
        let w = (ui.available_width() - 8.0).clamp(140.0, 420.0);
        if ui
            .add(egui::TextEdit::singleline(value).desired_width(w).margin(egui::Margin::symmetric(6.0, 3.0)))
            .changed()
        {
            changed = true;
        }
    });
    ui.add_space(2.0);
    changed
}
