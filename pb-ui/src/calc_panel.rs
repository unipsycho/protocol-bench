//! Calculation node properties — operator-facing edge transforms.
//!
//! Shown in the Hubs inspector when a calculation node is selected.
//! Catalog lives in a large modal help guide; picking a type pre-fills config.

use egui::{Align, Color32, Id, Layout, RichText, Sense, Vec2};
use pb_engine::PointSnapshot;

use crate::hubs::HubStep;
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
    live_preview(ui, step, kind, host_points, p);

    ui.add_space(14.0);
    ui.horizontal(|ui| {
        if ui
            .add(egui::Button::new(
                RichText::new("Calculation types").font(font_ui(13.0)),
            ))
            .clicked()
        {
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
        let mut still_open = open;
        egui::Window::new("Calculation types")
            .id(Id::new(("pb_calc_types_win", step_id)))
            .open(&mut still_open)
            .collapsible(false)
            .resizable(true)
            .default_size([920.0, 640.0])
            .min_width(720.0)
            .min_height(420.0)
            .show(ui.ctx(), |ui| {
                ui.label(
                    RichText::new(
                        "Twelve operator-ready blocks. Click a type to apply it and pre-fill \
configuration from the example. Types marked time use a window or time constant.",
                    )
                    .font(font_ui(13.0))
                    .color(p.text),
                );
                ui.add_space(14.0);
                egui::ScrollArea::vertical()
                    .id_salt(("pb_calc_types_scroll", step_id))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let gap = 12.0;
                        let col_w = ((ui.available_width() - gap * 2.0) / 3.0).max(200.0);
                        for row in CALC_KINDS.chunks(3) {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = gap;
                                for kind in row {
                                    let active = step.calc_kind == kind.id;
                                    ui.allocate_ui_with_layout(
                                        Vec2::new(col_w, 0.0),
                                        Layout::top_down(Align::Min),
                                        |ui| {
                                            ui.set_min_width(col_w);
                                            ui.set_max_width(col_w);
                                            if catalog_card(ui, kind, active, p) {
                                                pick = Some(kind.id);
                                            }
                                        },
                                    );
                                }
                            });
                            ui.add_space(gap);
                        }
                    });
            });
        if let Some(id) = pick {
            if let Some(kind) = kind_by_id(id) {
                apply_kind_example(step, kind);
                dirty = true;
            }
            still_open = false;
        }
        open = still_open;
        ui.ctx().data_mut(|d| d.insert_temp(modal_id, open));
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
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!("Example · {}", kind.example))
                    .font(font_ui(11.5))
                    .color(p.text),
            );
        });
    ui.interact(resp.response.rect, ui.id().with(("calc_card", kind.id)), Sense::click())
        .clicked()
}

fn source_point_picker(
    ui: &mut egui::Ui,
    value: &mut String,
    point_names: &[String],
    step_id: u64,
    p: &ThemePalette,
) -> bool {
    let mut dirty = false;
    ui.label(RichText::new("SOURCE POINT").font(font_ui(10.5)).color(p.muted));
    ui.add_space(2.0);
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
        .width(ui.available_width().max(120.0))
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
            let band = a.unwrap_or(0.5).abs();
            Ok(format!(
                "{}  (band ±{}; hold uses last published)",
                fmt_num(x),
                fmt_num(band)
            ))
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
        "rolling_avg" | "ema" | "rate_of_change" | "min_max_hold" | "time_window" => Err(
            "time-based preview needs the edge runner (sample shown as input only)".into(),
        ),
        _ => Err("no preview for this type yet".into()),
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
    p: &ThemePalette,
) {
    heading(ui, "Live preview", p.live, p);
    ui.add_space(6.0);
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
                "Source point “{}” is not in the live host yet.",
                step.source_point
            ))
            .font(font_ui(12.0))
            .color(p.muted),
        );
        return;
    };
    if pt.value.trim().is_empty() {
        ui.label(
            RichText::new(format!(
                "{} · quality {} — no value yet.",
                pt.name, pt.quality
            ))
            .font(font_ui(12.0))
            .color(p.muted),
        );
        return;
    }

    let fill = Color32::from_rgba_unmultiplied(p.live.r(), p.live.g(), p.live.b(), 24);
    egui::Frame::none()
        .fill(fill)
        .stroke(egui::Stroke::new(1.0, p.line))
        .rounding(egui::Rounding::same(8.0))
        .inner_margin(egui::Margin::symmetric(12.0, 10.0))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(
                RichText::new(format!("Point · {} · quality {}", pt.name, pt.quality))
                    .font(font_ui(11.5))
                    .color(p.muted),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Input").font(font_semi(12.5)).color(p.muted));
                ui.add_space(8.0);
                ui.label(RichText::new(&pt.value).font(font_semi(15.0)).color(p.text));
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Output").font(font_semi(12.5)).color(p.muted));
                ui.add_space(8.0);
                match parse_f64(&pt.value) {
                    Some(x) => match compute_preview(kind, step, x) {
                        Ok(out) => {
                            ui.label(RichText::new(out).font(font_semi(15.0)).color(p.accent));
                        }
                        Err(msg) => {
                            ui.label(RichText::new(msg).font(font_ui(12.0)).color(p.muted));
                        }
                    },
                    None => {
                        ui.label(
                            RichText::new("non-numeric input — preview needs a number")
                                .font(font_ui(12.0))
                                .color(p.muted),
                        );
                    }
                }
            });
            ui.add_space(4.0);
            ui.label(
                RichText::new("In-place on the path to Output · preview only (edge runner follows).")
                    .font(font_ui(11.0))
                    .color(p.muted),
            );
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
    ui.add_space(6.0);
    ui.label(
        RichText::new(format!("{} — {}", kind.name, kind.blurb))
            .font(font_ui(12.0))
            .color(p.text),
    );
    ui.add_space(10.0);
    if source_point_picker(ui, &mut step.source_point, point_names, step_id, p) {
        dirty = true;
    }
    ui.add_space(4.0);
    ui.label(
        RichText::new(
            "Transforms this point in place on its path to the Output node. \
The Output node owns the destination topic / bind.",
        )
        .font(font_ui(11.0))
        .color(p.muted),
    );
    ui.add_space(10.0);

    match kind.id {
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
        "rolling_avg" | "ema" | "rate_of_change" | "min_max_hold" | "time_window" => {
            if field_row(ui, "Window (s)", &mut step.window_secs, p) {
                dirty = true;
            }
            ui.add_space(4.0);
            if field_row(ui, "Param A", &mut step.param_a, p) {
                dirty = true;
            }
            ui.add_space(4.0);
            if field_row(ui, "Param B", &mut step.param_b, p) {
                dirty = true;
            }
            ui.add_space(4.0);
            ui.label(
                RichText::new("Time-based filters use wall clock on the host edge runner (stub).")
                    .font(font_ui(11.0))
                    .color(p.muted),
            );
        }
        _ => {
            if field_row(ui, "Param A", &mut step.param_a, p) {
                dirty = true;
            }
            ui.add_space(4.0);
            if field_row(ui, "Param B", &mut step.param_b, p) {
                dirty = true;
            }
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!("Hint · {}", kind.example))
                    .font(font_ui(11.0))
                    .color(p.muted),
            );
        }
    }
    dirty
}

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
