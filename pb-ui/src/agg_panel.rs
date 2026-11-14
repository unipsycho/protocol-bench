//! Aggregator node properties — N≥2 member combine → host `agg_N`.

use egui::{Align, Layout, RichText, Vec2};
use pb_engine::{agg_point_name, peek_agg, AggConfig, EngineHandle, PointSnapshot};
use pb_points::{PointValue, Quality};

use crate::hubs::HubStep;
use crate::theme::{font_semi, font_ui, ThemePalette};

const AGG_MODES: &[(&str, &str)] = &[
    ("sum", "Sum"),
    ("mean", "Mean"),
    ("min", "Min"),
    ("max", "Max"),
    ("count", "Count"),
    ("median", "Median"),
    ("first", "First"),
    ("last", "Last"),
    ("any-good", "Any good"),
    ("majority", "Majority"),
];

const AGG_WINDOWS: &[(&str, &str)] = &[
    ("none", "None (current)"),
    ("n:5", "Last 5"),
    ("n:10", "Last 10"),
    ("n:20", "Last 20"),
    ("n:50", "Last 50"),
    ("t:5", "Last 5 s"),
    ("t:10", "Last 10 s"),
    ("t:30", "Last 30 s"),
    ("t:60", "Last 60 s"),
];

/// Build engine aggregator configs from hub steps with ≥1 configured member.
pub fn configs_from_hubs(hubs: &crate::hubs::HubModel) -> Vec<AggConfig> {
    let mut out = Vec::new();
    for hub in &hubs.hubs {
        for step in &hub.aggregators {
            let members: Vec<String> = step
                .members
                .iter()
                .map(|m| m.trim().to_string())
                .filter(|m| !m.is_empty())
                .collect();
            if members.is_empty() {
                continue;
            }
            out.push(AggConfig {
                id: step.id,
                label: step.label.clone(),
                mode: if step.agg_mode.trim().is_empty() {
                    "mean".into()
                } else {
                    step.agg_mode.clone()
                },
                window: if step.agg_window.trim().is_empty() {
                    "none".into()
                } else {
                    step.agg_window.clone()
                },
                members,
            });
        }
    }
    out
}

fn config_from_step(step: &HubStep) -> AggConfig {
    AggConfig {
        id: step.id,
        label: step.label.clone(),
        mode: step.agg_mode.clone(),
        window: step.agg_window.clone(),
        members: step.members.clone(),
    }
}

fn mode_label(id: &str) -> &str {
    AGG_MODES
        .iter()
        .find(|(k, _)| *k == id)
        .map(|(_, l)| *l)
        .unwrap_or(id)
}

fn window_label(id: &str) -> &str {
    AGG_WINDOWS
        .iter()
        .find(|(k, _)| *k == id)
        .map(|(_, l)| *l)
        .unwrap_or(id)
}

/// Paint aggregator inspector. Returns true when mutated.
pub fn paint_node_props(
    ui: &mut egui::Ui,
    step: &mut HubStep,
    p: &ThemePalette,
    host_points: &[PointSnapshot],
    engine: &EngineHandle,
) -> bool {
    let mut dirty = false;
    let step_id = step.id;
    let point_names: Vec<String> = host_points
        .iter()
        .filter(|pt| !pt.name.starts_with("agg_"))
        .map(|pt| pt.name.clone())
        .collect();

    ui.label(
        RichText::new("Combine N≥2 member points into one derived host point.")
            .font(font_ui(12.0))
            .color(p.muted),
    );
    ui.add_space(10.0);

    // Mode + window row (matches Calc type/source rhythm).
    let gap = 12.0;
    let half = ((ui.available_width() - gap) * 0.5).max(120.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        ui.vertical(|ui| {
            ui.set_width(half);
            ui.label(RichText::new("MODE").font(font_ui(10.5)).color(p.muted));
            ui.add_space(2.0);
            let mut mode = step.agg_mode.clone();
            egui::ComboBox::from_id_salt(("pb_agg_mode", step_id))
                .selected_text(mode_label(&mode))
                .width(ui.available_width().max(100.0))
                .show_ui(ui, |ui| {
                    for (id, label) in AGG_MODES {
                        ui.selectable_value(&mut mode, (*id).to_string(), *label);
                    }
                });
            if mode != step.agg_mode {
                step.agg_mode = mode;
                dirty = true;
            }
        });
        ui.vertical(|ui| {
            ui.set_width(half);
            ui.label(RichText::new("WINDOW").font(font_ui(10.5)).color(p.muted));
            ui.add_space(2.0);
            let mut win = step.agg_window.clone();
            egui::ComboBox::from_id_salt(("pb_agg_window", step_id))
                .selected_text(window_label(&win))
                .width(ui.available_width().max(100.0))
                .show_ui(ui, |ui| {
                    for (id, label) in AGG_WINDOWS {
                        ui.selectable_value(&mut win, (*id).to_string(), *label);
                    }
                });
            if win != step.agg_window {
                step.agg_window = win;
                dirty = true;
            }
        });
    });

    ui.add_space(12.0);
    ui.label(RichText::new("MEMBERS").font(font_ui(10.5)).color(p.muted));
    ui.add_space(4.0);
    ui.label(
        RichText::new(format!("Output point · {}", agg_point_name(step_id)))
            .font(font_ui(12.0))
            .color(p.muted),
    );
    ui.add_space(6.0);

    // Ensure at least two picker rows.
    while step.members.len() < 2 {
        step.members.push(String::new());
        dirty = true;
    }

    let mut remove: Option<usize> = None;
    for i in 0..step.members.len() {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.label(
                RichText::new(format!("{}", i + 1))
                    .font(font_ui(12.0))
                    .color(p.muted),
            );
            let mut pick = step.members[i].clone();
            let label = if pick.is_empty() {
                "(pick a point)".to_string()
            } else if point_names.iter().any(|n| n == &pick) {
                pick.clone()
            } else {
                format!("{pick} (missing)")
            };
            egui::ComboBox::from_id_salt(("pb_agg_member", step_id, i))
                .selected_text(label)
                .width((ui.available_width() - 36.0).max(100.0))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut pick, String::new(), "(none)");
                    for name in &point_names {
                        ui.selectable_value(&mut pick, name.clone(), name);
                    }
                });
            if pick != step.members[i] {
                step.members[i] = pick;
                dirty = true;
            }
            if step.members.len() > 2
                && ui
                    .add(egui::Button::new(RichText::new("×").font(font_ui(14.0))).min_size(Vec2::new(28.0, 24.0)))
                    .clicked()
            {
                remove = Some(i);
            }
        });
        ui.add_space(4.0);
    }
    if let Some(i) = remove {
        step.members.remove(i);
        dirty = true;
    }

    ui.add_space(4.0);
    if ui
        .add(egui::Button::new(
            RichText::new("Add member").font(font_ui(12.5)),
        ))
        .clicked()
    {
        step.members.push(String::new());
        dirty = true;
    }

    let filled = step
        .members
        .iter()
        .filter(|m| !m.trim().is_empty())
        .count();
    if filled < 2 {
        ui.add_space(6.0);
        ui.label(
            RichText::new("Pick at least two member points to run this aggregator.")
                .font(font_ui(12.0))
                .color(p.muted),
        );
    }

    ui.add_space(14.0);
    live_preview(ui, step, host_points, engine, p);

    dirty
}

fn live_preview(
    ui: &mut egui::Ui,
    step: &HubStep,
    host_points: &[PointSnapshot],
    engine: &EngineHandle,
    p: &ThemePalette,
) {
    ui.label(RichText::new("LIVE PREVIEW").font(font_ui(10.5)).color(p.muted));
    ui.add_space(4.0);
    let filled: Vec<&str> = step
        .members
        .iter()
        .map(|m| m.trim())
        .filter(|m| !m.is_empty())
        .collect();
    if filled.len() < 2 {
        ui.label(
            RichText::new("Preview needs two or more members.")
                .font(font_ui(12.0))
                .color(p.muted),
        );
        return;
    }

    let cfg = config_from_step(step);
    let lookup = |name: &str| {
        host_points.iter().find(|pt| pt.name == name).map(|pt| {
            let value = pt
                .value
                .parse::<f64>()
                .map(PointValue::F64)
                .unwrap_or_else(|_| PointValue::String(pt.value.clone()));
            let quality = match pt.quality.as_str() {
                "bad" => Quality::Bad,
                "uncertain" => Quality::Uncertain,
                _ => Quality::Good,
            };
            pb_engine::MemberReading {
                name: pt.name.clone(),
                value,
                quality,
                updated_ms: pt.updated_ms,
            }
        })
    };

    let out = if let Ok(rt) = engine.agg_runtime().lock() {
        peek_agg(&cfg, &rt, &lookup)
    } else {
        None
    };

    match out {
        Some(o) => {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(RichText::new(&o.name).font(font_ui(12.0)).color(p.muted));
                ui.label(RichText::new("→").font(font_ui(13.0)).color(p.muted));
                ui.label(RichText::new(o.value).font(font_semi(13.0)).color(p.accent));
                ui.label(
                    RichText::new(o.quality.label())
                        .font(font_ui(11.5))
                        .color(p.muted),
                );
            });
            ui.add_space(4.0);
            ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                for name in filled {
                    if let Some(pt) = host_points.iter().find(|pt| pt.name == name) {
                        ui.label(
                            RichText::new(format!("{name}={}", pt.value))
                                .font(font_ui(11.5))
                                .color(p.text),
                        );
                    } else {
                        ui.label(
                            RichText::new(format!("{name}=?"))
                                .font(font_ui(11.5))
                                .color(p.muted),
                        );
                    }
                }
            });
        }
        None => {
            ui.label(
                RichText::new("Waiting for live member values…")
                    .font(font_ui(12.0))
                    .color(p.muted),
            );
        }
    }
}
