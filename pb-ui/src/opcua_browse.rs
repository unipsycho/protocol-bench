//! Shared OPC UA browse tree (panel preview + Hub Input pick).
//!
//! Hierarchical Objects → folders → variables, MQTT topic-tree style (PB-89).
//! Lazy-expand: opening a folder with no loaded children requests a browse.

use std::collections::BTreeMap;

use egui::{RichText, Rounding, Sense, Vec2};

use crate::theme::{font_ui, space, tint, ThemePalette};

#[derive(Debug, Clone, Default)]
pub struct BrowseNode {
    pub node_id: String,
    pub browse_name: String,
    pub display_name: String,
    pub is_variable: bool,
    pub children: BTreeMap<String, BrowseNode>,
    /// True after a browse of this node returned (even if empty).
    pub children_loaded: bool,
}

impl BrowseNode {
    fn label(&self) -> &str {
        if self.display_name.trim().is_empty() {
            self.browse_name.as_str()
        } else {
            self.display_name.as_str()
        }
    }

    fn from_flat(row: &FlatBrowseRow) -> Self {
        Self {
            node_id: row.node_id.clone(),
            browse_name: row.browse_name.clone(),
            display_name: row.display_name.clone(),
            is_variable: row.is_variable,
            children: BTreeMap::new(),
            children_loaded: false,
        }
    }
}

/// One row from pack `detail_json` browse map values.
#[derive(Debug, Clone, Default)]
pub struct FlatBrowseRow {
    pub node_id: String,
    pub browse_name: String,
    pub display_name: String,
    pub is_variable: bool,
}

#[derive(Debug, Clone)]
pub struct BrowseTree {
    /// Synthetic Objects root — children are the Objects-folder browse.
    pub objects: BrowseNode,
}

impl Default for BrowseTree {
    fn default() -> Self {
        Self {
            objects: BrowseNode {
                node_id: "Objects".into(),
                browse_name: "Objects".into(),
                display_name: "Objects".into(),
                is_variable: false,
                children: BTreeMap::new(),
                children_loaded: false,
            },
        }
    }
}

impl BrowseTree {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn is_empty(&self) -> bool {
        !self.objects.children_loaded && self.objects.children.is_empty()
    }

    /// Merge children under `parent_id` (`"Objects"` or a NodeId). Preserves
    /// already-loaded descendants when the same child node_id reappears.
    pub fn merge_children(&mut self, parent_id: &str, rows: &[FlatBrowseRow]) {
        let parent_key = normalize_parent(parent_id);
        if parent_key == "Objects" {
            self.objects.children = merge_child_map(&self.objects.children, rows);
            self.objects.children_loaded = true;
            return;
        }
        if let Some(node) = find_mut(&mut self.objects, &parent_key) {
            node.children = merge_child_map(&node.children, rows);
            node.children_loaded = true;
        } else {
            // Orphan batch — attach under Objects so it still shows (rare).
            let mut orphan = BrowseNode {
                node_id: parent_key.clone(),
                browse_name: parent_key.clone(),
                display_name: parent_key.clone(),
                is_variable: false,
                children: BTreeMap::new(),
                children_loaded: true,
            };
            orphan.children = merge_child_map(&BTreeMap::new(), rows);
            self.objects
                .children
                .entry(parent_key)
                .and_modify(|n| {
                    n.children = merge_child_map(&n.children, rows);
                    n.children_loaded = true;
                })
                .or_insert(orphan);
            self.objects.children_loaded = true;
        }
    }

    /// Apply a full parent→rows map from pack snapshot.
    pub fn sync_from_parent_map(&mut self, by_parent: &BTreeMap<String, Vec<FlatBrowseRow>>) {
        if by_parent.is_empty() {
            return;
        }
        // Objects first so folders exist before deeper merges.
        if let Some(rows) = by_parent.get("Objects") {
            self.merge_children("Objects", rows);
        }
        for (parent, rows) in by_parent {
            if parent == "Objects" {
                continue;
            }
            self.merge_children(parent, rows);
        }
    }
}

fn normalize_parent(id: &str) -> String {
    let t = id.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("objects") || t == "i=85" {
        "Objects".into()
    } else {
        t.to_string()
    }
}

fn merge_child_map(
    prior: &BTreeMap<String, BrowseNode>,
    rows: &[FlatBrowseRow],
) -> BTreeMap<String, BrowseNode> {
    let mut out = BTreeMap::new();
    for row in rows {
        let key = if row.node_id.is_empty() {
            row.browse_name.clone()
        } else {
            row.node_id.clone()
        };
        if key.is_empty() {
            continue;
        }
        let mut next = BrowseNode::from_flat(row);
        if let Some(old) = prior.get(&key) {
            next.children = old.children.clone();
            next.children_loaded = old.children_loaded;
        }
        out.insert(key, next);
    }
    out
}

fn find_mut<'a>(node: &'a mut BrowseNode, id: &str) -> Option<&'a mut BrowseNode> {
    if node.node_id == id {
        return Some(node);
    }
    // Path of map keys from this node down to `id`.
    let mut path = Vec::new();
    if !find_path(node, id, &mut path) {
        return None;
    }
    let mut cur = node;
    for key in path {
        cur = cur.children.get_mut(&key)?;
    }
    Some(cur)
}

fn find_path(node: &BrowseNode, id: &str, path: &mut Vec<String>) -> bool {
    for (key, child) in &node.children {
        path.push(key.clone());
        if child.node_id == id || find_path(child, id, path) {
            return true;
        }
        path.pop();
    }
    false
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrowseMode {
    /// Expand / browse only.
    Preview,
    /// Click a Variable → select NodeId.
    PickNode,
}

#[derive(Clone, Debug, Default)]
pub struct BrowsePick {
    pub node_id: String,
    pub display_name: String,
}

/// Result of painting the tree for one frame.
#[derive(Clone, Debug)]
pub enum BrowseAction {
    /// Variable clicked (PickNode mode).
    Pick(BrowsePick),
    /// Folder opened that still needs a server browse.
    Expand(String),
}

/// Draw the hierarchical browse tree. Returns pick and/or expand requests.
pub fn paint(
    ui: &mut egui::Ui,
    tree: &BrowseTree,
    id_salt: &str,
    mode: BrowseMode,
    p: &ThemePalette,
) -> Option<BrowseAction> {
    if tree.is_empty() {
        ui.label(
            RichText::new("No browse results yet — Connect, then Browse Objects.")
                .font(font_ui(12.0))
                .color(p.muted),
        );
        return None;
    }

    let mut action = None;
    let prev_vline = ui.visuals().indent_has_left_vline;
    ui.visuals_mut().indent_has_left_vline = false;
    let prev_y = ui.spacing().item_spacing.y;
    let prev_interact_y = ui.spacing().interact_size.y;
    ui.spacing_mut().item_spacing.y = 2.0;
    ui.spacing_mut().interact_size.y = 18.0;

    egui::ScrollArea::vertical()
        .id_salt(id_salt)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            paint_folder(ui, &tree.objects, id_salt, mode, p, true, &mut action);
        });

    ui.visuals_mut().indent_has_left_vline = prev_vline;
    ui.spacing_mut().item_spacing.y = prev_y;
    ui.spacing_mut().interact_size.y = prev_interact_y;
    action
}

fn paint_folder(
    ui: &mut egui::Ui,
    node: &BrowseNode,
    id_salt: &str,
    mode: BrowseMode,
    p: &ThemePalette,
    default_open: bool,
    action: &mut Option<BrowseAction>,
) {
    let id = ui.make_persistent_id(("pb_opcua_br", id_salt, &node.node_id));
    let req_id = ui.make_persistent_id(("pb_ua_br_req", id_salt, &node.node_id));
    if node.children_loaded {
        ui.ctx().data_mut(|d| d.remove_temp::<bool>(req_id));
    }
    egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, default_open)
        .show_header(ui, |ui| {
            let label = node.label();
            let kind = if node.is_variable { "VAR" } else { "OBJ" };
            ui.label(
                RichText::new(format!("[{kind}] {label}"))
                    .font(font_ui(12.5))
                    .color(p.text),
            );
            ui.add_space(space::BUTTON);
            ui.label(
                RichText::new(&node.node_id)
                    .font(font_ui(11.0))
                    .color(p.muted),
            );
        })
        .body(|ui| {
            // Body only runs while open — one browse until children land (PB-89).
            if !node.children_loaded && !node.is_variable && action.is_none() {
                let already = ui
                    .ctx()
                    .data(|d| d.get_temp::<bool>(req_id))
                    .unwrap_or(false);
                if !already {
                    ui.ctx().data_mut(|d| d.insert_temp(req_id, true));
                    *action = Some(BrowseAction::Expand(node.node_id.clone()));
                }
            }
            if !node.children_loaded && node.children.is_empty() {
                ui.label(
                    RichText::new("Loading…")
                        .font(font_ui(11.5))
                        .color(p.muted),
                );
                return;
            }
            if node.children.is_empty() {
                ui.label(
                    RichText::new("(empty)")
                        .font(font_ui(11.5))
                        .color(p.muted),
                );
                return;
            }
            for child in node.children.values() {
                if child.is_variable && child.children.is_empty() {
                    paint_variable_row(ui, child, mode, p, action);
                } else {
                    // Folders / objects with possible children — nested collapse.
                    paint_folder(ui, child, id_salt, mode, p, false, action);
                }
            }
        });
}

fn paint_variable_row(
    ui: &mut egui::Ui,
    node: &BrowseNode,
    mode: BrowseMode,
    p: &ThemePalette,
    action: &mut Option<BrowseAction>,
) {
    let row_h = ui.spacing().interact_size.y.max(18.0);
    let w = ui.available_width().max(1.0);
    let pickable = mode == BrowseMode::PickNode;
    let sense = if pickable {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (row, resp) = ui.allocate_exact_size(Vec2::new(w, row_h), sense);
    if resp.hovered() {
        ui.painter()
            .rect_filled(row, Rounding::same(4.0), tint(p.accent, 16));
        if pickable {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }
    let label = node.label();
    let text = format!("[VAR] {label}");
    let galley = ui.fonts(|f| f.layout_no_wrap(text, font_ui(12.5), p.text));
    let label_w = galley.size().x;
    let pos = egui::pos2(row.min.x + 18.0, row.center().y - galley.size().y * 0.5);
    ui.painter().galley(pos, galley, p.text);
    let nid_galley = ui.fonts(|f| {
        f.layout_no_wrap(node.node_id.clone(), font_ui(11.0), p.muted)
    });
    let nid_x = pos.x + label_w + 10.0;
    ui.painter().galley(
        egui::pos2(nid_x, row.center().y - nid_galley.size().y * 0.5),
        nid_galley,
        p.muted,
    );
    if pickable {
        resp.clone().on_hover_text(format!(
            "Click to select NodeId\n{}",
            node.node_id
        ));
    } else {
        resp.clone().on_hover_text(&node.node_id);
    }
    if pickable && resp.clicked() && action.is_none() {
        *action = Some(BrowseAction::Pick(BrowsePick {
            node_id: node.node_id.clone(),
            display_name: label.to_string(),
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_objects_then_lazy_child() {
        let mut tree = BrowseTree::default();
        assert!(tree.is_empty());
        tree.merge_children(
            "Objects",
            &[FlatBrowseRow {
                node_id: "ns=2;s=Points".into(),
                browse_name: "Points".into(),
                display_name: "Points".into(),
                is_variable: false,
            }],
        );
        assert!(!tree.is_empty());
        assert!(tree.objects.children_loaded);
        assert_eq!(tree.objects.children.len(), 1);
        let points = tree.objects.children.get("ns=2;s=Points").unwrap();
        assert!(!points.children_loaded);

        tree.merge_children(
            "ns=2;s=Points",
            &[FlatBrowseRow {
                node_id: "ns=2;s=Temperature".into(),
                browse_name: "Temperature".into(),
                display_name: "Temperature".into(),
                is_variable: true,
            }],
        );
        let points = tree.objects.children.get("ns=2;s=Points").unwrap();
        assert!(points.children_loaded);
        assert!(points.children["ns=2;s=Temperature"].is_variable);
    }
}
