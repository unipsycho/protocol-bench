//! Saved dock tree, theme, and hub diagram. Protocol I/O is not stored.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::dock::DockPane;
use crate::hubs::HubModel;
use crate::layout::{LayoutNode, TileEdge};
use crate::theme::AppTheme;

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
        }
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

    fn sanitize(&mut self) {
        if !self.tree.contains_pane(DockPane::Hubs) {
            self.tree.band_north(DockPane::Hubs, 0.34);
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
}
