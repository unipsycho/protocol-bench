//! Shared TopicBench-style topic tree from subscribed MQTT traffic.
//!
//! Used for MQTT panel preview and hub Input bind picking (topic → auto mqtt_N).

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use egui::{Align2, Color32, Pos2, RichText, Rounding, Sense, Stroke, Vec2};

use crate::theme::{font_ui, tint, ThemePalette};

const FLASH_MS: u64 = 500;

#[derive(Clone, Debug, Default)]
pub struct TopicNode {
    pub children: BTreeMap<String, TopicNode>,
    pub last_payload: Option<String>,
    pub message_count: u64,
    pub flash_until: Option<Instant>,
}

impl TopicNode {
    pub fn observe(&mut self, topic: &str, payload: &str, count: u64, now: Instant) {
        let flash = now + Duration::from_millis(FLASH_MS);
        let parts: Vec<&str> = topic.split('/').filter(|p| !p.is_empty()).collect();
        if parts.is_empty() {
            return;
        }
        let mut node = self;
        for (i, part) in parts.iter().enumerate() {
            node = node.children.entry((*part).to_string()).or_default();
            if i + 1 == parts.len() {
                node.last_payload = Some(payload.to_string());
                node.message_count = count.max(1);
                node.flash_until = Some(flash);
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.children.is_empty()
    }
}

#[derive(Clone, Debug, Default)]
pub struct TopicTree {
    pub root: TopicNode,
}

impl TopicTree {
    pub fn clear(&mut self) {
        self.root = TopicNode::default();
    }

    pub fn is_empty(&self) -> bool {
        self.root.is_empty()
    }

    /// Replace tree from pack traffic rows `{topic, payload, count}`.
    pub fn sync_from_traffic(&mut self, rows: &[(String, String, u64)]) {
        self.clear();
        let now = Instant::now();
        for (topic, payload, count) in rows {
            self.root.observe(topic, payload, *count, now);
        }
    }
}

/// How the tree responds to clicks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeMode {
    /// Browse only (MQTT panel preview).
    Preview,
    /// Click a leaf topic → pick for Input bind.
    PickTopic,
}

#[derive(Clone, Debug, Default)]
pub struct TreePick {
    pub topic: String,
    /// Optional JSON field path when a payload key row is clicked (unused for now).
    pub path: String,
}

/// Draw the tree. Returns a pick when `mode` is [`TreeMode::PickTopic`] and a leaf is clicked.
pub fn paint(
    ui: &mut egui::Ui,
    tree: &TopicTree,
    id_salt: &str,
    mode: TreeMode,
    p: &ThemePalette,
) -> Option<TreePick> {
    if tree.is_empty() {
        ui.label(
            RichText::new("No traffic yet — connect and subscribe to see topics.")
                .font(font_ui(12.0))
                .color(p.muted),
        );
        return None;
    }
    let now = Instant::now();
    let mut pick = None;
    // Tight vertical rhythm; no group guide lines.
    let prev_vline = ui.visuals().indent_has_left_vline;
    ui.visuals_mut().indent_has_left_vline = false;
    let prev_y = ui.spacing().item_spacing.y;
    let prev_interact_y = ui.spacing().interact_size.y;
    ui.spacing_mut().item_spacing.y = 1.0;
    ui.spacing_mut().interact_size.y = 16.0;
    paint_node(
        ui,
        &tree.root,
        String::new(),
        id_salt,
        mode,
        now,
        p,
        &mut pick,
    );
    ui.visuals_mut().indent_has_left_vline = prev_vline;
    ui.spacing_mut().item_spacing.y = prev_y;
    ui.spacing_mut().interact_size.y = prev_interact_y;
    pick
}

fn paint_node(
    ui: &mut egui::Ui,
    node: &TopicNode,
    path: String,
    id_salt: &str,
    mode: TreeMode,
    now: Instant,
    p: &ThemePalette,
    pick: &mut Option<TreePick>,
) {
    for (seg, child) in &node.children {
        let child_path = if path.is_empty() {
            seg.clone()
        } else {
            format!("{path}/{seg}")
        };
        let has_kids = !child.children.is_empty();
        let is_leaf = child.last_payload.is_some();

        if has_kids {
            // Salt bump clears prior depth<2 defaults so deep leaves open.
            let id = ui.make_persistent_id(("pb_tt_open", id_salt, &child_path));
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
                .show_header(ui, |ui| {
                    // Parent headers: labels only (expand owns the row).
                    row_contents(ui, seg, child, is_leaf, now, mode, p, &child_path, pick);
                })
                .body(|ui| {
                    paint_node(ui, child, child_path, id_salt, mode, now, p, pick);
                });
        } else {
            // Full-width leaf row so hover/click covers topic + payload.
            paint_leaf_row(ui, seg, child, is_leaf, now, mode, p, &child_path, pick);
        }
    }
}

fn paint_leaf_row(
    ui: &mut egui::Ui,
    seg: &str,
    child: &TopicNode,
    is_leaf: bool,
    now: Instant,
    mode: TreeMode,
    p: &ThemePalette,
    full_path: &str,
    pick: &mut Option<TreePick>,
) {
    let row_h = ui.spacing().interact_size.y.max(16.0);
    let w = ui.available_width().max(1.0);
    let pickable = mode == TreeMode::PickTopic && is_leaf;
    let sense = if pickable {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (row, resp) = ui.allocate_exact_size(Vec2::new(w, row_h), sense);
    // Match inspector list hover: soft accent wash + hand on pickable leaves.
    if resp.hovered() {
        ui.painter()
            .rect_filled(row, Rounding::same(4.0), tint(p.accent, 16));
        if pickable {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }
    let mut x = row.min.x + 18.0;
    paint_row_text(ui, row, &mut x, seg, child, is_leaf, now, p);
    if pickable && resp.clicked() {
        *pick = Some(TreePick {
            topic: full_path.to_string(),
            path: String::new(),
        });
    }
    if pickable {
        resp.on_hover_text("Click to bind topic → auto mqtt_N");
    }
}

fn row_contents(
    ui: &mut egui::Ui,
    seg: &str,
    child: &TopicNode,
    is_leaf: bool,
    now: Instant,
    mode: TreeMode,
    p: &ThemePalette,
    full_path: &str,
    pick: &mut Option<TreePick>,
) {
    let pickable = mode == TreeMode::PickTopic && is_leaf;
    let flash = child
        .flash_until
        .map(|until| until > now)
        .unwrap_or(false);
    let mut text = RichText::new(seg).font(font_ui(12.5)).color(p.text);
    if flash {
        text = text.color(p.accent);
    }
    let sense = if pickable {
        Sense::click()
    } else {
        Sense::hover()
    };
    let resp = ui.add(egui::Label::new(text).sense(sense));
    if pickable && resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if is_leaf {
        if let Some(payload) = child.last_payload.as_deref() {
            ui.add_space(8.0);
            let preview = truncate(payload, 40);
            ui.label(
                RichText::new(preview)
                    .font(font_ui(11.0))
                    .color(p.muted),
            );
        }
    }
    if pickable && resp.clicked() {
        *pick = Some(TreePick {
            topic: full_path.to_string(),
            path: String::new(),
        });
    }
    if pickable {
        resp.on_hover_text("Click to bind topic → auto mqtt_N");
    }
}

fn paint_row_text(
    ui: &mut egui::Ui,
    row: egui::Rect,
    x: &mut f32,
    seg: &str,
    child: &TopicNode,
    is_leaf: bool,
    now: Instant,
    p: &ThemePalette,
) {
    let flash = child
        .flash_until
        .map(|until| until > now)
        .unwrap_or(false);
    let color = if flash { p.accent } else { p.text };
    let galley = ui.fonts(|f| f.layout_no_wrap(seg.to_owned(), font_ui(12.5), color));
    let pos = Pos2::new(*x, row.center().y - galley.size().y * 0.5);
    *x += galley.size().x;
    ui.painter().galley(pos, galley, color);
    if is_leaf {
        if let Some(payload) = child.last_payload.as_deref() {
            *x += 8.0;
            let preview = truncate(payload, 40);
            let galley = ui.fonts(|f| f.layout_no_wrap(preview, font_ui(11.0), p.muted));
            let pos = Pos2::new(*x, row.center().y - galley.size().y * 0.5);
            ui.painter().galley(pos, galley, p.muted);
        }
    }
}

fn truncate(value: &str, max: usize) -> String {
    let flat = value.replace('\n', " ");
    if flat.chars().count() <= max {
        flat
    } else {
        let mut out: String = flat.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

/// Parse pack `detail_json` clients array into per-connection traffic rows.
/// Returns `None` when JSON is truncated/invalid (caller keeps prior trees).
pub fn traffic_from_detail_ok(
    detail_json: &str,
) -> Option<BTreeMap<String, Vec<(String, String, u64)>>> {
    let t = detail_json.trim();
    if t.is_empty() {
        return Some(BTreeMap::new());
    }
    let value: serde_json::Value = serde_json::from_str(t).ok()?;
    let arr = value.as_array()?;
    let mut out = BTreeMap::new();
    for client in arr {
        let id = client
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if id.is_empty() {
            continue;
        }
        let mut rows = Vec::new();
        if let Some(traffic) = client.get("traffic").and_then(|v| v.as_array()) {
            for leaf in traffic {
                let topic = leaf
                    .get("topic")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if topic.is_empty() {
                    continue;
                }
                let payload = leaf
                    .get("payload")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let count = leaf.get("count").and_then(|v| v.as_u64()).unwrap_or(1);
                rows.push((topic, payload, count));
            }
        }
        out.insert(id, rows);
    }
    Some(out)
}

#[allow(dead_code)]
fn _theme_bits(p: &ThemePalette) {
    let _ = Color32::BLACK;
    let _ = Stroke::new(1.0, p.line);
    let _ = Vec2::ZERO;
    let _ = Align2::LEFT_CENTER;
}
