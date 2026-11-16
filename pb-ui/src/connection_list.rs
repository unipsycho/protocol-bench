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
