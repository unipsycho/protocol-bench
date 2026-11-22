//! Wrapping form rows for the inspector and panels.
//!
//! A [`flow`] lays labeled cells (`Label [widget]`) left to right and wraps to a new row when
//! the next cell would not fit, so a wide inspector uses its width and a narrow one grows
//! taller instead of clipping. Rows are planned up front: cells that may grow share the
//! leftover width, so each row fills the available space; fixed cells (drag values, combos)
//! keep their size.
//!
//! Cells take `&mut S` as a parameter instead of capturing it, so several cells can edit
//! the same struct without fighting the borrow checker:
//!
//! ```ignore
//! let changed = form::flow(ui, &mut draft, p, vec![
//!     Cell::fixed("Address", 70.0, |ui, w, d: &mut Bind| ui.add_sized([w, H], DragValue::new(&mut d.address)).changed()),
//!     Cell::grow("Point", 120.0, |ui, w, d| ui.add(TextEdit::singleline(&mut d.point).desired_width(w)).changed()),
//! ]);
//! ```

use egui::{vec2, Align, Layout, Ui};

use crate::theme::{font_ui, space, ThemePalette};

/// Height of one form row.
pub const ROW_H: f32 = 24.0;
/// Gap between the label and its widget.
const LABEL_GAP: f32 = 6.0;

type CellFn<'a, S> = Box<dyn FnOnce(&mut Ui, f32, &mut S) -> bool + 'a>;

/// One labeled cell: `label` then a widget drawn by `add(ui, widget_width, state)`.
/// `add` returns true when the value changed.
pub struct Cell<'a, S> {
    label: &'a str,
    min: f32,
    max: f32,
    add: CellFn<'a, S>,
}

impl<'a, S> Cell<'a, S> {
    /// Widget keeps exactly `width` (drag values, combos, checkboxes).
    pub fn fixed(
        label: &'a str,
        width: f32,
        add: impl FnOnce(&mut Ui, f32, &mut S) -> bool + 'a,
    ) -> Self {
        Self { label, min: width, max: width, add: Box::new(add) }
    }

    /// Widget is at least `min` wide and takes spare room on its row (text fields).
    pub fn grow(
        label: &'a str,
        min: f32,
        add: impl FnOnce(&mut Ui, f32, &mut S) -> bool + 'a,
    ) -> Self {
        Self { label, min, max: f32::INFINITY, add: Box::new(add) }
    }

    /// Like [`Cell::grow`] but never wider than `max` (combos that read badly when huge).
    pub fn grow_to(
        label: &'a str,
        min: f32,
        max: f32,
        add: impl FnOnce(&mut Ui, f32, &mut S) -> bool + 'a,
    ) -> Self {
        Self { label, min, max, add: Box::new(add) }
    }
}

/// Planned geometry for one cell: its total width and the widget's share of it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Slot {
    pub cell: f32,
    pub widget: f32,
}

/// Greedy wrap + fill. `cells` are `(label_width, min, max)`; returns one `Vec<Slot>` per row.
/// Pure so the wrapping rules are unit-tested without a UI.
pub fn plan_rows(cells: &[(f32, f32, f32)], avail: f32, gap: f32) -> Vec<Vec<Slot>> {
    let total = |c: &(f32, f32, f32)| if c.0 > 0.0 { c.0 + LABEL_GAP + c.1 } else { c.1 };
    let mut rows: Vec<Vec<usize>> = Vec::new();
    let mut used = 0.0_f32;
    for (i, c) in cells.iter().enumerate() {
        let w = total(c);
        match rows.last_mut() {
            Some(row) if used + gap + w <= avail => {
                row.push(i);
                used += gap + w;
            }
            _ => {
                rows.push(vec![i]);
                used = w;
            }
        }
    }
    rows.into_iter()
        .map(|row| {
            let base: f32 =
                row.iter().map(|&i| total(&cells[i])).sum::<f32>() + gap * (row.len() - 1) as f32;
            let mut extra = (avail - base).max(0.0);
            let mut widget: Vec<f32> = row.iter().map(|&i| cells[i].1).collect();
            // Hand spare width to growable cells, respecting each cell's max.
            for _ in 0..row.len() {
                let growable: Vec<usize> = (0..row.len())
                    .filter(|&k| widget[k] + 0.01 < cells[row[k]].2)
                    .collect();
                if growable.is_empty() || extra <= 0.01 {
                    break;
                }
                let share = extra / growable.len() as f32;
                for k in growable {
                    let room = cells[row[k]].2 - widget[k];
                    let add = share.min(room);
                    widget[k] += add;
                    extra -= add;
                }
            }
            row.iter()
                .enumerate()
                .map(|(k, &i)| {
                    let lw = cells[i].0;
                    let cell = if lw > 0.0 { lw + LABEL_GAP + widget[k] } else { widget[k] };
                    Slot { cell, widget: widget[k] }
                })
                .collect()
        })
        .collect()
}

/// Lay out `cells` as wrapping rows. Returns true when any cell reported a change.
pub fn flow<S>(ui: &mut Ui, state: &mut S, p: &ThemePalette, cells: Vec<Cell<'_, S>>) -> bool {
    let gap = space::ITEM_X;
    let label_w: Vec<f32> = cells
        .iter()
        .map(|c| {
            if c.label.is_empty() {
                0.0
            } else {
                ui.fonts(|f| f.layout_no_wrap(c.label.to_owned(), font_ui(12.0), p.muted))
                    .size()
                    .x
            }
        })
        .collect();
    let shape: Vec<(f32, f32, f32)> = cells
        .iter()
        .zip(&label_w)
        .map(|(c, &lw)| (lw, c.min, c.max))
        .collect();
    // Keep a small right margin so focus rings are not clipped by the scroll area.
    let rows = plan_rows(&shape, (ui.available_width() - 2.0).max(60.0), gap);
    let mut changed = false;
    let mut it = cells.into_iter().zip(label_w);
    for (r, row) in rows.iter().enumerate() {
        if r > 0 {
            ui.add_space(space::ITEM_Y - 2.0);
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            ui.set_min_height(ROW_H);
            for slot in row {
                let (cell, lw) = it.next().expect("plan matches cells");
                ui.allocate_ui_with_layout(
                    vec2(slot.cell, ROW_H),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        ui.set_width(slot.cell);
                        ui.spacing_mut().item_spacing.x = LABEL_GAP;
                        if lw > 0.0 {
                            ui.add(egui::Label::new(
                                egui::RichText::new(cell.label)
                                    .font(font_ui(12.0))
                                    .color(p.muted),
                            ));
                        }
                        if (cell.add)(ui, slot.widget, state) {
                            changed = true;
                        }
                    },
                );
            }
        });
    }
    changed
}

/// Fixed-width numeric drag value, as used in [`Cell`]s.
pub fn drag_f32(ui: &mut Ui, w: f32, v: &mut f32, speed: f64, range: std::ops::RangeInclusive<f32>) -> bool {
    ui.add_sized(
        [w, ROW_H - 2.0],
        egui::DragValue::new(v).speed(speed).range(range),
    )
    .changed()
}

/// Full-cell-width single-line text edit.
pub fn text(ui: &mut Ui, w: f32, v: &mut String) -> bool {
    ui.add_sized([w, ROW_H - 2.0], egui::TextEdit::singleline(v).desired_width(w))
        .changed()
}

/// Combo over `(key, label)` options that edits `value`; width `w`.
pub fn combo(ui: &mut Ui, salt: impl std::hash::Hash, w: f32, value: &mut String, options: &[(&str, &str)]) -> bool {
    let shown = options
        .iter()
        .find(|(k, _)| *k == value.as_str())
        .map(|(_, l)| (*l).to_string())
        .unwrap_or_else(|| value.clone());
    let mut next = value.clone();
    egui::ComboBox::from_id_salt(salt)
        .selected_text(shown)
        .width(w)
        .show_ui(ui, |ui| {
            for (k, l) in options {
                ui.selectable_value(&mut next, (*k).to_string(), *l);
            }
        });
    let changed = next != *value;
    *value = next;
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_wrap_when_they_do_not_fit() {
        // three cells, 100 wide each (no label), gap 10: two fit in 230, the third wraps
        let rows = plan_rows(&[(0.0, 100.0, 100.0); 3], 230.0, 10.0);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].len(), 2);
        assert_eq!(rows[1].len(), 1);
    }

    #[test]
    fn growable_cells_fill_the_row_and_fixed_ones_do_not() {
        let rows = plan_rows(&[(0.0, 50.0, 50.0), (0.0, 80.0, f32::INFINITY)], 300.0, 10.0);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][0].widget, 50.0);
        assert!((rows[0][1].widget - 240.0).abs() < 0.1, "{:?}", rows[0][1]);
    }

    #[test]
    fn max_caps_growth_and_spare_goes_to_the_other_cell() {
        let rows = plan_rows(&[(0.0, 60.0, 100.0), (0.0, 60.0, f32::INFINITY)], 400.0, 10.0);
        assert_eq!(rows[0][0].widget, 100.0);
        assert!((rows[0][1].widget - 290.0).abs() < 0.1, "{:?}", rows[0][1]);
    }

    #[test]
    fn a_cell_wider_than_the_row_gets_its_own_row() {
        let rows = plan_rows(&[(0.0, 500.0, 500.0), (0.0, 50.0, 50.0)], 200.0, 10.0);
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn rows_never_exceed_the_available_width_when_cells_fit() {
        let cells = [(30.0, 60.0, 60.0), (40.0, 70.0, f32::INFINITY), (20.0, 90.0, 90.0), (50.0, 80.0, 120.0)];
        for avail in [180.0, 260.0, 400.0, 900.0] {
            for row in plan_rows(&cells, avail, 10.0) {
                let w: f32 = row.iter().map(|s| s.cell).sum::<f32>() + 10.0 * (row.len() - 1) as f32;
                if row.len() > 1 {
                    assert!(w <= avail + 0.1, "avail {avail}: row {w}");
                }
            }
        }
    }
}
