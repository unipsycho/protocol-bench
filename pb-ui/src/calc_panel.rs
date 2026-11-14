//! Calculation node properties — operator-facing edge transforms.
//!
//! Shown in the Hubs inspector when a calculation node is selected.
//! Catalog lives in a large modal help guide; picking a type pre-fills config.

use egui::{Align, Align2, Color32, Id, Layout, RichText, Sense, Vec2};
use pb_engine::{peek_for_point, CalcConfig, EngineHandle, PointSnapshot};

use crate::hubs::{HubModel, HubStep};
use crate::theme::{font_semi, font_ui, ThemePalette};

/// Build engine calc configs from every hub calculation with a source point.
pub fn configs_from_hubs(hubs: &HubModel) -> Vec<CalcConfig> {
    let mut out = Vec::new();
    for hub in &hubs.hubs {
        for step in &hub.calculations {
            if step.source_point.trim().is_empty() {
                continue;
            }
            out.push(config_from_step(step));
        }
    }
    out
}

fn config_from_step(step: &HubStep) -> CalcConfig {
    CalcConfig {
        id: step.id,
        kind: step.calc_kind.clone(),
        source_point: step.source_point.clone(),
        param_a: step.param_a.clone(),
        param_b: step.param_b.clone(),
        window_secs: step.window_secs.clone(),
        formula: step.formula.clone(),
    }
}

/// One catalog entry operators can place on a hub edge.
#[derive(Debug, Clone, Copy)]
pub struct CalcKind {
    pub id: &'static str,
    pub name: &'static str,
    pub blurb: &'static str,
    pub example: &'static str,
    pub time_based: bool,
}

/// Built-in calculation types.
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

fn kind_by_id(id: &str) -> Option<&'static CalcKind> {
    CALC_KINDS.iter().find(|k| k.id == id)
}

/// Human label for a stored calc kind id.
pub fn kind_label(id: &str) -> &str {
    kind_by_id(id).map(|k| k.name).unwrap_or("Calculation")
}

/// Apply catalog example values onto the step.
fn apply_kind_example(step: &mut HubStep, kind: &CalcKind) {
    let prev_name = kind_by_id(step.calc_kind.as_str()).map(|k| k.name);
    step.calc_kind = kind.id.into();
    if step.label.is_empty()
        || step.label == "Calculation"
        || prev_name.is_some_and(|n| step.label == n)
    {
        step.label = kind.name.into();
    }
    step.formula.clear();
    step.param_a.clear();
    step.param_b.clear();
    step.window_secs.clear();
    match kind.id {
        "scale_offset" => {
            step.param_a = "0.1".into();
            step.param_b = "-40".into();
        }
        "unit_convert" => {
            step.param_a = "°C".into();
            step.param_b = "°F".into();
        }
        "clamp" => {
            step.param_a = "0".into();
            step.param_b = "100".into();
        }
        "deadband" => {
            step.param_a = "0.5".into();
        }
        "rolling_avg" => {
            step.window_secs = "10".into();
            step.param_a = "auto".into();
        }
        "ema" => {
            step.window_secs = "5".into();
            step.param_a = "0.2".into();
        }
        "rate_of_change" => {
            step.param_a = "/s".into();
        }
        "min_max_hold" => {
            step.window_secs = "30".into();
            step.param_a = "max".into();
        }
        "hysteresis" => {
            step.param_a = "80".into();
            step.param_b = "70".into();
        }
        "boolean_compare" => {
            step.param_a = ">".into();
            step.param_b = "50".into();
        }
        "formula" => {
            step.formula = "(A * 1.8) + 32".into();
        }
        "time_window" => {
            step.window_secs = "60".into();
            step.param_a = "avg".into();
        }
        _ => {}
    }
}

fn modal_open_id(step_id: u64) -> Id {
    Id::new(("pb_calc_types_modal", step_id))
}

/// Paint calculation node properties (Hubs inspector). Returns true when mutated.
pub fn paint_node_props(
    ui: &mut egui::Ui,
    step: &mut HubStep,
    p: &ThemePalette,
    host_points: &[PointSnapshot],
    engine: &EngineHandle,
) -> bool {
    let mut dirty = false;
    let step_id = step.id;
    let modal_id = modal_open_id(step_id);
    let point_names: Vec<String> = host_points.iter().map(|pt| pt.name.clone()).collect();

    // Configuration first: source point + type params, then live preview.
    let kind = kind_by_id(step.calc_kind.as_str()).unwrap_or(&CALC_KINDS[0]);
    if step.calc_kind != kind.id {
        step.calc_kind = kind.id.into();
        dirty = true;
    }
    if detail_editor(ui, kind, step, p, &point_names) {
        dirty = true;
    }

    ui.add_space(14.0);
    live_preview(ui, step, kind, host_points, engine, p);

    ui.add_space(14.0);
    ui.horizontal(|ui| {
        // Accent CTA — distinct from primary form/save buttons (dark label on accent fill).
        let types_btn = egui::Button::new(
            RichText::new("Calculation types")
                .font(font_semi(13.0))
                .color(Color32::from_rgb(0x12, 0x14, 0x18)),
        )
        .fill(p.accent)
        .stroke(egui::Stroke::new(1.0, p.accent))
        .min_size(Vec2::new(0.0, 28.0));
        if ui.add(types_btn).clicked() {
            ui.ctx().data_mut(|d| d.insert_temp(modal_id, true));
        }
        ui.label(
            RichText::new("Help guide — pick a type to pre-fill this config.")
                .font(font_ui(11.5))
                .color(p.muted),
        );
    });

    let mut open = ui
        .ctx()
        .data(|d| d.get_temp::<bool>(modal_id))
        .unwrap_or(false);
    // One-shot: `PB_SHOT_CALC_TYPES=1` opens the guide for screenshots.
    if !open && std::env::var_os("PB_SHOT_CALC_TYPES").is_some() {
        open = true;
        ui.ctx().data_mut(|d| d.insert_temp(modal_id, true));
    }
    if open {
        let mut pick = None;
        let mut close_clicked = false;
        let mut still_open = open;
        // Escape always dismisses — even when the title-bar X is off-screen.
        if ui
            .ctx()
            .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            still_open = false;
        }
        // Mark open so the hub canvas does not steal wheel zoom behind the modal.
        ui.ctx()
            .data_mut(|d| d.insert_temp(Id::new("pb_calc_types_blocking"), true));

        // Size to a 3-column catalog; hard-cap to the viewport so X stays reachable.
        const COL: f32 = 220.0;
        const GAP: f32 = 12.0;
        const CARD_H: f32 = 128.0;
        const ROWS: f32 = 4.0;
        let screen = ui.ctx().screen_rect().size();
        let max_w = (screen.x - 48.0).max(280.0);
        let max_h = (screen.y - 48.0).max(200.0);
        let ideal_w = (COL * 3.0 + GAP * 2.0 + 48.0).min(max_w);
        let ideal_h = (72.0 + ROWS * CARD_H + (ROWS - 1.0) * GAP + 56.0).min(max_h);

        egui::Window::new("Calculation types")
            .id(Id::new(("pb_calc_types_win", step_id)))
            .open(&mut still_open)
            .collapsible(false)
            .resizable(true)
            .constrain(true)
            .order(egui::Order::Foreground)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .default_size([ideal_w, ideal_h])
            .min_width(280.0_f32.min(max_w))
            .min_height(200.0_f32.min(max_h))
            .max_size([max_w, max_h])
            .show(ui.ctx(), |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(
                            "Twelve operator-ready blocks. Click a type to apply it and pre-fill \
configuration from the example. Types marked time use a window or time constant.",
                        )
                        .font(font_ui(13.0))
                        .color(p.text),
                    );
                    ui.with_layout(Layout::right_to_left(Align::TOP), |ui| {
                        // Extra Close — title-bar X can be unreachable if the window was oversized.
                        if ui
                            .add(egui::Button::new(RichText::new("Close").font(font_ui(12.5))))
                            .clicked()
                        {
                            close_clicked = true;
                        }
                    });
                });
                ui.add_space(10.0);
                // Bound the scroll body so the grid cannot inflate the window past max_size.
                let body_h = (ui.available_height() - 8.0).max(80.0);
                egui::ScrollArea::vertical()
                    .id_salt(("pb_calc_types_scroll", step_id))
                    .max_height(body_h)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        // Equal 3-column grid — fixed card size, no staggered flow.
                        let col_w = ((ui.available_width() - GAP * 2.0) / 3.0).clamp(160.0, COL);
                        egui::Grid::new(("pb_calc_types_grid", step_id))
                            .num_columns(3)
                            .spacing([GAP, GAP])
                            .min_col_width(col_w)
                            .max_col_width(col_w)
                            .show(ui, |ui| {
                                for (i, kind) in CALC_KINDS.iter().enumerate() {
                                    let active = step.calc_kind == kind.id;
                                    ui.allocate_ui_with_layout(
                                        Vec2::new(col_w, CARD_H),
                                        Layout::top_down(Align::Min),
                                        |ui| {
                                            ui.set_min_size(Vec2::new(col_w, CARD_H));
                                            ui.set_max_size(Vec2::new(col_w, CARD_H));
                                            if catalog_card(ui, kind, active, p) {
                                                pick = Some(kind.id);
                                            }
                                        },
                                    );
                                    if (i + 1) % 3 == 0 {
                                        ui.end_row();
                                    }
                                }
                            });
                    });
            });
        if close_clicked {
            still_open = false;
        }
        if let Some(id) = pick {
            if let Some(kind) = kind_by_id(id) {
                apply_kind_example(step, kind);
                dirty = true;
            }
            still_open = false;
        }
        open = still_open;
        ui.ctx().data_mut(|d| {
            d.insert_temp(modal_id, open);
            if !open {
                d.insert_temp(Id::new("pb_calc_types_blocking"), false);
            }
        });
    } else {
        ui.ctx()
            .data_mut(|d| d.insert_temp(Id::new("pb_calc_types_blocking"), false));
    }

    dirty
}

fn heading(ui: &mut egui::Ui, title: &str, color: Color32, _p: &ThemePalette) {
    ui.label(RichText::new(title).font(font_semi(14.0)).color(color));
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
    let size = ui.available_size();
    let resp = egui::Frame::none()
        .fill(fill)
        .stroke(stroke)
        .rounding(egui::Rounding::same(8.0))
        .inner_margin(egui::Margin::symmetric(12.0, 10.0))
        .show(ui, |ui| {
            ui.set_min_size(size);
            ui.set_max_size(size);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(kind.name)
                        .font(font_semi(13.5))
                        .color(if active { p.accent } else { p.text }),
                );
                if kind.time_based {
                    ui.add_space(6.0);
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
            ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
                ui.label(
                    RichText::new(format!("Example · {}", kind.example))
                        .font(font_ui(11.5))
                        .color(p.text),
                );
            });
        });
    ui.interact(resp.response.rect, ui.id().with(("calc_card", kind.id)), Sense::click())
        .clicked()
}

fn source_point_combo(
    ui: &mut egui::Ui,
    value: &mut String,
    point_names: &[String],
    step_id: u64,
    width: f32,
) -> bool {
    let mut dirty = false;
    let mut pick = value.clone();
    // Keep a stale selection visible until the operator picks again.
    let label = if pick.is_empty() {
        "(pick a point)".to_string()
    } else if point_names.iter().any(|n| n == &pick) {
        pick.clone()
    } else {
        format!("{pick} (missing)")
    };
    egui::ComboBox::from_id_salt(("pb_calc_src_point", step_id))
        .selected_text(label)
        .width(width.max(100.0))
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut pick, String::new(), "(none)");
            for name in point_names {
                ui.selectable_value(&mut pick, name.clone(), name);
            }
        });
    if pick != *value {
        *value = pick;
        dirty = true;
    }
    dirty
}

fn parse_f64(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    // Accept bool-ish for compare preview.
    match t.to_ascii_lowercase().as_str() {
        "true" | "on" => return Some(1.0),
        "false" | "off" => return Some(0.0),
        _ => {}
    }
    t.parse::<f64>().ok()
}

fn fmt_num(v: f64) -> String {
    if v.fract().abs() < 1e-9 {
        format!("{v:.0}")
    } else {
        format!("{v:.6}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string()
    }
}

/// Best-effort UI preview (not the host edge runner).
fn compute_preview(kind: &CalcKind, step: &HubStep, x: f64) -> Result<String, String> {
    let a = parse_f64(&step.param_a);
    let b = parse_f64(&step.param_b);
    match kind.id {
        "scale_offset" => {
            let scale = a.unwrap_or(1.0);
            let offset = b.unwrap_or(0.0);
            Ok(fmt_num(x * scale + offset))
        }
        "unit_convert" => {
            let from = step.param_a.trim();
            let to = step.param_b.trim();
            let y = match (from, to) {
                ("°C" | "C" | "c", "°F" | "F" | "f") => x * 1.8 + 32.0,
                ("°F" | "F" | "f", "°C" | "C" | "c") => (x - 32.0) / 1.8,
                ("psi", "kPa") => x * 6.894757,
                ("kPa", "psi") => x / 6.894757,
                _ => {
                    return Err(format!(
                        "preview needs known units (got {from} → {to}); try °C/°F or psi/kPa"
                    ));
                }
            };
            Ok(fmt_num(y))
        }
        "clamp" => {
            let lo = a.unwrap_or(0.0);
            let hi = b.unwrap_or(100.0);
            Ok(fmt_num(x.clamp(lo.min(hi), lo.max(hi))))
        }
        "deadband" => {
            // Stateless fallback only — live_preview prefers engine hold state.
            let band = a.unwrap_or(0.5).abs();
            Ok(format!("{}  (band ±{})", fmt_num(x), fmt_num(band)))
        }
        "hysteresis" => {
            let on = a.unwrap_or(80.0);
            let off = b.unwrap_or(70.0);
            let out = if x >= on {
                "true (on)"
            } else if x <= off {
                "false (off)"
            } else {
                "hold (between thresholds)"
            };
            Ok(out.into())
        }
        "boolean_compare" => {
            let op = step.param_a.trim();
            let rhs = b.unwrap_or(0.0);
            let ok = match op {
                ">" => x > rhs,
                ">=" => x >= rhs,
                "<" => x < rhs,
                "<=" => x <= rhs,
                "==" | "=" => (x - rhs).abs() < 1e-9,
                "!=" => (x - rhs).abs() >= 1e-9,
                _ => {
                    return Err("Param A should be a compare op (>, >=, <, <=, ==, !=)".into());
                }
            };
            Ok(ok.to_string())
        }
        "formula" => {
            let expr = step.formula.trim();
            if expr.is_empty() {
                return Err("enter a formula to preview".into());
            }
            // Tiny evaluator: replace x / A with the sample, then allow + - * / ( ) numbers.
            let mut s = expr.replace('x', &format!("({x})"));
            s = s.replace('A', &format!("({x})"));
            s = s.replace('B', &format!("({})", b.unwrap_or(0.0)));
            match eval_simple_arith(&s) {
                Some(v) => Ok(fmt_num(v)),
                None => Err("formula preview supports + − × ÷ ( ) and x/A/B only".into()),
            }
        }
        "rolling_avg" | "ema" | "rate_of_change" | "min_max_hold" | "time_window" => {
            // Pass-through until the edge runner owns window state.
            Ok(fmt_num(x))
        }
        _ => Ok(fmt_num(x)),
    }
}

/// Recursive-descent arithmetic for formula preview (no functions).
fn eval_simple_arith(expr: &str) -> Option<f64> {
    let chars: Vec<char> = expr.chars().filter(|c| !c.is_whitespace()).collect();
    let mut i = 0usize;
    fn parse_expr(chars: &[char], i: &mut usize) -> Option<f64> {
        let mut v = parse_term(chars, i)?;
        while *i < chars.len() {
            match chars[*i] {
                '+' => {
                    *i += 1;
                    v += parse_term(chars, i)?;
                }
                '-' => {
                    *i += 1;
                    v -= parse_term(chars, i)?;
                }
                _ => break,
            }
        }
        Some(v)
    }
    fn parse_term(chars: &[char], i: &mut usize) -> Option<f64> {
        let mut v = parse_factor(chars, i)?;
        while *i < chars.len() {
            match chars[*i] {
                '*' => {
                    *i += 1;
                    v *= parse_factor(chars, i)?;
                }
                '/' => {
                    *i += 1;
                    let d = parse_factor(chars, i)?;
                    if d == 0.0 {
                        return None;
                    }
                    v /= d;
                }
                _ => break,
            }
        }
        Some(v)
    }
    fn parse_factor(chars: &[char], i: &mut usize) -> Option<f64> {
        if *i >= chars.len() {
            return None;
        }
        match chars[*i] {
            '+' => {
                *i += 1;
                parse_factor(chars, i)
            }
            '-' => {
                *i += 1;
                Some(-(parse_factor(chars, i)?))
            }
            '(' => {
                *i += 1;
                let v = parse_expr(chars, i)?;
                if *i >= chars.len() || chars[*i] != ')' {
                    return None;
                }
                *i += 1;
                Some(v)
            }
            c if c.is_ascii_digit() || c == '.' => {
                let start = *i;
                while *i < chars.len() && (chars[*i].is_ascii_digit() || chars[*i] == '.') {
                    *i += 1;
                }
                chars[start..*i].iter().collect::<String>().parse().ok()
            }
            _ => None,
        }
    }
    let v = parse_expr(&chars, &mut i)?;
    if i != chars.len() {
        return None;
    }
    Some(v)
}

fn live_preview(
    ui: &mut egui::Ui,
    step: &HubStep,
    kind: &CalcKind,
    host_points: &[PointSnapshot],
    engine: &EngineHandle,
    p: &ThemePalette,
) {
    ui.label(RichText::new("LIVE PREVIEW").font(font_ui(10.5)).color(p.muted));
    ui.add_space(4.0);
    if step.source_point.trim().is_empty() {
        ui.label(
            RichText::new("Pick a source point to preview input → output.")
                .font(font_ui(12.0))
                .color(p.muted),
        );
        return;
    }
    let Some(pt) = host_points.iter().find(|pt| pt.name == step.source_point) else {
        ui.label(
            RichText::new(format!(
                "Source “{}” not live yet.",
                step.source_point
            ))
            .font(font_ui(12.0))
            .color(p.muted),
        );
        return;
    };
    if pt.value.trim().is_empty() {
        ui.label(
            RichText::new(format!("{} · no value yet", pt.name))
                .font(font_ui(12.0))
                .color(p.muted),
        );
        return;
    }

    // Stateful kinds peek the shared edge-runner; others stay local.
    let out = match kind.id {
        "deadband" | "rolling_avg" | "rate_of_change" | "hysteresis" | "scale_offset"
        | "clamp" => {
            let cfg = config_from_step(step);
            if let Ok(rt) = engine.calc_runtime().lock() {
                peek_for_point(&[cfg], &rt, &step.source_point, &pt.value)
            } else {
                parse_f64(&pt.value)
                    .and_then(|x| compute_preview(kind, step, x).ok())
                    .unwrap_or_else(|| pt.value.clone())
            }
        }
        _ => match parse_f64(&pt.value) {
            Some(x) => compute_preview(kind, step, x).unwrap_or_else(|m| m),
            None => "non-numeric".into(),
        },
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(RichText::new("Input").font(font_ui(12.0)).color(p.muted));
        ui.label(RichText::new(&pt.value).font(font_semi(13.0)).color(p.text));
        ui.label(RichText::new("→").font(font_ui(13.0)).color(p.muted));
        ui.label(RichText::new("Output").font(font_ui(12.0)).color(p.muted));
        ui.label(RichText::new(out).font(font_semi(13.0)).color(p.accent));
    });
}

fn detail_editor(
    ui: &mut egui::Ui,
    kind: &CalcKind,
    step: &mut HubStep,
    p: &ThemePalette,
    point_names: &[String],
) -> bool {
    let mut dirty = false;
    let step_id = step.id;
    heading(ui, "Configure", p.accent, p);
    ui.add_space(8.0);
    // Type + source on one row; blurb under.
    let mut kind_id = step.calc_kind.clone();
    let gap = 12.0;
    let half = ((ui.available_width() - gap) * 0.5).max(120.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        ui.vertical(|ui| {
            ui.set_width(half);
            ui.label(RichText::new("TYPE").font(font_ui(10.5)).color(p.muted));
            ui.add_space(2.0);
            egui::ComboBox::from_id_salt(("pb_calc_type", step_id))
                .selected_text(kind.name)
                .width(ui.available_width().max(100.0))
                .show_ui(ui, |ui| {
                    for k in CALC_KINDS {
                        ui.selectable_value(&mut kind_id, k.id.to_string(), k.name);
                    }
                });
        });
        ui.vertical(|ui| {
            ui.set_width(half);
            ui.label(RichText::new("SOURCE POINT").font(font_ui(10.5)).color(p.muted));
            ui.add_space(2.0);
            if source_point_combo(
                ui,
                &mut step.source_point,
                point_names,
                step_id,
                ui.available_width(),
            ) {
                dirty = true;
            }
        });
    });
    if kind_id != step.calc_kind {
        if let Some(next) = kind_by_id(&kind_id) {
            apply_kind_example(step, next);
            dirty = true;
        }
    }
    let kind = kind_by_id(step.calc_kind.as_str()).unwrap_or(kind);
    ui.add_space(4.0);
    ui.label(
        RichText::new(kind.blurb)
            .font(font_ui(12.0))
            .color(p.muted),
    );
    ui.add_space(10.0);

    match kind.id {
        "scale_offset" => {
            if field_row(ui, "Scale", &mut step.param_a, p) {
                dirty = true;
            }
            ui.add_space(4.0);
            if field_row(ui, "Offset", &mut step.param_b, p) {
                dirty = true;
            }
        }
        "unit_convert" => {
            if enum_row(
                ui,
                "From",
                &mut step.param_a,
                UNIT_CHOICES,
                ("pb_calc_unit_from", step_id),
                p,
            ) {
                dirty = true;
            }
            ui.add_space(4.0);
            if enum_row(
                ui,
                "To",
                &mut step.param_b,
                UNIT_CHOICES,
                ("pb_calc_unit_to", step_id),
                p,
            ) {
                dirty = true;
            }
        }
        "clamp" => {
            if field_row(ui, "Min", &mut step.param_a, p) {
                dirty = true;
            }
            ui.add_space(4.0);
            if field_row(ui, "Max", &mut step.param_b, p) {
                dirty = true;
            }
        }
        "deadband" => {
            if field_row(ui, "Band ±", &mut step.param_a, p) {
                dirty = true;
            }
        }
        "rolling_avg" => {
            if field_row(ui, "Window (s)", &mut step.window_secs, p) {
                dirty = true;
            }
            ui.add_space(4.0);
            if enum_row(
                ui,
                "Samples",
                &mut step.param_a,
                SAMPLE_MODES,
                ("pb_calc_roll_samples", step_id),
                p,
            ) {
                dirty = true;
            }
        }
        "ema" => {
            if field_row(ui, "Tau (s)", &mut step.window_secs, p) {
                dirty = true;
            }
            ui.add_space(4.0);
            if field_row(ui, "Alpha", &mut step.param_a, p) {
                dirty = true;
            }
        }
        "rate_of_change" => {
            if enum_row(
                ui,
                "Units",
                &mut step.param_a,
                ROC_UNITS,
                ("pb_calc_roc_units", step_id),
                p,
            ) {
                dirty = true;
            }
            ui.add_space(4.0);
            if enum_row(
                ui,
                "Sign",
                &mut step.param_b,
                ROC_SIGN,
                ("pb_calc_roc_sign", step_id),
                p,
            ) {
                dirty = true;
            }
        }
        "min_max_hold" => {
            if enum_row(
                ui,
                "Mode",
                &mut step.param_a,
                HOLD_MODES,
                ("pb_calc_hold_mode", step_id),
                p,
            ) {
                dirty = true;
            }
            ui.add_space(4.0);
            if field_row(ui, "Hold (s)", &mut step.window_secs, p) {
                dirty = true;
            }
        }
        "hysteresis" => {
            if field_row(ui, "On ≥", &mut step.param_a, p) {
                dirty = true;
            }
            ui.add_space(4.0);
            if field_row(ui, "Off ≤", &mut step.param_b, p) {
                dirty = true;
            }
        }
        "boolean_compare" => {
            if enum_row(
                ui,
                "Operator",
                &mut step.param_a,
                COMPARE_OPS,
                ("pb_calc_cmp_op", step_id),
                p,
            ) {
                dirty = true;
            }
            ui.add_space(4.0);
            if field_row(ui, "Setpoint", &mut step.param_b, p) {
                dirty = true;
            }
        }
        "formula" => {
            ui.label(RichText::new("FORMULA").font(font_ui(10.5)).color(p.muted));
            ui.add_space(4.0);
            if ui
                .add(
                    egui::TextEdit::multiline(&mut step.formula)
                        .desired_width(f32::INFINITY)
                        .desired_rows(3),
                )
                .changed()
            {
                dirty = true;
            }
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "Use point ids or x for the edge value. Functions: min, max, abs, clamp, sqrt.",
                )
                .font(font_ui(11.0))
                .color(p.muted),
            );
        }
        "time_window" => {
            if field_row(ui, "Window (s)", &mut step.window_secs, p) {
                dirty = true;
            }
            ui.add_space(4.0);
            if enum_row(
                ui,
                "Mode",
                &mut step.param_a,
                TIME_WINDOW_MODES,
                ("pb_calc_tw_mode", step_id),
                p,
            ) {
                dirty = true;
            }
        }
        _ => {
            if field_row(ui, "Param A", &mut step.param_a, p) {
                dirty = true;
            }
            ui.add_space(4.0);
            if field_row(ui, "Param B", &mut step.param_b, p) {
                dirty = true;
            }
        }
    }
    dirty
}

const COMPARE_OPS: &[(&str, &str)] = &[
    (">", ">"),
    (">=", "≥"),
    ("<", "<"),
    ("<=", "≤"),
    ("==", "="),
    ("!=", "≠"),
];

const UNIT_CHOICES: &[(&str, &str)] = &[
    ("°C", "°C"),
    ("°F", "°F"),
    ("psi", "psi"),
    ("kPa", "kPa"),
    ("bar", "bar"),
    ("m", "m"),
    ("ft", "ft"),
];

const HOLD_MODES: &[(&str, &str)] = &[("min", "Min"), ("max", "Max")];

const TIME_WINDOW_MODES: &[(&str, &str)] = &[
    ("avg", "Average"),
    ("gate", "Gate"),
    ("debounce", "Debounce"),
];

const ROC_UNITS: &[(&str, &str)] = &[
    ("/s", "per second"),
    ("/min", "per minute"),
    ("/h", "per hour"),
];

const ROC_SIGN: &[(&str, &str)] = &[("", "Signed"), ("abs", "Absolute")];

const SAMPLE_MODES: &[(&str, &str)] = &[
    ("auto", "Auto"),
    ("5", "5"),
    ("10", "10"),
    ("20", "20"),
    ("50", "50"),
];

fn field_row(ui: &mut egui::Ui, label: &str, value: &mut String, p: &ThemePalette) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.add_sized(
            [88.0, 20.0],
            egui::Label::new(RichText::new(label).font(font_ui(12.0)).color(p.muted)),
        );
        if ui
            .add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY))
            .changed()
        {
            changed = true;
        }
    });
    changed
}

fn enum_row(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut String,
    choices: &[(&str, &str)],
    salt: impl std::hash::Hash,
    p: &ThemePalette,
) -> bool {
    let mut changed = false;
    // Normalize unknown stored values to the first choice.
    if !choices.iter().any(|(id, _)| *id == value.as_str()) {
        if let Some((id, _)) = choices.first() {
            *value = (*id).into();
            changed = true;
        }
    }
    let shown = choices
        .iter()
        .find(|(id, _)| *id == value.as_str())
        .map(|(_, label)| (*label).to_string())
        .unwrap_or_else(|| value.clone());
    ui.horizontal(|ui| {
        ui.add_sized(
            [88.0, 20.0],
            egui::Label::new(RichText::new(label).font(font_ui(12.0)).color(p.muted)),
        );
        let mut pick = value.clone();
        egui::ComboBox::from_id_salt(salt)
            .selected_text(shown)
            .width(ui.available_width().max(80.0))
            .show_ui(ui, |ui| {
                for (id, name) in choices {
                    ui.selectable_value(&mut pick, (*id).to_string(), *name);
                }
            });
        if pick != *value {
            *value = pick;
            changed = true;
        }
    });
    changed
}
