//! Nested split tree. A leaf is one area: several panes can share it as tabs.
//! Edge dock inserts a sibling; the top drop adds a tab; the center swaps panes.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::dock::DockPane;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SplitDir {
    /// Side by side.
    Row,
    /// Stacked.
    Col,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileEdge {
    /// Tree split only. The top drop in the window adds a tab instead of this edge.
    #[allow(dead_code)]
    North,
    South,
    West,
    East,
}

impl TileEdge {
    pub fn phrase(self) -> &'static str {
        match self {
            Self::North => "above",
            Self::South => "below",
            Self::West => "left of",
            Self::East => "right of",
        }
    }
}

/// One panel area. `tabs` is the strip, left to right. `active` is the one on top.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabGroup {
    pub tabs: Vec<DockPane>,
    pub active: usize,
}

impl TabGroup {
    pub fn single(pane: DockPane) -> Self {
        Self {
            tabs: vec![pane],
            active: 0,
        }
    }

    pub fn contains(&self, pane: DockPane) -> bool {
        self.tabs.contains(&pane)
    }

    /// The pane the area is showing. Empty groups are collapsed before this is used.
    pub fn active_pane(&self) -> DockPane {
        let index = self.active.min(self.tabs.len().saturating_sub(1));
        self.tabs[index]
    }

    fn remove(&mut self, pane: DockPane) -> bool {
        let Some(index) = self.tabs.iter().position(|item| *item == pane) else {
            return false;
        };
        self.tabs.remove(index);
        if self.active > index {
            self.active -= 1;
        }
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len().saturating_sub(1);
        }
        true
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum LayoutNode {
    Leaf(TabGroup),
    Split {
        dir: SplitDir,
        /// First-child share of the pair, 0..1.
        ratio: f32,
        first: Box<LayoutNode>,
        second: Box<LayoutNode>,
    },
}

impl Default for LayoutNode {
    fn default() -> Self {
        Self::default_tree()
    }
}

impl LayoutNode {
    /// Opening arrangement: hubs across the top; project, points and mappings as tabs on the left;
    /// the Protocols pane in the middle; traffic on the right. Users can still drag tabs out.
    pub fn default_tree() -> Self {
        fn tabs(panes: &[DockPane]) -> LayoutNode {
            LayoutNode::Leaf(TabGroup {
                tabs: panes.to_vec(),
                active: 0,
            })
        }
        split(
            SplitDir::Col,
            0.42,
            leaf(DockPane::Hubs),
            split(
                SplitDir::Row,
                0.30,
                tabs(&[DockPane::Project, DockPane::Points, DockPane::Mappings, DockPane::Help]),
                split(
                    SplitDir::Row,
                    0.76,
                    leaf(DockPane::Protocols),
                    leaf(DockPane::Traffic),
                ),
            ),
        )
    }

    pub fn contains_pane(&self, want: DockPane) -> bool {
        match self {
            LayoutNode::Leaf(group) => group.contains(want),
            LayoutNode::Split { first, second, .. } => {
                first.contains_pane(want) || second.contains_pane(want)
            }
        }
    }

    /// The area that holds `pane`, if it is in the tree.
    pub fn group_of(&self, pane: DockPane) -> Option<&TabGroup> {
        match self {
            LayoutNode::Leaf(group) if group.contains(pane) => Some(group),
            LayoutNode::Leaf(_) => None,
            LayoutNode::Split { first, second, .. } => {
                first.group_of(pane).or_else(|| second.group_of(pane))
            }
        }
    }

    /// Leaf that holds `pane`, so a maximized area can keep its tab strip.
    pub fn area_containing(&self, pane: DockPane) -> Option<&LayoutNode> {
        match self {
            LayoutNode::Leaf(group) if group.contains(pane) => Some(self),
            LayoutNode::Leaf(_) => None,
            LayoutNode::Split { first, second, .. } => first
                .area_containing(pane)
                .or_else(|| second.area_containing(pane)),
        }
    }

    pub fn same_group(&self, a: DockPane, b: DockPane) -> bool {
        self.group_of(a).is_some_and(|group| group.contains(b))
    }

    /// Show `pane` in its area. No-op when the pane is not in the tree.
    pub fn set_active(&mut self, pane: DockPane) -> bool {
        match self {
            LayoutNode::Leaf(group) if group.contains(pane) => {
                group.active = group
                    .tabs
                    .iter()
                    .position(|item| *item == pane)
                    .unwrap_or(0);
                true
            }
            LayoutNode::Leaf(_) => false,
            LayoutNode::Split { first, second, .. } => {
                first.set_active(pane) || second.set_active(pane)
            }
        }
    }

    /// Leaf order, first child then second, tabs left to right.
    pub fn collect_panes(&self, out: &mut Vec<DockPane>) {
        match self {
            LayoutNode::Leaf(group) => out.extend(group.tabs.iter().copied()),
            LayoutNode::Split { first, second, .. } => {
                first.collect_panes(out);
                second.collect_panes(out);
            }
        }
    }

    /// Clone with hidden panes removed. An area with no visible tabs drops out.
    pub fn without_hidden(&self, hidden: &[DockPane]) -> Option<LayoutNode> {
        match self {
            LayoutNode::Leaf(group) => {
                let active = group.active_pane();
                let tabs: Vec<DockPane> = group
                    .tabs
                    .iter()
                    .copied()
                    .filter(|pane| !hidden.iter().any(|item| item == pane))
                    .collect();
                if tabs.is_empty() {
                    None
                } else {
                    let active = tabs.iter().position(|pane| *pane == active).unwrap_or(0);
                    Some(LayoutNode::Leaf(TabGroup { tabs, active }))
                }
            }
            LayoutNode::Split {
                dir,
                ratio,
                first,
                second,
            } => match (first.without_hidden(hidden), second.without_hidden(hidden)) {
                (None, None) => None,
                (Some(only), None) | (None, Some(only)) => Some(only),
                (Some(left), Some(right)) => Some(LayoutNode::Split {
                    dir: *dir,
                    ratio: *ratio,
                    first: Box::new(left),
                    second: Box::new(right),
                }),
            },
        }
    }

    pub fn swap_panes(&mut self, a: DockPane, b: DockPane) {
        if a == b {
            return;
        }
        self.for_each_pane(&mut |pane| {
            if *pane == a {
                *pane = b;
            } else if *pane == b {
                *pane = a;
            }
        });
    }

    /// Put `to` where `from` sits (same area, same tab slot).
    pub fn replace_pane(&mut self, from: DockPane, to: DockPane) {
        self.for_each_pane(&mut |pane| {
            if *pane == from {
                *pane = to;
            }
        });
    }

    fn for_each_pane(&mut self, f: &mut dyn FnMut(&mut DockPane)) {
        match self {
            LayoutNode::Leaf(group) => {
                for pane in &mut group.tabs {
                    f(pane);
                }
            }
            LayoutNode::Split { first, second, .. } => {
                first.for_each_pane(f);
                second.for_each_pane(f);
            }
        }
    }

    /// Take `want` out of its area. A last tab collapses the parent split. A lone root tab stays.
    pub fn remove_leaf(&mut self, want: DockPane) -> bool {
        match self {
            LayoutNode::Leaf(group) => group.tabs.len() > 1 && group.remove(want),
            LayoutNode::Split { first, second, .. } => {
                if sole_tab(first, want) {
                    *self = second.as_ref().clone();
                    return true;
                }
                if sole_tab(second, want) {
                    *self = first.as_ref().clone();
                    return true;
                }
                first.remove_leaf(want) || second.remove_leaf(want)
            }
        }
    }

    /// Split the area that holds `target`, putting `incoming` on `edge`. The area's tabs stay together.
    pub fn split_at(&mut self, target: DockPane, incoming: DockPane, edge: TileEdge) -> bool {
        match self {
            LayoutNode::Leaf(group) if group.contains(target) => {
                let keep = LayoutNode::Leaf(group.clone());
                let neu = leaf(incoming);
                let (dir, first, second) = match edge {
                    TileEdge::North => (SplitDir::Col, neu, keep),
                    TileEdge::South => (SplitDir::Col, keep, neu),
                    TileEdge::West => (SplitDir::Row, neu, keep),
                    TileEdge::East => (SplitDir::Row, keep, neu),
                };
                *self = LayoutNode::Split {
                    dir,
                    ratio: 0.5,
                    first: Box::new(first),
                    second: Box::new(second),
                };
                true
            }
            LayoutNode::Leaf(_) => false,
            LayoutNode::Split { first, second, .. } => {
                first.split_at(target, incoming, edge) || second.split_at(target, incoming, edge)
            }
        }
    }

    /// Move `moving` onto the tab strip of the area that holds `target`.
    pub fn tab_onto(&mut self, moving: DockPane, target: DockPane) -> bool {
        if moving == target || !self.contains_pane(moving) || !self.contains_pane(target) {
            return false;
        }
        if self.same_group(moving, target) {
            return false;
        }
        let backup = self.clone();
        if !self.remove_leaf(moving) {
            return false;
        }
        if !self.push_tab(target, moving) {
            *self = backup;
            return false;
        }
        true
    }

    fn push_tab(&mut self, target: DockPane, incoming: DockPane) -> bool {
        match self {
            LayoutNode::Leaf(group) if group.contains(target) => {
                if !group.contains(incoming) {
                    group.tabs.push(incoming);
                    group.active = group.tabs.len() - 1;
                }
                true
            }
            LayoutNode::Leaf(_) => false,
            LayoutNode::Split { first, second, .. } => {
                first.push_tab(target, incoming) || second.push_tab(target, incoming)
            }
        }
    }

    /// Detach `moving` and split it onto `target`. Restores the tree if the split fails.
    pub fn dock_into(&mut self, moving: DockPane, target: DockPane, edge: TileEdge) -> bool {
        if moving == target || !self.contains_pane(moving) || !self.contains_pane(target) {
            return false;
        }
        let backup = self.clone();
        if !self.remove_leaf(moving) {
            return false;
        }
        if !self.split_at(target, moving, edge) {
            *self = backup;
            return false;
        }
        true
    }

    /// Move `pane` within its own tab strip so it lands before the tab now at `index`
    /// (`index == len` puts it last). The moved tab stays selected if it was.
    pub fn reorder_tab(&mut self, pane: DockPane, index: usize) -> bool {
        let Some(group) = self.leaf_mut(pane) else {
            return false;
        };
        let Some(from) = group.tabs.iter().position(|item| *item == pane) else {
            return false;
        };
        let index = index.min(group.tabs.len());
        if index == from || index == from + 1 {
            return false;
        }
        let active = group.active_pane();
        group.tabs.remove(from);
        let to = if index > from { index - 1 } else { index };
        group.tabs.insert(to, pane);
        group.active = group.tabs.iter().position(|item| *item == active).unwrap_or(0);
        true
    }

    /// Like [`Self::tab_onto`], but inserts `moving` before the tab at `index` of the target strip.
    pub fn tab_onto_at(&mut self, moving: DockPane, target: DockPane, index: usize) -> bool {
        if moving == target || !self.contains_pane(moving) || !self.contains_pane(target) {
            return false;
        }
        if self.same_group(moving, target) {
            return false;
        }
        let backup = self.clone();
        if !self.remove_leaf(moving) {
            return false;
        }
        let Some(group) = self.leaf_mut(target) else {
            *self = backup;
            return false;
        };
        let at = index.min(group.tabs.len());
        group.tabs.insert(at, moving);
        group.active = at;
        true
    }

    /// Mutable area that holds `pane`.
    fn leaf_mut(&mut self, pane: DockPane) -> Option<&mut TabGroup> {
        match self {
            LayoutNode::Leaf(group) if group.contains(pane) => Some(group),
            LayoutNode::Leaf(_) => None,
            LayoutNode::Split { first, second, .. } => {
                if first.contains_pane(pane) {
                    first.leaf_mut(pane)
                } else {
                    second.leaf_mut(pane)
                }
            }
        }
    }

    /// Remove the whole area holding `member`, collapsing its parent split. The root area stays.
    fn remove_group(&mut self, member: DockPane) -> Option<TabGroup> {
        match self {
            LayoutNode::Leaf(_) => None,
            LayoutNode::Split { first, second, .. } => {
                if let LayoutNode::Leaf(group) = first.as_ref() {
                    if group.contains(member) {
                        let taken = group.clone();
                        *self = second.as_ref().clone();
                        return Some(taken);
                    }
                }
                if let LayoutNode::Leaf(group) = second.as_ref() {
                    if group.contains(member) {
                        let taken = group.clone();
                        *self = first.as_ref().clone();
                        return Some(taken);
                    }
                }
                if first.contains_pane(member) {
                    first.remove_group(member)
                } else {
                    second.remove_group(member)
                }
            }
        }
    }

    /// Move the whole area holding `member` (all its tabs) to `edge` of the area holding `target`.
    pub fn dock_group(&mut self, member: DockPane, target: DockPane, edge: TileEdge) -> bool {
        if self.same_group(member, target) || !self.contains_pane(target) {
            return false;
        }
        let backup = self.clone();
        let Some(moving) = self.remove_group(member) else {
            return false;
        };
        if !self.split_at_with(target, LayoutNode::Leaf(moving), edge) {
            *self = backup;
            return false;
        }
        true
    }

    /// Move every tab of the area holding `member` into the strip holding `target`, before `index`.
    pub fn merge_group(&mut self, member: DockPane, target: DockPane, index: usize) -> bool {
        if self.same_group(member, target) || !self.contains_pane(target) {
            return false;
        }
        let backup = self.clone();
        let Some(moving) = self.remove_group(member) else {
            return false;
        };
        let Some(group) = self.leaf_mut(target) else {
            *self = backup;
            return false;
        };
        let at = index.min(group.tabs.len());
        let shown = moving.active_pane();
        for (i, pane) in moving.tabs.iter().enumerate() {
            group.tabs.insert(at + i, *pane);
        }
        group.active = group.tabs.iter().position(|item| *item == shown).unwrap_or(at);
        true
    }

    /// Exchange two whole areas (tabs and selection) in place.
    pub fn swap_groups(&mut self, a: DockPane, b: DockPane) -> bool {
        if self.same_group(a, b) {
            return false;
        }
        let (Some(ga), Some(gb)) = (self.group_of(a).cloned(), self.group_of(b).cloned()) else {
            return false;
        };
        self.for_each_leaf(&mut |group| {
            if *group == ga {
                *group = gb.clone();
            } else if *group == gb {
                *group = ga.clone();
            }
        });
        true
    }

    fn for_each_leaf(&mut self, f: &mut dyn FnMut(&mut TabGroup)) {
        match self {
            LayoutNode::Leaf(group) => f(group),
            LayoutNode::Split { first, second, .. } => {
                first.for_each_leaf(f);
                second.for_each_leaf(f);
            }
        }
    }

    /// Split the area that holds `target`, putting `node` on `edge`.
    fn split_at_with(&mut self, target: DockPane, node: LayoutNode, edge: TileEdge) -> bool {
        match self {
            LayoutNode::Leaf(group) if group.contains(target) => {
                let keep = LayoutNode::Leaf(group.clone());
                let (dir, first, second) = match edge {
                    TileEdge::North => (SplitDir::Col, node, keep),
                    TileEdge::South => (SplitDir::Col, keep, node),
                    TileEdge::West => (SplitDir::Row, node, keep),
                    TileEdge::East => (SplitDir::Row, keep, node),
                };
                *self = LayoutNode::Split {
                    dir,
                    ratio: 0.5,
                    first: Box::new(first),
                    second: Box::new(second),
                };
                true
            }
            LayoutNode::Leaf(_) => false,
            LayoutNode::Split { first, second, .. } => {
                if first.contains_pane(target) {
                    first.split_at_with(target, node, edge)
                } else {
                    second.split_at_with(target, node, edge)
                }
            }
        }
    }

    /// Insert `pane` beside `beside` when it is not already in the tree.
    pub fn ensure_pane(&mut self, pane: DockPane, beside: DockPane, edge: TileEdge) -> bool {
        if self.contains_pane(pane) {
            return false;
        }
        if self.contains_pane(beside) {
            return self.split_at(beside, pane, edge);
        }
        let keep = self.clone();
        let neu = leaf(pane);
        let (dir, first, second) = match edge {
            TileEdge::North => (SplitDir::Col, neu, keep),
            TileEdge::South => (SplitDir::Col, keep, neu),
            TileEdge::West => (SplitDir::Row, neu, keep),
            TileEdge::East => (SplitDir::Row, keep, neu),
        };
        *self = LayoutNode::Split {
            dir,
            ratio: 0.35,
            first: Box::new(first),
            second: Box::new(second),
        };
        true
    }

    /// Full-width band on the top of the whole tree. Used when a new view joins a saved layout.
    pub fn band_north(&mut self, pane: DockPane, ratio: f32) {
        if self.contains_pane(pane) {
            return;
        }
        let keep = self.clone();
        *self = LayoutNode::Split {
            dir: SplitDir::Col,
            ratio: ratio.clamp(0.2, 0.6),
            first: Box::new(leaf(pane)),
            second: Box::new(keep),
        };
    }

    /// `path` walks false=first, true=second. Empty path is this split.
    pub fn set_ratio_at(&mut self, path: &[bool], ratio: f32) {
        if path.is_empty() {
            if let LayoutNode::Split { ratio: slot, .. } = self {
                *slot = ratio.clamp(0.08, 0.92);
            }
            return;
        }
        if let LayoutNode::Split { first, second, .. } = self {
            if path[0] {
                second.set_ratio_at(&path[1..], ratio);
            } else {
                first.set_ratio_at(&path[1..], ratio);
            }
        }
    }
}

fn sole_tab(node: &LayoutNode, want: DockPane) -> bool {
    matches!(node, LayoutNode::Leaf(group) if group.tabs.len() == 1 && group.tabs[0] == want)
}

fn leaf(pane: DockPane) -> LayoutNode {
    LayoutNode::Leaf(TabGroup::single(pane))
}

/// Saved trees used `Leaf.pane`. New trees use `Leaf.tabs` plus `active`.
#[derive(Serialize, Deserialize)]
enum LayoutSer {
    Leaf {
        #[serde(default)]
        tabs: Vec<DockPane>,
        #[serde(default)]
        active: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane: Option<DockPane>,
    },
    Split {
        dir: SplitDir,
        ratio: f32,
        first: Box<LayoutSer>,
        second: Box<LayoutSer>,
    },
}

impl LayoutNode {
    fn to_ser(&self) -> LayoutSer {
        match self {
            LayoutNode::Leaf(group) => LayoutSer::Leaf {
                tabs: group.tabs.clone(),
                active: group.active,
                pane: None,
            },
            LayoutNode::Split {
                dir,
                ratio,
                first,
                second,
            } => LayoutSer::Split {
                dir: *dir,
                ratio: *ratio,
                first: Box::new(first.to_ser()),
                second: Box::new(second.to_ser()),
            },
        }
    }

    fn from_ser(ser: LayoutSer) -> Self {
        match ser {
            LayoutSer::Leaf { tabs, active, pane } => {
                let mut tabs = tabs;
                if tabs.is_empty() {
                    if let Some(pane) = pane {
                        tabs.push(pane);
                    }
                }
                let mut seen = Vec::new();
                tabs.retain(|pane| {
                    if seen.contains(pane) {
                        false
                    } else {
                        seen.push(*pane);
                        true
                    }
                });
                if tabs.is_empty() {
                    tabs.push(DockPane::Project);
                }
                let active = active.min(tabs.len() - 1);
                LayoutNode::Leaf(TabGroup { tabs, active })
            }
            LayoutSer::Split {
                dir,
                ratio,
                first,
                second,
            } => LayoutNode::Split {
                dir,
                ratio,
                first: Box::new(LayoutNode::from_ser(*first)),
                second: Box::new(LayoutNode::from_ser(*second)),
            },
        }
    }
}

impl Serialize for LayoutNode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_ser().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for LayoutNode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        LayoutSer::deserialize(deserializer).map(LayoutNode::from_ser)
    }
}

fn split(dir: SplitDir, ratio: f32, first: LayoutNode, second: LayoutNode) -> LayoutNode {
    LayoutNode::Split {
        dir,
        ratio,
        first: Box::new(first),
        second: Box::new(second),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panes(tree: &LayoutNode) -> Vec<DockPane> {
        let mut out = Vec::new();
        tree.collect_panes(&mut out);
        out
    }

    #[test]
    fn default_tree_has_every_pane_once() {
        let tree = LayoutNode::default_tree();
        let got = panes(&tree);
        assert_eq!(got.len(), DockPane::ALL.len());
        for pane in DockPane::ALL {
            assert_eq!(got.iter().filter(|item| **item == pane).count(), 1);
        }
    }

    #[test]
    fn swap_exchanges_two_leaves() {
        let mut tree = LayoutNode::default_tree();
        let before = panes(&tree);
        let a = before[0];
        let b = before[1];
        tree.swap_panes(a, b);
        let after = panes(&tree);
        assert_eq!(after[0], b);
        assert_eq!(after[1], a);
    }

    #[test]
    fn dock_moves_a_pane_and_keeps_the_rest() {
        let mut tree = LayoutNode::default_tree();
        assert!(tree.dock_into(DockPane::Traffic, DockPane::Project, TileEdge::North));
        for pane in DockPane::ALL {
            assert!(tree.contains_pane(pane));
        }
        assert!(traffic_above_project(&tree));
    }

    fn traffic_above_project(node: &LayoutNode) -> bool {
        match node {
            LayoutNode::Split {
                dir, first, second, ..
            } => {
                let placed = *dir == SplitDir::Col
                    && sole_tab(first, DockPane::Traffic)
                    && second.contains_pane(DockPane::Project);
                placed || traffic_above_project(first) || traffic_above_project(second)
            }
            LayoutNode::Leaf(_) => false,
        }
    }

    /// Plain split layout for the tab-movement tests (independent of the opening arrangement).
    fn tab_test_tree() -> LayoutNode {
        split(
            SplitDir::Row,
            0.5,
            leaf(DockPane::Project),
            split(
                SplitDir::Row,
                0.5,
                leaf(DockPane::Modbus),
                split(SplitDir::Col, 0.5, leaf(DockPane::Files), leaf(DockPane::Traffic)),
            ),
        )
    }

    fn tabs_leaf(panes: &[DockPane], active: usize) -> LayoutNode {
        LayoutNode::Leaf(TabGroup { tabs: panes.to_vec(), active })
    }

    #[test]
    fn reorder_tab_moves_within_strip_and_keeps_selection() {
        use DockPane::*;
        let mut tree = split(SplitDir::Row, 0.5, tabs_leaf(&[Project, Points, Mappings], 1), leaf(Hubs));
        assert!(!tree.reorder_tab(Project, 0), "same slot is a no-op");
        assert!(!tree.reorder_tab(Project, 1), "slot right after itself is a no-op");
        assert!(tree.reorder_tab(Project, 3));
        let g = tree.group_of(Points).unwrap();
        assert_eq!(g.tabs, vec![Points, Mappings, Project]);
        assert_eq!(g.active_pane(), Points);
        assert!(tree.reorder_tab(Project, 0));
        assert_eq!(tree.group_of(Points).unwrap().tabs, vec![Project, Points, Mappings]);
    }

    #[test]
    fn tab_onto_at_inserts_at_position() {
        use DockPane::*;
        let mut tree = split(SplitDir::Row, 0.5, tabs_leaf(&[Project, Points], 0), leaf(Hubs));
        assert!(tree.tab_onto_at(Hubs, Project, 1));
        let g = tree.group_of(Hubs).unwrap();
        assert_eq!(g.tabs, vec![Project, Hubs, Points]);
        assert_eq!(g.active_pane(), Hubs);
    }

    #[test]
    fn group_dock_merge_and_swap() {
        use DockPane::*;
        let base = split(
            SplitDir::Row,
            0.5,
            tabs_leaf(&[Project, Points], 1),
            split(SplitDir::Col, 0.5, leaf(Hubs), tabs_leaf(&[Mqtt, Modbus], 0)),
        );

        let mut t = base.clone();
        assert!(t.dock_group(Points, Hubs, TileEdge::South));
        assert_eq!(t.group_of(Project).unwrap().tabs, vec![Project, Points]);
        assert_eq!(t.group_of(Project).unwrap().active_pane(), Points);
        assert!(!t.same_group(Project, Hubs));

        let mut t = base.clone();
        assert!(t.merge_group(Mqtt, Project, 1));
        let g = t.group_of(Project).unwrap();
        assert_eq!(g.tabs, vec![Project, Mqtt, Modbus, Points]);
        assert_eq!(g.active_pane(), Mqtt);

        let mut t = base.clone();
        assert!(t.swap_groups(Project, Modbus));
        let mut order = Vec::new();
        t.collect_panes(&mut order);
        assert_eq!(order, vec![Mqtt, Modbus, Hubs, Project, Points]);

        let mut t = base.clone();
        assert!(!t.dock_group(Project, Points, TileEdge::West), "cannot dock onto itself");
        let mut lone = tabs_leaf(&[Project, Points], 0);
        assert!(!lone.dock_group(Project, Points, TileEdge::East));
    }

    #[test]
    fn tab_onto_joins_an_area_and_selects_the_new_tab() {
        let mut tree = tab_test_tree();
        assert!(tree.tab_onto(DockPane::Files, DockPane::Project));
        let group = tree.group_of(DockPane::Project).expect("project area");
        assert_eq!(group.tabs, vec![DockPane::Project, DockPane::Files]);
        assert_eq!(group.active_pane(), DockPane::Files);
        assert_eq!(
            panes(&tree)
                .iter()
                .filter(|pane| **pane == DockPane::Files)
                .count(),
            1
        );
        assert!(!tree.tab_onto(DockPane::Files, DockPane::Project));
    }

    #[test]
    fn moving_a_tab_leaves_the_rest_of_the_strip() {
        let mut tree = tab_test_tree();
        assert!(tree.tab_onto(DockPane::Files, DockPane::Project));
        assert!(tree.tab_onto(DockPane::Traffic, DockPane::Project));
        assert!(tree.tab_onto(DockPane::Files, DockPane::Modbus));
        let project = tree.group_of(DockPane::Project).expect("project area");
        assert_eq!(project.tabs, vec![DockPane::Project, DockPane::Traffic]);
        let modbus = tree.group_of(DockPane::Modbus).expect("modbus area");
        assert_eq!(modbus.tabs, vec![DockPane::Modbus, DockPane::Files]);
        assert_eq!(modbus.active_pane(), DockPane::Files);
    }

    #[test]
    fn edge_dock_keeps_a_tab_group_together() {
        let mut tree = tab_test_tree();
        assert!(tree.tab_onto(DockPane::Files, DockPane::Project));
        assert!(tree.dock_into(DockPane::Modbus, DockPane::Project, TileEdge::South));
        let group = tree.group_of(DockPane::Project).expect("project area");
        assert_eq!(group.tabs, vec![DockPane::Project, DockPane::Files]);
        assert!(!group.contains(DockPane::Modbus));
        assert!(tree.contains_pane(DockPane::Modbus));
    }

    #[test]
    fn pulling_one_tab_out_by_an_edge_leaves_the_others() {
        let mut tree = tab_test_tree();
        assert!(tree.tab_onto(DockPane::Files, DockPane::Project));
        assert!(tree.tab_onto(DockPane::Traffic, DockPane::Project));
        assert!(tree.dock_into(DockPane::Files, DockPane::Project, TileEdge::East));
        let group = tree.group_of(DockPane::Project).expect("project area");
        assert_eq!(group.tabs, vec![DockPane::Project, DockPane::Traffic]);
        let files = tree.area_containing(DockPane::Files).expect("files area");
        assert!(sole_tab(files, DockPane::Files));
    }

    #[test]
    fn legacy_pane_field_loads_as_a_single_tab() {
        let text = "[Leaf]\npane = \"Modbus\"\n";
        let tree: LayoutNode = toml::from_str(text).expect("parse");
        let group = tree.group_of(DockPane::Modbus).expect("modbus");
        assert_eq!(group.tabs, vec![DockPane::Modbus]);
        assert_eq!(group.active, 0);
    }

    #[test]
    fn hidden_tab_drops_out_of_the_strip_only() {
        let mut tree = tab_test_tree();
        assert!(tree.tab_onto(DockPane::Files, DockPane::Project));
        let shown = tree
            .without_hidden(&[DockPane::Files])
            .expect("area remains");
        let group = shown.group_of(DockPane::Project).expect("project");
        assert_eq!(group.tabs, vec![DockPane::Project]);
        assert!(tree
            .group_of(DockPane::Project)
            .unwrap()
            .contains(DockPane::Files));
    }

    #[test]
    fn hidden_leaf_collapses() {
        let tree = LayoutNode::default_tree();
        let shown = tree
            .without_hidden(&[DockPane::Files])
            .expect("still panes");
        assert!(!shown.contains_pane(DockPane::Files));
        assert!(shown.contains_pane(DockPane::Traffic));
    }
}
