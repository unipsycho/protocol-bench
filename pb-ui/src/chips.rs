//! Shared selectable chips with an in-chip delete control (PB-92).
//!
//! Label click selects; × inside the same chip requests delete (caller confirms).

use egui::{Align2, Color32, Pos2, Rect, Rounding, Sense, Stroke, Vec2};

use crate::theme::{font_ui, space, ThemePalette};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChipAction {
    Select,
    Delete,
}

/// One rounded chip: `[ label ]` or `[ label  × ]` when `show_delete`.
pub fn selectable_chip(
    ui: &mut egui::Ui,
    label: &str,
    selected: bool,
    show_delete: bool,
    p: &ThemePalette,
) -> Option<ChipAction> {
    let pad_x = 10.0;
    let close_slot = 20.0;
    let gap = if show_delete { 6.0 } else { 0.0 };
    let fill = if selected { p.accent } else { p.control };
    let text_color = if selected {
        Color32::WHITE
    } else {
        p.text
    };
    let close_color = if selected {
        Color32::from_rgba_unmultiplied(255, 255, 255, 200)
    } else {
        p.muted
    };

    let galley = ui.fonts(|f| f.layout_no_wrap(label.to_string(), font_ui(12.5), text_color));
    let h = 26.0_f32;
    let label_w = galley.size().x;
    let w = (pad_x + label_w + gap + if show_delete { close_slot } else { 0.0 } + pad_x).max(56.0);

    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
    ui.painter()
        .rect_filled(rect, Rounding::same(5.0), fill);
    ui.painter()
        .rect_stroke(rect, Rounding::same(5.0), Stroke::new(1.0, p.line));

    let label_right = if show_delete {
        rect.max.x - pad_x - close_slot - gap * 0.5
    } else {
        rect.max.x - pad_x
    };
    let label_rect = Rect::from_min_max(
        Pos2::new(rect.min.x + pad_x, rect.min.y),
        Pos2::new(label_right, rect.max.y),
    );
    ui.painter().galley(
        Pos2::new(
            label_rect.min.x,
            label_rect.center().y - galley.size().y * 0.5,
        ),
        galley,
        text_color,
    );

    let label_id = ui.id().with(("pb_chip_label", label));
    let label_resp = ui.interact(label_rect, label_id, Sense::click());
    if label_resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    let mut action = None;
    if label_resp.clicked() {
        action = Some(ChipAction::Select);
    }

    if show_delete {
        let close_rect = Rect::from_center_size(
            Pos2::new(rect.max.x - pad_x - close_slot * 0.5, rect.center().y),
            Vec2::splat(close_slot),
        );
        let close_id = ui.id().with(("pb_chip_x", label));
        let close_resp = ui
            .interact(close_rect, close_id, Sense::click())
            .on_hover_text(format!("Delete {label}"));
        if close_resp.hovered() {
            ui.painter().rect_filled(
                close_rect.shrink(2.0),
                Rounding::same(3.0),
                if selected {
                    Color32::from_rgba_unmultiplied(0, 0, 0, 40)
                } else {
                    crate::theme::tint(p.accent, 20)
                },
            );
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        ui.painter().text(
            close_rect.center(),
            Align2::CENTER_CENTER,
            "×",
            font_ui(14.0),
            close_color,
        );
        if close_resp.clicked() {
            action = Some(ChipAction::Delete);
        }
    }

    action
}

/// Modal confirm before removing a chip. Returns the pending index when Delete is chosen.
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
        .id(egui::Id::new(("pb_chip_confirm", idx, &name)))
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
