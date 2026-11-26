//! App settings: theme, auto-run hubs, and a dock/hubs mirror in `ui.toml`.
//! Authoritative dock layout for a project lives in `project.json` (PB-95).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::dock::DockPane;
use crate::hubs::HubModel;
use crate::layout::{LayoutNode, TileEdge};
use crate::theme::AppTheme;

/// Cap for File → Recent projects (MRU, stored in `ui.toml`).
pub const MAX_RECENT_PROJECTS: usize = 15;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiState {
    #[serde(default)]
    pub tree: LayoutNode,
    #[serde(default = "default_theme")]
    pub theme_id: String,
    #[serde(default)]
    pub maximized: Option<DockPane>,
    /// Panes toggled off from the rail. They stay in `tree` so showing them restores the place.
    #[serde(default)]
    pub hidden_panes: Vec<DockPane>,
    /// Conversion hubs drawn in the Hubs panel. Missing files get the sample diagram.
    #[serde(default)]
    pub hubs: HubModel,
    /// App-level: start MQTT + OPC UA embedded hubs on load when configured (PB-58).
    #[serde(default)]
    pub auto_run_embedded_hubs: bool,
    /// Most-recently-used project paths (absolute when possible), newest first.
    #[serde(default)]
    pub recent_projects: Vec<String>,
    /// Protocol the Protocols pane shows (a protocol pane id).
    #[serde(default)]
    pub protocols_selected: Option<DockPane>,
}

fn default_theme() -> String {
    "copper".into()
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            tree: LayoutNode::default_tree(),
            theme_id: default_theme(),
            maximized: None,
            hidden_panes: Vec::new(),
            hubs: HubModel::sample(),
            auto_run_embedded_hubs: false,
            recent_projects: Vec::new(),
            protocols_selected: None,
        }
    }
}

/// Push `path` to the front of recent projects (dedupe, cap at [`MAX_RECENT_PROJECTS`]).
/// Returns true when the list changed.
pub fn note_recent_project(state: &mut UiState, path: &Path) -> bool {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let s = path.display().to_string();
    if s.trim().is_empty() {
        return false;
    }
    let before = state.recent_projects.clone();
    state.recent_projects.retain(|p| p != &s);
    state.recent_projects.insert(0, s);
    if state.recent_projects.len() > MAX_RECENT_PROJECTS {
        state.recent_projects.truncate(MAX_RECENT_PROJECTS);
    }
    state.recent_projects != before
}

/// Drop a missing / unwanted recent entry.
pub fn remove_recent_project(state: &mut UiState, path: &Path) -> bool {
    let s = path.display().to_string();
    let canon = std::fs::canonicalize(path)
        .ok()
        .map(|p| p.display().to_string());
    let n = state.recent_projects.len();
    state.recent_projects.retain(|p| {
        if p == &s {
            return false;
        }
        if let Some(c) = canon.as_ref() {
            if p == c {
                return false;
            }
        }
        true
    });
    state.recent_projects.len() != n
}

/// Is this JSON file a Protocol Bench project (not `ipc.json`, a sidecar or some other JSON)?
/// Cheap structural check: an object with a numeric `version` and a `hubs` or `mqtt` section.
pub fn looks_like_project(path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return false;
    };
    value.get("version").map_or(false, |v| v.is_number())
        && (value.get("hubs").is_some() || value.get("mqtt").is_some())
}

/// Recent projects that still exist on disk and are projects (old entries such as `ipc.json`
/// are skipped), newest first. The stored strings are returned unchanged so the menu can open
/// them exactly as recorded.
pub fn existing_recent_projects(state: &UiState) -> Vec<PathBuf> {
    state
        .recent_projects
        .iter()
        .map(PathBuf::from)
        .filter(|p| p.is_file() && looks_like_project(p))
        .collect()
}

/// Forget every recent project (File → Open recent → Clear). Returns true when the list changed.
pub fn clear_recent_projects(state: &mut UiState) -> bool {
    let changed = !state.recent_projects.is_empty();
    state.recent_projects.clear();
    changed
}

/// Folder the Open / Save as dialogs should start in: the current project's folder, unless
/// that is the service's own config folder (the default project lives there and the sample
/// projects do not), in which case the folder of the newest recent project elsewhere.
pub fn dialog_start_dir(state: &UiState, current: &Path, config_dir: Option<&Path>) -> Option<PathBuf> {
    let is_config = |d: &Path| config_dir.map_or(false, |c| c == d);
    let current_dir = current.parent().filter(|d| !d.as_os_str().is_empty());
    if let Some(dir) = current_dir.filter(|d| !is_config(d)) {
        return Some(dir.to_path_buf());
    }
    state
        .recent_projects
        .iter()
        .filter_map(|r| Path::new(r).parent().map(Path::to_path_buf))
        .find(|d| !is_config(d) && d.is_dir())
        .or_else(|| current_dir.map(Path::to_path_buf))
}

impl UiState {
    pub fn pane_visible(&self, pane: DockPane) -> bool {
        self.tree.contains_pane(pane) && !self.hidden_panes.iter().any(|item| *item == pane)
    }

    /// Show a pane, inserting it beside Project if a saved tree somehow dropped it.
    pub fn reveal_pane(&mut self, pane: DockPane) {
        self.tree
            .ensure_pane(pane, DockPane::Project, TileEdge::East);
        self.hidden_panes.retain(|item| *item != pane);
    }

    /// Tab / header text. The Protocols pane names the protocol it is showing.
    pub fn pane_label(&self, pane: DockPane) -> String {
        match (pane, self.protocols_selected) {
            (DockPane::Protocols, Some(sel)) => format!("Protocols \u{00B7} {}", sel.title()),
            _ => pane.title().to_string(),
        }
    }

    /// Protocol panes that have their own tile (popped out of Protocols).
    pub fn popped_protocols(&self) -> Vec<DockPane> {
        DockPane::PROTOCOLS
            .into_iter()
            .filter(|pane| self.pane_visible(*pane))
            .collect()
    }

    /// Bring a pane forward. A protocol goes to its own tile when popped out, else it is
    /// picked in the Protocols pane.
    pub fn open_pane(&mut self, pane: DockPane) -> DockPane {
        let target = if pane.is_protocol() && !self.pane_visible(pane) {
            self.protocols_selected = Some(pane);
            DockPane::Protocols
        } else {
            pane
        };
        self.reveal_pane(target);
        self.tree.set_active(target);
        if self.maximized.is_some() {
            self.maximized = Some(target);
        }
        target
    }

    /// Give a protocol its own tile beside the Protocols pane.
    pub fn pop_out_protocol(&mut self, pane: DockPane) {
        if !pane.is_protocol() {
            return;
        }
        let beside = if self.pane_visible(DockPane::Protocols) {
            DockPane::Protocols
        } else {
            DockPane::Project
        };
        self.tree.ensure_pane(pane, beside, TileEdge::East);
        self.hidden_panes.retain(|item| *item != pane);
        if self.protocols_selected == Some(pane) {
            // The Protocols pane moves on to another protocol.
            self.protocols_selected = None;
        }
        if self.maximized.is_some() {
            self.maximized = None;
        }
    }

    /// Close a popped-out protocol's tile and show it in the Protocols pane again.
    pub fn dock_back_protocol(&mut self, pane: DockPane) {
        if !pane.is_protocol() {
            return;
        }
        if !self.tree.remove_leaf(pane) {
            // Lone root tab: keep the tree non-empty by putting Protocols there first.
            self.tree.replace_pane(pane, DockPane::Protocols);
        }
        self.hidden_panes.retain(|item| *item != pane);
        if self.maximized == Some(pane) {
            self.maximized = None;
        }
        self.open_pane(pane);
    }

    /// Older layouts held one tile per protocol. Fold them into a single Protocols pane where
    /// the first one sat, keeping the protocol that was on top as the pick.
    fn migrate_protocol_panes(&mut self) {
        if self.tree.contains_pane(DockPane::Protocols) {
            return;
        }
        let mut panes = Vec::new();
        self.tree.collect_panes(&mut panes);
        let protos: Vec<DockPane> = panes.into_iter().filter(|pane| pane.is_protocol()).collect();
        let Some(&first) = protos.first() else {
            return;
        };
        let on_top = self
            .tree
            .group_of(first)
            .map(|group| group.active_pane())
            .filter(|pane| pane.is_protocol() && !self.hidden_panes.contains(pane));
        let was_active = on_top.is_some();
        self.tree.replace_pane(first, DockPane::Protocols);
        for pane in protos.iter().skip(1) {
            let _ = self.tree.remove_leaf(*pane);
        }
        if self.maximized.is_some_and(|pane| pane.is_protocol()) {
            self.maximized = Some(DockPane::Protocols);
        }
        if was_active {
            self.tree.set_active(DockPane::Protocols);
        }
        if self.protocols_selected.is_none() {
            self.protocols_selected = on_top;
        }
    }

    /// Hide or show a rail pane. Refuses to hide the last visible one.
    pub fn toggle_pane_visible(&mut self, pane: DockPane) -> bool {
        if self.pane_visible(pane) {
            let visible = DockPane::ALL
                .iter()
                .copied()
                .filter(|item| self.pane_visible(*item))
                .count();
            if visible <= 1 {
                return false;
            }
            self.hidden_panes.push(pane);
            if self.maximized == Some(pane) {
                self.maximized = None;
            }
            return true;
        }
        self.reveal_pane(pane);
        true
    }

    /// Ensure every pane exists in the tree; drop retired Calculations leaf.
    pub fn sanitize(&mut self) {
        if !self.tree.contains_pane(DockPane::Hubs) {
            self.tree.band_north(DockPane::Hubs, 0.34);
        }
        // Calculations moved into hub node properties — drop any leftover dock leaf.
        let _ = self.tree.remove_leaf(DockPane::Calculations);
        self.hidden_panes
            .retain(|pane| *pane != DockPane::Calculations);
        if self.maximized == Some(DockPane::Calculations) {
            self.maximized = None;
        }
        self.migrate_protocol_panes();
        // Protocol tiles are never hidden, only docked back: drop hidden ones from the tree.
        let hidden_protos: Vec<DockPane> = self
            .hidden_panes
            .iter()
            .copied()
            .filter(|pane| pane.is_protocol())
            .collect();
        for pane in hidden_protos {
            let _ = self.tree.remove_leaf(pane);
        }
        self.hidden_panes.retain(|pane| !pane.is_protocol());
        if self.protocols_selected.is_some_and(|pane| !pane.is_protocol()) {
            self.protocols_selected = None;
        }
        for pane in DockPane::ALL {
            self.tree
                .ensure_pane(pane, DockPane::Project, TileEdge::East);
        }
        self.hidden_panes.sort_by_key(|pane| pane.id());
        self.hidden_panes.dedup();
        if let Some(max) = self.maximized {
            if !self.pane_visible(max) {
                self.maximized = None;
            }
        }
        self.hubs.sanitize();
    }
}

pub fn config_path() -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("ca", "AgileAutomation", "ProtocolBench")?;
    Some(dirs.config_dir().join("ui.toml"))
}

pub fn load() -> UiState {
    let Some(path) = config_path() else {
        return UiState::default();
    };
    let mut state = match std::fs::read_to_string(&path) {
        Ok(text) => toml::from_str(&text).unwrap_or_default(),
        Err(_) => UiState::default(),
    };
    state.sanitize();
    state
}

pub fn save(state: &UiState) -> Result<(), String> {
    let path = config_path().ok_or_else(|| "no config directory".to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let text = toml::to_string_pretty(state).map_err(|err| err.to_string())?;
    std::fs::write(path, text).map_err(|err| err.to_string())
}

pub fn theme_from_state(state: &UiState) -> AppTheme {
    AppTheme::from_id(&state.theme_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_toml_roundtrip() {
        let state = UiState::default();
        let text = toml::to_string_pretty(&state).expect("toml");
        let back: UiState = toml::from_str(&text).expect("parse");
        assert!(back.tree.contains_pane(DockPane::Protocols));
        assert!(!back.tree.contains_pane(DockPane::Modbus));
        assert_eq!(back.theme_id, "copper");
    }

    /// An old ui.toml tree with one tab per protocol (Modbus on top).
    fn legacy_state() -> UiState {
        use crate::layout::TabGroup;
        let mut state = UiState::default();
        state.tree = LayoutNode::Split {
            dir: crate::layout::SplitDir::Row,
            ratio: 0.4,
            first: Box::new(LayoutNode::Leaf(TabGroup::single(DockPane::Project))),
            second: Box::new(LayoutNode::Split {
                dir: crate::layout::SplitDir::Col,
                ratio: 0.5,
                first: Box::new(LayoutNode::Leaf(TabGroup {
                    tabs: vec![DockPane::Streams, DockPane::Modbus, DockPane::Mqtt],
                    active: 1,
                })),
                second: Box::new(LayoutNode::Leaf(TabGroup::single(DockPane::OpcUa))),
            }),
        };
        state.hidden_panes = vec![DockPane::Mqtt];
        state
    }

    #[test]
    fn legacy_protocol_tabs_fold_into_one_protocols_pane() {
        let mut state = legacy_state();
        state.sanitize();
        let mut panes = Vec::new();
        state.tree.collect_panes(&mut panes);
        assert!(panes.iter().all(|pane| !pane.is_protocol()), "{panes:?}");
        assert_eq!(panes.iter().filter(|p| **p == DockPane::Protocols).count(), 1);
        // Protocols took the first protocol tab's place and is on top there.
        let group = state.tree.group_of(DockPane::Protocols).unwrap();
        assert_eq!(group.active_pane(), DockPane::Protocols);
        assert_eq!(state.protocols_selected, Some(DockPane::Modbus));
        assert!(state.hidden_panes.iter().all(|pane| !pane.is_protocol()));
        for pane in DockPane::ALL {
            assert!(state.tree.contains_pane(pane), "{pane:?}");
        }
        // Runs once: a second pass changes nothing.
        let before = state.tree.clone();
        state.sanitize();
        assert_eq!(state.tree, before);
    }

    #[test]
    fn pop_out_and_dock_back() {
        let mut state = UiState::default();
        state.sanitize();
        state.protocols_selected = Some(DockPane::Modbus);
        state.pop_out_protocol(DockPane::Modbus);
        assert!(state.pane_visible(DockPane::Modbus));
        assert_eq!(state.popped_protocols(), vec![DockPane::Modbus]);
        assert_eq!(state.protocols_selected, None);
        // Popped tiles survive sanitize (they are not migrated away).
        state.sanitize();
        assert!(state.pane_visible(DockPane::Modbus));
        // Opening a popped protocol focuses its tile; an unpopped one is picked in Protocols.
        assert_eq!(state.open_pane(DockPane::Modbus), DockPane::Modbus);
        assert_eq!(state.open_pane(DockPane::Mqtt), DockPane::Protocols);
        assert_eq!(state.protocols_selected, Some(DockPane::Mqtt));
        state.dock_back_protocol(DockPane::Modbus);
        assert!(!state.tree.contains_pane(DockPane::Modbus));
        assert_eq!(state.protocols_selected, Some(DockPane::Modbus));
        assert!(state.popped_protocols().is_empty());
        assert_eq!(state.pane_label(DockPane::Protocols), "Protocols \u{00B7} Modbus");
    }

    #[test]
    fn rail_never_lists_protocol_panes() {
        assert!(DockPane::ALL.iter().all(|pane| !pane.is_protocol()));
        assert!(DockPane::PROTOCOLS.iter().all(|pane| pane.is_protocol()));
    }

    #[test]
    fn cannot_hide_the_last_pane() {
        let mut state = UiState::default();
        for pane in DockPane::ALL.iter().copied().take(DockPane::ALL.len() - 1) {
            assert!(state.toggle_pane_visible(pane));
        }
        let last = DockPane::ALL[DockPane::ALL.len() - 1];
        assert!(state.pane_visible(last));
        assert!(!state.toggle_pane_visible(last));
        assert!(state.pane_visible(last));
    }

    #[test]
    fn existing_recent_projects_skip_missing_and_clear_empties() {
        let dir = std::env::temp_dir().join(format!("pb-recent-exist-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let kept = dir.join("kept.json");
        std::fs::write(&kept, r#"{"version":10,"hubs":{}}"#).unwrap();
        let ipc = dir.join("ipc.json");
        std::fs::write(&ipc, r#"{"host":"127.0.0.1","port":1}"#).unwrap();
        let mut state = UiState::default();
        note_recent_project(&mut state, &dir.join("gone.json"));
        note_recent_project(&mut state, &ipc);
        note_recent_project(&mut state, &kept);
        let got = existing_recent_projects(&state);
        assert_eq!(got.len(), 1, "missing files and ipc.json are skipped");
        assert!(got[0].ends_with("kept.json"));
        assert!(clear_recent_projects(&mut state));
        assert!(state.recent_projects.is_empty());
        assert!(!clear_recent_projects(&mut state));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn dialog_skips_the_config_folder() {
        let mut state = UiState::default();
        state.recent_projects = vec!["/tmp/work/x.json".into(), "/cfg/project.json".into()];
        let cfg = Path::new("/cfg");
        // current project is the default one in the config folder -> use the recent elsewhere
        let got = dialog_start_dir(&state, Path::new("/cfg/project.json"), Some(cfg));
        // /tmp/work does not exist here, so only the fallback (current dir) is possible
        assert_eq!(got, Some(PathBuf::from("/cfg")));
    }

    #[test]
    fn recent_projects_mru_dedupe_and_cap() {
        let mut state = UiState::default();
        for i in 0..20 {
            assert!(note_recent_project(
                &mut state,
                Path::new(&format!("/tmp/pb-recent-{i}.json"))
            ));
        }
        assert_eq!(state.recent_projects.len(), MAX_RECENT_PROJECTS);
        assert!(state.recent_projects[0].ends_with("pb-recent-19.json"));
        // Re-open an older entry — moves to front, no growth.
        let older = state.recent_projects[5].clone();
        assert!(note_recent_project(&mut state, Path::new(&older)));
        assert_eq!(state.recent_projects[0], older);
        assert_eq!(state.recent_projects.len(), MAX_RECENT_PROJECTS);
        assert!(remove_recent_project(&mut state, Path::new(&older)));
        assert!(!state.recent_projects.iter().any(|p| p == &older));
    }
}
