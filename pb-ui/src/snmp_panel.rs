//! SNMP panel — agent/target endpoints for hub Input poll (v2c first). Locked SNMP plan.

use egui::{Layout, RichText, Sense, UiBuilder};

use crate::connection_list::{self, Dot, RowAction};
use pb_engine::{EngineId, HostSnapshot};
use crate::theme::{apply_form_spacing, font_ui, form_inset, space, ThemePalette};

const LABEL_W: f32 = 110.0;

/// One target as the SNMP pack reports it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LiveTarget {
    pub id: String,
    pub endpoint: String,
    pub state: String,
    pub connected: bool,
    pub error: String,
    pub polls: u64,
    pub errors: u64,
}

pub fn parse_live(detail_json: &str) -> Vec<LiveTarget> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(detail_json) else { return Vec::new() };
    let st = |c: &serde_json::Value, k: &str| c.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let n = |c: &serde_json::Value, k: &str| c.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
    v.get("targets")
        .and_then(|c| c.as_array())
        .into_iter()
        .flatten()
        .map(|c| LiveTarget {
            id: st(c, "id"),
            endpoint: st(c, "endpoint"),
            state: st(c, "state"),
            connected: c.get("connected").and_then(|x| x.as_bool()).unwrap_or(false),
            error: st(c, "error"),
            polls: n(c, "polls"),
            errors: n(c, "errors"),
        })
        .collect()
}

/// Live targets from the host snapshot, plus whether the SNMP pack is loaded.
pub fn live_from_host(host: &HostSnapshot) -> (Vec<LiveTarget>, bool) {
    match host.slot(EngineId::Snmp) {
        Some(slot) => (parse_live(&slot.detail_json), slot.pack_loaded),
        None => (Vec::new(), false),
    }
}

/// Dot + text for a target (also used by the hub SNMP Input inspector).
pub fn show_status(ui: &mut egui::Ui, p: &ThemePalette, live: Option<&LiveTarget>, pack_loaded: bool) {
    let (dot, text, err) = if !pack_loaded {
        (Dot::Warn, "SNMP pack not loaded — copy libpb_snmp into the service modules/ folder".to_string(), String::new())
    } else {
        match live {
            None => (Dot::Idle, "Idle — no hub SNMP Input uses this target".to_string(), String::new()),
            Some(l) if l.connected => (Dot::Ok, "Agent answering".to_string(), l.error.clone()),
            Some(l) if l.state == "no response" => (Dot::Err, "No response".to_string(), l.error.clone()),
            Some(l) => (Dot::Warn, l.state.clone(), l.error.clone()),
        }
    };
    connection_list::status_line(ui, p, dot, &text, &err);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnmpVersion {
    V2c,
    V3,
}

impl SnmpVersion {
    pub fn label(self) -> &'static str {
        match self {
            Self::V2c => "v2c",
            Self::V3 => "v3 (later)",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "v3" | "3" => Self::V3,
            _ => Self::V2c,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::V2c => "v2c",
            Self::V3 => "v3",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SnmpTarget {
    pub id: String,
    pub label: String,
    pub host: String,
    pub port: String,
    pub version: SnmpVersion,
    pub community: String,
    pub timeout_secs: String,
}

impl SnmpTarget {
    pub fn default_local() -> Self {
        Self {
            id: "switch".into(),
            label: "Core switch".into(),
            host: "192.168.1.1".into(),
            port: "161".into(),
            version: SnmpVersion::V2c,
            community: "public".into(),
            timeout_secs: "3".into(),
        }
    }

    pub fn summary(&self) -> String {
        format!(
            "{}:{}  {}  ·  community {}",
            if self.host.trim().is_empty() {
                "(no host)"
            } else {
                self.host.as_str()
            },
            if self.port.trim().is_empty() {
                "161"
            } else {
                self.port.as_str()
            },
            self.version.label(),
            if self.community.trim().is_empty() {
                "(empty)"
            } else {
                self.community.as_str()
            }
        )
    }
}

#[derive(Debug, Clone)]
pub struct SnmpPanelState {
    pub targets: Vec<SnmpTarget>,
    pub selected: Option<usize>,
    pub pending_delete: Option<(usize, String)>,
}

impl Default for SnmpPanelState {
    fn default() -> Self {
        Self {
            targets: vec![SnmpTarget::default_local()],
            selected: None,
            pending_delete: None,
        }
    }
}

impl SnmpPanelState {
    pub fn selected(&self) -> Option<&SnmpTarget> {
        self.selected.and_then(|i| self.targets.get(i))
    }

    pub fn selected_mut(&mut self) -> Option<&mut SnmpTarget> {
        self.selected.and_then(|i| self.targets.get_mut(i))
    }

    pub fn connection_choices(&self) -> Vec<(String, String)> {
        self.targets
            .iter()
            .map(|t| {
                let label = if t.label.trim().is_empty() {
                    t.id.clone()
                } else {
                    t.label.clone()
                };
                (t.id.clone(), format!("{} · {}", label, t.host))
            })
            .collect()
    }

    /// Keep the selection in range. An empty list stays empty (and is saved that way).
    fn ensure_targets(&mut self) {
        if self.selected.is_some_and(|i| i >= self.targets.len()) {
            self.selected = None;
        }
    }

    fn unique_id(&self, base: &str) -> String {
        connection_list::unique_id(base, |id| self.targets.iter().any(|t| t.id == id))
    }
}

pub fn paint(ui: &mut egui::Ui, p: &ThemePalette, state: &mut SnmpPanelState, host: &HostSnapshot) -> bool {
    state.ensure_targets();
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

fn paint_body(ui: &mut egui::Ui, p: &ThemePalette, state: &mut SnmpPanelState, host: &HostSnapshot) -> bool {
    apply_form_spacing(ui);
    let (live, pack_loaded) = live_from_host(host);
    let mut dirty = false;
    ui.label(
        RichText::new(
            "SNMP agents for hub Inputs (GET poll). v2c first; OID binds live on hub nodes.",
        )
        .font(font_ui(12.5))
        .color(p.muted),
    );
    ui.add_space(space::AFTER_BLURB);

    if connection_list::heading_with_add(ui, "Targets", "+ Add", p.live, p) {
        let mut t = SnmpTarget::default_local();
        t.id = state.unique_id("snmp");
        t.label = format!("Target {}", state.targets.len() + 1);
        state.targets.push(t);
        state.selected = Some(state.targets.len() - 1);
        dirty = true;
    }
    ui.add_space(space::AFTER_HEADING);
    ui.label(
        RichText::new("Host / community only — map OIDs on hub Input (SNMP) nodes.")
            .font(font_ui(12.0))
            .color(p.muted),
    );
    ui.add_space(space::AFTER_BLURB);

    let mut select = None;
    let show_delete = state.targets.len() > 1;
    for (i, t) in state.targets.iter().enumerate() {
        let selected = state.selected == Some(i);
        let label = if t.label.trim().is_empty() {
            t.id.clone()
        } else {
            t.label.clone()
        };
        let status = match live.iter().find(|l| l.id == t.id) {
            Some(l) if l.connected => "answering".to_string(),
            Some(l) => l.state.clone(),
            None if !pack_loaded => "pack not loaded".to_string(),
            None => "idle".to_string(),
        };
        let subtitle = format!("{}  ·  {status}", t.summary());
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
        if i < state.targets.len() {
            let was = state.selected;
            state.targets.remove(i);
            dirty = true;
            state.selected = match was {
                Some(s) if s == i => None,
                Some(s) if s > i => Some(s - 1),
                other => other,
            };
            state.ensure_targets();
        }
    }

    ui.add_space(space::SECTION);
    if state.selected().is_none() {
        ui.label(
            RichText::new("Select a target to edit.")
                .font(font_ui(12.0))
                .color(p.muted),
        );
        return dirty;
    }

    dirty |= edit_target(ui, state, p, &live, pack_loaded);
    dirty
}

fn edit_target(ui: &mut egui::Ui, state: &mut SnmpPanelState, p: &ThemePalette, live: &[LiveTarget], pack_loaded: bool) -> bool {
    let mut dirty = false;
    let t = state.selected_mut().unwrap();
    ui.label(RichText::new("STATUS").font(font_ui(10.5)).color(p.muted));
    ui.add_space(space::AFTER_HEADING);
    let mine = live.iter().find(|l| l.id == t.id);
    show_status(ui, p, mine, pack_loaded);
    if let Some(l) = mine {
        ui.label(RichText::new(format!("{} polls  ·  {} errors", l.polls, l.errors)).font(font_ui(11.0)).color(p.muted));
    }
    ui.add_space(space::SECTION);
    dirty |= field(ui, "Id", &mut t.id, p);
    dirty |= field(ui, "Label", &mut t.label, p);
    dirty |= field(ui, "Host", &mut t.host, p);
    dirty |= field(ui, "Port", &mut t.port, p);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::ITEM_X;
        ui.add_sized(
            [LABEL_W, 20.0],
            egui::Label::new(RichText::new("Version").font(font_ui(12.0)).color(p.muted)),
        );
        let mut ver = t.version;
        egui::ComboBox::from_id_salt("pb_snmp_ver")
            .selected_text(ver.label())
            .width(140.0)
            .show_ui(ui, |ui| {
                for v in [SnmpVersion::V2c, SnmpVersion::V3] {
                    ui.selectable_value(&mut ver, v, v.label());
                }
            });
        if ver != t.version {
            t.version = ver;
            dirty = true;
        }
    });

    if t.version == SnmpVersion::V2c {
        dirty |= field(ui, "Community", &mut t.community, p);
    } else {
        ui.add_space(4.0);
        ui.label(
            RichText::new("v3 auth/priv fields ship with the pack — panel accepts the version flag now.")
                .font(font_ui(11.0))
                .color(p.muted),
        );
    }
    dirty |= field(ui, "Timeout (s)", &mut t.timeout_secs, p);

    ui.add_space(8.0);
    ui.label(
        RichText::new("OID binds live on hub Input (SNMP) — pack follows later.")
            .font(font_ui(11.0))
            .color(p.muted),
    );
    dirty
}

fn field(ui: &mut egui::Ui, label: &str, value: &mut String, p: &ThemePalette) -> bool {
    connection_list::labeled_field(ui, LABEL_W, label, value, p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_targets_parse_from_pack_json() {
        let j = r#"{"targets":[{"id":"sw","endpoint":"10.0.0.2:161","state":"ok","connected":true,"error":"","polls":4,"errors":1}]}"#;
        let l = parse_live(j);
        assert_eq!(l.len(), 1);
        assert_eq!((l[0].polls, l[0].errors, l[0].connected), (4, 1, true));
        assert!(parse_live("").is_empty());
    }
}
