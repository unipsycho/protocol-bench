//! LAS panel — file endpoints for follow (Input) and write (Output). Locked LAS plan.

use egui::{Layout, RichText, Sense, UiBuilder};

use crate::connection_list::{self, RowAction};
use crate::theme::{apply_form_spacing, font_ui, form_inset, space, ThemePalette};

const LABEL_W: f32 = 110.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LasRole {
    Incoming,
    Outgoing,
}

impl LasRole {
    pub fn label(self) -> &'static str {
        match self {
            Self::Incoming => "Incoming (follow)",
            Self::Outgoing => "Outgoing (write)",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "outgoing" | "out" | "write" | "target" => Self::Outgoing,
            _ => Self::Incoming,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Incoming => "incoming",
            Self::Outgoing => "outgoing",
        }
    }
}

#[derive(Debug, Clone)]
pub struct LasEndpoint {
    pub id: String,
    pub label: String,
    pub role: LasRole,
    pub path: String,
    /// Declared / detected LAS version label (`2.0`, `1.2`, …).
    pub version: String,
    pub follow: bool,
}

impl LasEndpoint {
    pub fn default_incoming() -> Self {
        Self {
            id: "well-a".into(),
            label: "Well A".into(),
            role: LasRole::Incoming,
            path: String::new(),
            version: "2.0".into(),
            follow: true,
        }
    }

    pub fn summary(&self) -> String {
        let path = if self.path.trim().is_empty() {
            "(no path)"
        } else {
            self.path.as_str()
        };
        format!(
            "{}  LAS {}  ·  {}",
            path,
            self.version,
            if self.role == LasRole::Incoming {
                if self.follow {
                    "follow"
                } else {
                    "once"
                }
            } else {
                "write"
            }
        )
    }
}

#[derive(Debug, Clone)]
pub struct LasPanelState {
    pub endpoints: Vec<LasEndpoint>,
    pub selected: Option<usize>,
    pub pending_delete: Option<(usize, String)>,
}

impl Default for LasPanelState {
    fn default() -> Self {
        Self {
            endpoints: vec![LasEndpoint::default_incoming()],
            selected: None,
            pending_delete: None,
        }
    }
}

impl LasPanelState {
    pub fn selected(&self) -> Option<&LasEndpoint> {
        self.selected.and_then(|i| self.endpoints.get(i))
    }

    pub fn selected_mut(&mut self) -> Option<&mut LasEndpoint> {
        self.selected.and_then(|i| self.endpoints.get_mut(i))
    }

    pub fn connection_choices(&self) -> Vec<(String, String)> {
        self.endpoints
            .iter()
            .map(|e| {
                let label = if e.label.trim().is_empty() {
                    e.id.clone()
                } else {
                    e.label.clone()
                };
                (e.id.clone(), format!("{} · {}", label, e.role.label()))
            })
            .collect()
    }

    fn ensure_endpoints(&mut self) {
        if self.endpoints.is_empty() {
            self.endpoints.push(LasEndpoint::default_incoming());
            self.selected = None;
        } else if let Some(i) = self.selected {
            if i >= self.endpoints.len() {
                self.selected = None;
            }
        }
    }

    fn unique_id(&self, base: &str) -> String {
        let mut id = base.to_string();
        let mut n = 2;
        while self.endpoints.iter().any(|e| e.id == id) {
            id = format!("{base}-{n}");
            n += 1;
        }
        id
    }
}

pub fn paint(ui: &mut egui::Ui, p: &ThemePalette, state: &mut LasPanelState) -> bool {
    state.ensure_endpoints();
    let mut dirty = false;
    let full = ui.available_rect_before_wrap();
    let inner = form_inset(full);
    ui.allocate_new_ui(
        UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::top_down(egui::Align::Min)),
        |ui| {
            dirty = paint_body(ui, p, state);
        },
    );
    ui.allocate_rect(full, Sense::hover());
    dirty
}

fn paint_body(ui: &mut egui::Ui, p: &ThemePalette, state: &mut LasPanelState) -> bool {
    apply_form_spacing(ui);
    let mut dirty = false;
    ui.label(
        RichText::new(
            "Log ASCII files for hub Inputs (follow) and Outputs (write). LAS 2.0 primary; 1.x accepted.",
        )
        .font(font_ui(12.5))
        .color(p.muted),
    );
    ui.add_space(space::AFTER_BLURB);

    if connection_list::heading_with_add(ui, "Endpoints", "+ Add", p.live, p) {
        let mut e = LasEndpoint::default_incoming();
        e.id = state.unique_id("las");
        e.label = format!("LAS {}", state.endpoints.len() + 1);
        state.endpoints.push(e);
        state.selected = Some(state.endpoints.len() - 1);
        dirty = true;
    }
    ui.add_space(space::AFTER_HEADING);
    ui.label(
        RichText::new("File path and role — curve binds live on hub Input / Output (LAS) nodes.")
            .font(font_ui(12.0))
            .color(p.muted),
    );
    ui.add_space(space::AFTER_BLURB);

    let mut select = None;
    let show_delete = state.endpoints.len() > 1;
    for (i, ep) in state.endpoints.iter().enumerate() {
        let selected = state.selected == Some(i);
        let label = if ep.label.trim().is_empty() {
            ep.id.clone()
        } else {
            ep.label.clone()
        };
        match connection_list::selectable_row(ui, &label, &ep.summary(), selected, show_delete, p) {
            Some(RowAction::Select) => select = Some(i),
            Some(RowAction::Delete) => {
                state.pending_delete = Some((i, label));
            }
            None => {}
        }
        ui.add_space(2.0);
    }
    if let Some(i) = select {
        state.selected = if state.selected == Some(i) {
            None
        } else {
            Some(i)
        };
    }
    if let Some(i) = connection_list::confirm_delete(ui.ctx(), &mut state.pending_delete, p) {
        if i < state.endpoints.len() {
            let was = state.selected;
            state.endpoints.remove(i);
            dirty = true;
            state.selected = match was {
                Some(s) if s == i => None,
                Some(s) if s > i => Some(s - 1),
                other => other,
            };
            state.ensure_endpoints();
        }
    }

    ui.add_space(space::SECTION);
    if state.selected().is_none() {
        ui.label(
            RichText::new("Select an endpoint to edit.")
                .font(font_ui(12.0))
                .color(p.muted),
        );
        return dirty;
    }

    dirty |= edit_endpoint(ui, state, p);
    dirty
}

fn edit_endpoint(ui: &mut egui::Ui, state: &mut LasPanelState, p: &ThemePalette) -> bool {
    let mut dirty = false;
    let mut browse = false;
    {
        let e = state.selected_mut().unwrap();
        dirty |= field(ui, "Id", &mut e.id, p);
        dirty |= field(ui, "Label", &mut e.label, p);

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = space::ITEM_X;
            ui.add_sized(
                [LABEL_W, 20.0],
                egui::Label::new(RichText::new("Role").font(font_ui(12.0)).color(p.muted)),
            );
            let mut role = e.role;
            egui::ComboBox::from_id_salt("pb_las_role")
                .selected_text(role.label())
                .width(180.0)
                .show_ui(ui, |ui| {
                    for r in [LasRole::Incoming, LasRole::Outgoing] {
                        ui.selectable_value(&mut role, r, r.label());
                    }
                });
            if role != e.role {
                e.role = role;
                dirty = true;
            }
        });

        dirty |= field(ui, "File path", &mut e.path, p);
        ui.horizontal(|ui| {
            ui.add_space(LABEL_W + space::ITEM_X);
            if text_btn(ui, "Browse…", false, p).clicked() {
                browse = true;
            }
        });

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = space::ITEM_X;
            ui.add_sized(
                [LABEL_W, 20.0],
                egui::Label::new(RichText::new("Version").font(font_ui(12.0)).color(p.muted)),
            );
            let mut ver = e.version.clone();
            egui::ComboBox::from_id_salt("pb_las_ver")
                .selected_text(&ver)
                .width(100.0)
                .show_ui(ui, |ui| {
                    for v in ["2.0", "1.2", "1.0"] {
                        ui.selectable_value(&mut ver, v.to_string(), v);
                    }
                });
            if ver != e.version {
                e.version = ver;
                dirty = true;
            }
        });

        if e.role == LasRole::Incoming {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = space::ITEM_X;
                ui.add_sized(
                    [LABEL_W, 20.0],
                    egui::Label::new(RichText::new("Follow").font(font_ui(12.0)).color(p.muted)),
                );
                if ui.checkbox(&mut e.follow, "Grow with file").changed() {
                    dirty = true;
                }
            });
        }
    }

    if browse {
        let start = state
            .selected()
            .map(|e| e.path.clone())
            .unwrap_or_default();
        // Native dialog off UI thread (Hyprland).
        let path = std::thread::spawn(move || {
            let mut d = rfd::FileDialog::new()
                .add_filter("LAS", &["las", "LAS"])
                .add_filter("All", &["*"]);
            if !start.is_empty() {
                if let Some(parent) = std::path::Path::new(&start).parent() {
                    d = d.set_directory(parent);
                }
            }
            d.pick_file()
        })
        .join()
        .ok()
        .flatten();
        if let Some(path) = path {
            if let Some(e) = state.selected_mut() {
                e.path = path.display().to_string();
                dirty = true;
            }
        }
    }

    ui.add_space(8.0);
    ui.label(
        RichText::new("Curve binds live on hub Input / Output (LAS) nodes — pack follows later.")
            .font(font_ui(11.0))
            .color(p.muted),
    );
    dirty
}

fn field(ui: &mut egui::Ui, label: &str, value: &mut String, p: &ThemePalette) -> bool {
    let mut dirty = false;
    ui.horizontal(|ui| {
        ui.add_sized(
            [LABEL_W, 20.0],
            egui::Label::new(RichText::new(label).font(font_ui(12.0)).color(p.muted)),
        );
        if ui
            .add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY))
            .changed()
        {
            dirty = true;
        }
    });
    ui.add_space(2.0);
    dirty
}

fn text_btn(ui: &mut egui::Ui, label: &str, selected: bool, p: &ThemePalette) -> egui::Response {
    let fill = if selected { p.accent } else { p.control };
    let text = if selected {
        egui::Color32::from_rgb(0x12, 0x14, 0x18)
    } else {
        p.text
    };
    ui.add(
        egui::Button::new(RichText::new(label).font(font_ui(12.0)).color(text))
            .fill(fill)
            .stroke(egui::Stroke::new(1.0, p.line))
            .sense(Sense::click()),
    )
}
