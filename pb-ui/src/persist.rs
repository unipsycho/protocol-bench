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

/// Compact menu label: `name — parent` (full path stays on hover).
pub fn recent_project_label(path: &str) -> String {
    let p = Path::new(path);
    match (
        p.file_name().and_then(|s| s.to_str()),
        p.parent().map(|parent| parent.display().to_string()),
    ) {
        (Some(name), Some(parent)) if !parent.is_empty() => format!("{name} — {parent}"),
        (Some(name), _) => name.to_string(),
        _ => path.to_string(),
    }
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
        assert!(back.tree.contains_pane(DockPane::Modbus));
        assert_eq!(back.theme_id, "copper");
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
