//! LAS panel — file endpoints for follow (Input) and write (Output). Locked LAS plan.

use egui::{Layout, RichText, Sense, UiBuilder};

use crate::connection_list::{self, text_btn, Dot, RowAction};
use pb_engine::{EngineId, HostSnapshot};
use crate::theme::{apply_form_spacing, font_ui, form_inset, space, ThemePalette};

const LABEL_W: f32 = 110.0;

/// One endpoint as the LAS pack reports it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LiveEp {
    pub id: String,
    pub role: String,
    pub state: String,
    pub connected: bool,
    pub error: String,
    pub rows: u64,
    pub bad_rows: u64,
    pub offset: u64,
    /// Curve mnemonics of the file (incoming), for the curve picker.
    pub curves: Vec<String>,
}

pub fn parse_live(detail_json: &str) -> Vec<LiveEp> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(detail_json) else { return Vec::new() };
    let st = |c: &serde_json::Value, k: &str| c.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let n = |c: &serde_json::Value, k: &str| c.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
    v.get("endpoints")
        .and_then(|c| c.as_array())
        .into_iter()
        .flatten()
        .map(|c| LiveEp {
            id: st(c, "id"),
            role: st(c, "role"),
            state: st(c, "state"),
            connected: c.get("connected").and_then(|x| x.as_bool()).unwrap_or(false),
            error: st(c, "error"),
            rows: n(c, "rows"),
            bad_rows: n(c, "bad_rows"),
            offset: n(c, "offset"),
            curves: c.get("curves").and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default(),
        })
        .collect()
}

/// Live endpoints from the host snapshot, plus whether the LAS pack is loaded.
pub fn live_from_host(host: &HostSnapshot) -> (Vec<LiveEp>, bool) {
    match host.slot(EngineId::Las) {
        Some(slot) => (parse_live(&slot.detail_json), slot.pack_loaded),
        None => (Vec::new(), false),
    }
}

/// Dot + text for an endpoint (also used by the hub LAS Input / Output inspector).
pub fn show_status(ui: &mut egui::Ui, p: &ThemePalette, live: Option<&LiveEp>, pack_loaded: bool) {
    let (dot, text, err) = if !pack_loaded {
        (Dot::Warn, "LAS pack not loaded — copy libpb_las into the service modules/ folder".to_string(), String::new())
    } else {
        match live {
            None => (Dot::Idle, "Idle — no hub LAS node binds a curve from this endpoint".to_string(), String::new()),
            Some(l) if l.connected => (Dot::Ok, l.state.clone(), l.error.clone()),
            Some(l) if l.state == "error" => (Dot::Err, "Error".to_string(), l.error.clone()),
            Some(l) => (Dot::Warn, l.state.clone(), l.error.clone()),
        }
    };
    connection_list::status_line(ui, p, dot, &text, &err);
}

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
        connection_list::unique_id(base, |id| self.endpoints.iter().any(|e| e.id == id))
    }
}

pub fn paint(ui: &mut egui::Ui, p: &ThemePalette, state: &mut LasPanelState, host: &HostSnapshot) -> bool {
    state.ensure_endpoints();
    let mut dirty = false;
    let full = ui.available_rect_before_wrap();
    let inner = form_inset(full);
    ui.allocate_new_ui(
        UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::top_down(egui::Align::Min)),
        |ui| {
            dirty = paint_body(ui, p, state, host);
        },
    );
    ui.allocate_rect(full, Sense::hover());
    dirty
}

fn paint_body(ui: &mut egui::Ui, p: &ThemePalette, state: &mut LasPanelState, host: &HostSnapshot) -> bool {
    apply_form_spacing(ui);
    let (live, pack_loaded) = live_from_host(host);
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
        let status = match live.iter().find(|l| l.id == ep.id) {
            Some(l) => l.state.clone(),
            None if !pack_loaded => "pack not loaded".to_string(),
            None => "idle".to_string(),
        };
        let subtitle = format!("{}  ·  {status}", ep.summary());
        match connection_list::selectable_row(ui, &label, &subtitle, selected, show_delete, p) {
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

    dirty |= edit_endpoint(ui, state, p, &live, pack_loaded);
    dirty
}

fn edit_endpoint(ui: &mut egui::Ui, state: &mut LasPanelState, p: &ThemePalette, live: &[LiveEp], pack_loaded: bool) -> bool {
    let mut dirty = false;
    let mut browse = false;
    {
        let id = state.selected().map(|e| e.id.clone()).unwrap_or_default();
        ui.label(RichText::new("STATUS").font(font_ui(10.5)).color(p.muted));
        ui.add_space(space::AFTER_HEADING);
        let mine = live.iter().find(|l| l.id == id);
        show_status(ui, p, mine, pack_loaded);
        if let Some(l) = mine {
            let mut bits = vec![format!("{} rows", l.rows)];
            if l.role != "outgoing" {
                bits.push(format!("offset {} B", l.offset));
            }
            if l.bad_rows > 0 {
                bits.push(format!("{} bad", l.bad_rows));
            }
            if !l.curves.is_empty() {
                bits.push(format!("curves: {}", l.curves.join(", ")));
            }
            ui.label(RichText::new(bits.join("  ·  ")).font(font_ui(11.0)).color(p.muted));
        }
        ui.add_space(space::SECTION);
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_endpoints_parse_with_curves() {
        let j = r#"{"endpoints":[{"id":"w","path":"/a.las","role":"incoming","state":"following","connected":true,"error":"","rows":9,"bad_rows":0,"offset":120,"curves":["DEPT","GR"]}]}"#;
        let l = parse_live(j);
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].curves, vec!["DEPT", "GR"]);
        assert_eq!((l[0].rows, l[0].offset), (9, 120));
        assert!(parse_live("nope").is_empty());
    }
}
