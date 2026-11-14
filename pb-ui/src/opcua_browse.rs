//! Shared OPC UA browse tree (panel preview + Hub Input pick).

use std::collections::BTreeMap;

use egui::{RichText, Sense};

use crate::theme::{font_ui, ThemePalette};

#[derive(Debug, Clone, Default)]
pub struct BrowseNode {
    pub node_id: String,
    pub browse_name: String,
    pub display_name: String,
    pub is_variable: bool,
    pub children: BTreeMap<String, BrowseNode>,
}

#[derive(Debug, Clone, Default)]
pub struct BrowseTree {
    pub roots: Vec<BrowseNode>,
}

impl BrowseTree {
    pub fn clear(&mut self) {
        self.roots.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }

    /// Replace top-level from pack browse snapshot rows.
    pub fn sync_from_rows(&mut self, rows: &[BrowseNode]) {
        self.roots = rows.to_vec();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrowseMode {
    Preview,
    PickNode,
}

#[derive(Clone, Debug, Default)]
pub struct BrowsePick {
    pub node_id: String,
    pub display_name: String,
}

/// Draw browse rows. Returns a pick when mode is PickNode and a variable is clicked.
pub fn paint(
    ui: &mut egui::Ui,
    tree: &BrowseTree,
    id_salt: &str,
    mode: BrowseMode,
    p: &ThemePalette,
) -> Option<BrowsePick> {
    if tree.is_empty() {
        ui.label(
            RichText::new("No browse results yet — Connect, then Browse Objects.")
                .font(font_ui(12.0))
                .color(p.muted),
        );
        return None;
    }
    let mut pick = None;
    egui::ScrollArea::vertical()
        .id_salt(id_salt)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (i, node) in tree.roots.iter().enumerate() {
                if let Some(picks) = paint_node(ui, node, i, 0, mode, p) {
                    pick = Some(picks);
                }
            }
        });
    pick
}

fn paint_node(
    ui: &mut egui::Ui,
    node: &BrowseNode,
    idx: usize,
    depth: usize,
    mode: BrowseMode,
    p: &ThemePalette,
) -> Option<BrowsePick> {
    let mut pick = None;
    let indent = 12.0 * depth as f32;
    ui.horizontal(|ui| {
        ui.add_space(indent);
        let kind = if node.is_variable { "VAR" } else { "OBJ" };
        let label = if node.display_name.trim().is_empty() {
            node.browse_name.as_str()
        } else {
            node.display_name.as_str()
        };
        let text = format!("[{kind}] {label}");
        let resp = ui
            .add(
                egui::Label::new(RichText::new(text).font(font_ui(12.0)).color(p.text))
                    .sense(Sense::click()),
            )
            .on_hover_text(&node.node_id);
        if mode == BrowseMode::PickNode && node.is_variable && resp.clicked() {
            pick = Some(BrowsePick {
                node_id: node.node_id.clone(),
                display_name: label.to_string(),
            });
        }
        let _ = idx;
    });
    for (i, child) in node.children.values().enumerate() {
        if let Some(picks) = paint_node(ui, child, i, depth + 1, mode, p) {
            pick = Some(picks);
        }
    }
    pick
}
