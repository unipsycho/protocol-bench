//! Calculations panel — operator-facing catalog for edge transforms
//! that sit between Input/node path and Output.
//!
//! Engine evaluation can be stubbed; UI + project fields are production-shaped.

use egui::{Align, Color32, Layout, RichText, Sense, Vec2};

use crate::hubs::HubModel;
use crate::theme::{font_semi, font_ui, ThemePalette};

/// One catalog entry operators can place on a hub edge.
#[derive(Debug, Clone, Copy)]
pub struct CalcKind {
    pub id: &'static str,
    pub name: &'static str,
    pub blurb: &'static str,
    pub example: &'static str,
    pub time_based: bool,
}

/// Built-in calculation types (panel catalog).
pub const CALC_KINDS: &[CalcKind] = &[
    CalcKind {
        id: "scale_offset",
        name: "Scale / offset",
        blurb: "y = (x × scale) + offset. Use for raw counts → engineering units.",
        example: "scale = 0.1 · offset = −40",
        time_based: false,
    },
    CalcKind {
        id: "unit_convert",
        name: "Unit convert",
        blurb: "Map one unit system to another (e.g. °C ↔ °F, psi ↔ kPa).",
        example: "from = °C · to = °F",
        time_based: false,
    },
    CalcKind {
        id: "clamp",
        name: "Clamp",
        blurb: "Hold the value inside a hard min/max band.",
        example: "min = 0 · max = 100",
        time_based: false,
    },
    CalcKind {
        id: "deadband",
        name: "Deadband",
        blurb: "Ignore small chatter; only pass changes larger than the band.",
        example: "band = ±0.5",
        time_based: false,
    },
    CalcKind {
        id: "rolling_avg",
        name: "Rolling average",
        blurb: "Mean over the last N samples or a time window.",
        example: "window = 10 s · samples = auto",
        time_based: true,
    },
    CalcKind {
        id: "ema",
        name: "EMA",
        blurb: "Exponential moving average — faster response than a plain roll.",
        example: "alpha = 0.2 (or tau = 5 s)",
        time_based: true,
    },
    CalcKind {
        id: "rate_of_change",
        name: "Rate of change",
        blurb: "Δvalue / Δtime. Spot spikes or ramp faults.",
        example: "units = /s · abs = optional",
        time_based: true,
    },
    CalcKind {
        id: "min_max_hold",
        name: "Min / max hold",
        blurb: "Remember peak or valley until reset or window expiry.",
        example: "mode = max · hold = 30 s",
        time_based: true,
    },
    CalcKind {
        id: "hysteresis",
        name: "Hysteresis",
        blurb: "Two thresholds so digital outs do not chatter at the edge.",
        example: "on ≥ 80 · off ≤ 70",
        time_based: false,
    },
    CalcKind {
        id: "boolean_compare",
        name: "Boolean compare",
        blurb: "Compare to a setpoint or another point → true/false.",
        example: "x > 50 · or · x ≥ ref_point",
        time_based: false,
    },
    CalcKind {
        id: "formula",
        name: "Expression / formula",
        blurb: "Free expression over one or more points (operators + functions).",
        example: "(A * 1.8) + 32 · or · clamp(A / B, 0, 1)",
        time_based: false,
    },
    CalcKind {
        id: "time_window",
        name: "Time-window filter",
        blurb: "Pass, block, or average only inside a sliding time window.",
        example: "window = 60 s · mode = avg | gate | debounce",
        time_based: true,
    },
];

#[derive(Debug, Clone)]
pub struct CalcPanelState {
    pub selected: Option<&'static str>,
    pub formula_draft: String,
    pub window_secs: String,
    pub param_a: String,
    pub param_b: String,
}

impl Default for CalcPanelState {
    fn default() -> Self {
        Self {
            selected: Some("scale_offset"),
            formula_draft: "(x * scale) + offset".into(),
            window_secs: "10".into(),
            param_a: "1.0".into(),
            param_b: "0.0".into(),
        }
    }
}

fn kind_by_id(id: &str) -> Option<&'static CalcKind> {
    CALC_KINDS.iter().find(|k| k.id == id)
}

/// Human label for a stored calc kind id.
pub fn kind_label(id: &str) -> &str {
    kind_by_id(id).map(|k| k.name).unwrap_or("Calculation")
}

/// Apply the panel draft to the currently selected calculation node.
pub fn apply_draft_to_selection(hubs: &mut HubModel, state: &CalcPanelState) -> bool {
    let Some(hub_id) = hubs.selected else {
        return false;
    };
    let Some(step_id) = hubs.selected_port else {
        return false;
    };
    let Some(hub) = hubs.hub_mut(hub_id) else {
        return false;
    };
    let Some(step) = hub.calculations.iter_mut().find(|s| s.id == step_id) else {
        return false;
    };
    let kind = state.selected.unwrap_or("scale_offset");
    step.calc_kind = kind.into();
    step.formula = state.formula_draft.clone();
    step.param_a = state.param_a.clone();
    step.param_b = state.param_b.clone();
    step.window_secs = state.window_secs.clone();
    if let Some(k) = kind_by_id(kind) {
        if step.label == "Calculation" || step.label.is_empty() {
            step.label = k.name.into();
        }
    }
    true
}

/// Paint the Calculations dock body. Returns true when hubs were mutated.
pub fn paint_panel(
    ui: &mut egui::Ui,
    hubs: &mut HubModel,
    state: &mut CalcPanelState,
    p: &ThemePalette,
) -> bool {
    let mut dirty = false;
    egui::ScrollArea::vertical()
        .id_salt("pb_calc_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            heading(ui, "Calculations", p.accent, p);
            ui.add_space(8.0);
            ui.label(
                RichText::new(
                    "Calculations sit on the path between an Input (or hub) and an Output. \
They reshape values on that edge — not as free-floating peers of the hub.",
                )
                .font(font_ui(13.0))
                .color(p.text),
            );
            ui.add_space(10.0);
            ui.label(
                RichText::new(
                    "How to use: open Hubs → select a connection → Add calculation → return here to pick a type and parameters.",
                )
                .font(font_ui(12.0))
                .color(p.muted),
            );

            ui.add_space(20.0);
            heading(ui, "Path framing", p.live, p);
            ui.add_space(10.0);
            path_diagram(ui, p);
            ui.add_space(12.0);
            ui.label(
                RichText::new(format!(
                    "This project has {} calculation node(s) on hub edges.",
                    hubs.hubs.iter().map(|h| h.calculations.len()).sum::<usize>()
                ))
                .font(font_ui(12.0))
                .color(p.muted),
            );

            ui.add_space(22.0);
            heading(ui, "Calculation types", p.accent, p);
            ui.add_space(8.0);
            ui.label(
                RichText::new(
                    "Twelve operator-ready blocks. Types marked time use a window or time constant.",
                )
                .font(font_ui(12.0))
                .color(p.muted),
            );
            ui.add_space(14.0);

            let mut pick = None;
            for kind in CALC_KINDS {
                let active = state.selected == Some(kind.id);
                if catalog_card(ui, kind, active, p) {
                    pick = Some(kind.id);
                }
                ui.add_space(10.0);
            }
            if let Some(id) = pick {
                state.selected = Some(id);
                if id == "formula" && state.formula_draft.is_empty() {
                    state.formula_draft = "(x * scale) + offset".into();
                }
            }

            ui.add_space(10.0);
            if let Some(kind) = kind_by_id(state.selected.unwrap_or("scale_offset")) {
                detail_editor(ui, kind, state, p);
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let can_apply = hubs.selected.zip(hubs.selected_port).is_some_and(|(h, s)| {
                    hubs.hub(h)
                        .is_some_and(|hub| hub.calculations.iter().any(|c| c.id == s))
                });
                let label = if can_apply {
                    "Apply to selected calculation"
                } else {
                    "Select a calculation on Hubs first"
                };
                if ui
                    .add_enabled(can_apply, egui::Button::new(label))
                    .clicked()
                    && apply_draft_to_selection(hubs, state)
                {
                    dirty = true;
                }
            });

            ui.add_space(22.0);
            heading(ui, "On this project", p.live, p);
            ui.add_space(10.0);
            existing_calcs(ui, hubs, p);
            ui.add_space(18.0);
        });
    dirty
}

fn heading(ui: &mut egui::Ui, title: &str, color: Color32, _p: &ThemePalette) {
    ui.label(RichText::new(title).font(font_semi(15.0)).color(color));
}

fn path_diagram(ui: &mut egui::Ui, p: &ThemePalette) {
    let full = ui.available_width().max(280.0);
    let h = 56.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(full, h), Sense::hover());
    let painter = ui.painter();
    let y = rect.center().y;
    let pad = 12.0;
    let box_w = ((full - pad * 4.0) / 3.0).clamp(72.0, 140.0);
    let boxes = [
        ("Input / node", rect.min.x + pad),
        ("Calculation", rect.min.x + pad * 2.0 + box_w),
        ("Output", rect.min.x + pad * 3.0 + box_w * 2.0),
    ];
    for (i, (label, x)) in boxes.iter().enumerate() {
        let r = egui::Rect::from_min_size(egui::pos2(*x, y - 18.0), Vec2::new(box_w, 36.0));
        let fill = if i == 1 {
            Color32::from_rgba_unmultiplied(p.accent.r(), p.accent.g(), p.accent.b(), 40)
        } else {
            p.control
        };
        painter.rect_filled(r, 6.0, fill);
        painter.rect_stroke(r, 6.0, egui::Stroke::new(1.0, p.line));
        painter.text(
            r.center(),
            egui::Align2::CENTER_CENTER,
            *label,
            font_ui(12.0),
            p.text,
        );
        if i < 2 {
            let x0 = r.max.x + 4.0;
            let x1 = boxes[i + 1].1 - 4.0;
            painter.line_segment(
                [egui::pos2(x0, y), egui::pos2(x1, y)],
                egui::Stroke::new(1.5, p.muted),
            );
        }
    }
}

fn catalog_card(ui: &mut egui::Ui, kind: &CalcKind, active: bool, p: &ThemePalette) -> bool {
    let fill = if active {
        Color32::from_rgba_unmultiplied(p.accent.r(), p.accent.g(), p.accent.b(), 28)
    } else {
        p.control
    };
    let stroke = if active {
        egui::Stroke::new(1.4, p.accent)
    } else {
        egui::Stroke::new(1.0, p.line)
    };
    let resp = egui::Frame::none()
        .fill(fill)
        .stroke(stroke)
        .rounding(egui::Rounding::same(8.0))
        .inner_margin(egui::Margin::symmetric(12.0, 10.0))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(kind.name)
                        .font(font_semi(13.5))
                        .color(if active { p.accent } else { p.text }),
                );
                if kind.time_based {
                    ui.add_space(8.0);
                    ui.label(RichText::new("time").font(font_ui(10.5)).color(p.live));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if active {
                        ui.label(RichText::new("selected").font(font_ui(11.0)).color(p.muted));
                    }
                });
            });
            ui.add_space(4.0);
            ui.label(RichText::new(kind.blurb).font(font_ui(12.0)).color(p.muted));
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!("Example · {}", kind.example))
                    .font(font_ui(11.5))
                    .color(p.text),
            );
        });
    // Whole card is the hit target.
    ui.interact(resp.response.rect, ui.id().with(("calc_card", kind.id)), Sense::click())
        .clicked()
}

fn detail_editor(ui: &mut egui::Ui, kind: &CalcKind, state: &mut CalcPanelState, p: &ThemePalette) {
    ui.add_space(8.0);
    heading(ui, "Configure selection", p.accent, p);
    ui.add_space(8.0);
    ui.label(
        RichText::new(format!("{} — {}", kind.name, kind.blurb))
            .font(font_ui(12.5))
            .color(p.text),
    );
    ui.add_space(10.0);

    match kind.id {
        "formula" => {
            ui.label(RichText::new("FORMULA").font(font_ui(10.5)).color(p.muted));
            ui.add_space(4.0);
            ui.add(
                egui::TextEdit::multiline(&mut state.formula_draft)
                    .desired_width(f32::INFINITY)
                    .desired_rows(3),
            );
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "Use point ids or x for the edge value. Functions: min, max, abs, clamp, sqrt.",
                )
                .font(font_ui(11.5))
                .color(p.muted),
            );
        }
        "rolling_avg" | "ema" | "rate_of_change" | "min_max_hold" | "time_window" => {
            field_row(ui, "Window (s)", &mut state.window_secs, p);
            ui.add_space(4.0);
            field_row(ui, "Param A", &mut state.param_a, p);
            ui.add_space(4.0);
            field_row(ui, "Param B", &mut state.param_b, p);
            ui.add_space(4.0);
            ui.label(
                RichText::new("Time-based filters use wall clock on the host edge runner (stub).")
                    .font(font_ui(11.5))
                    .color(p.muted),
            );
        }
        _ => {
            field_row(ui, "Param A", &mut state.param_a, p);
            ui.add_space(4.0);
            field_row(ui, "Param B", &mut state.param_b, p);
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!("Hint · {}", kind.example))
                    .font(font_ui(11.5))
                    .color(p.muted),
            );
        }
    }
    ui.add_space(10.0);
    ui.label(
        RichText::new(
            "These defaults apply when you next add a calculation on a hub edge. Engine evaluation follows.",
        )
        .font(font_ui(11.5))
        .color(p.muted),
    );
}

fn field_row(ui: &mut egui::Ui, label: &str, value: &mut String, p: &ThemePalette) {
    ui.horizontal(|ui| {
        ui.add_sized(
            [100.0, 20.0],
            egui::Label::new(RichText::new(label).font(font_ui(12.0)).color(p.muted)),
        );
        ui.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY));
    });
}

fn existing_calcs(ui: &mut egui::Ui, hubs: &HubModel, p: &ThemePalette) {
    let mut any = false;
    for hub in &hubs.hubs {
        for step in &hub.calculations {
            any = true;
            let edge = hub
                .sources
                .iter()
                .chain(hub.outputs.iter())
                .find(|port| port.id == step.edge_port)
                .map(|port| port.label.as_str())
                .unwrap_or("?");
            let kind_label = kind_label(step.calc_kind.as_str());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("{} · {}", step.label, kind_label))
                        .font(font_ui(12.5))
                        .color(p.text),
                );
                ui.label(
                    RichText::new(format!("on {edge} · hub {}", hub.name))
                        .font(font_ui(11.5))
                        .color(p.muted),
                );
            });
            ui.add_space(4.0);
        }
    }
    if !any {
        ui.label(
            RichText::new("No calculation nodes yet. Add one from a hub edge on the Hubs diagram.")
                .font(font_ui(12.0))
                .color(p.muted),
        );
    }
}
