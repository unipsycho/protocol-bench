//! Grid logic for dashboard layouts: placement, move, resize, collision handling, swap, split.
//!
//! Pure data and integer maths (no UI), so every rule is unit-tested natively. The grid has
//! `cols` columns and unlimited rows; tiles never overlap and gaps are allowed. Dropping a tile
//! onto occupied cells pushes the tiles underneath **down** (gridstack-style).

use crate::{default_size, min_size, Layout, Tile, Widget};

/// A rectangle in cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub fn of(t: &Tile) -> Rect {
        Rect { x: t.x, y: t.y, w: t.w, h: t.h }
    }
    pub fn bottom(&self) -> u32 {
        self.y + self.h
    }
    pub fn right(&self) -> u32 {
        self.x + self.w
    }
    pub fn overlaps(&self, o: &Rect) -> bool {
        self.x < o.right() && o.x < self.right() && self.y < o.bottom() && o.y < self.bottom()
    }
}

/// Which side of a target tile a dragged tile was dropped on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

impl Layout {
    pub fn tile(&self, id: u32) -> Option<&Tile> {
        self.tiles.iter().find(|t| t.id == id)
    }

    pub fn tile_mut(&mut self, id: u32) -> Option<&mut Tile> {
        self.tiles.iter_mut().find(|t| t.id == id)
    }

    pub fn next_id(&self) -> u32 {
        self.tiles.iter().map(|t| t.id).max().unwrap_or(0) + 1
    }

    /// Rows in use (bottom of the lowest tile).
    pub fn rows(&self) -> u32 {
        self.tiles.iter().map(|t| t.y + t.h).max().unwrap_or(0)
    }

    /// Keep a tile inside the columns and at least its widget's minimum size.
    fn clamp_tile(cols: u32, t: &mut Tile) {
        let (mw, mh) = min_size(t.widget);
        t.w = t.w.max(mw).min(cols);
        t.h = t.h.max(mh);
        t.x = t.x.min(cols - t.w);
    }

    /// Clamp every tile, then push overlapping tiles down (no tile pinned).
    pub fn normalize(&mut self) {
        let cols = self.cols;
        for t in &mut self.tiles {
            Self::clamp_tile(cols, t);
        }
        self.resolve(None);
    }

    /// Resolve overlaps. `pinned` (the tile being dragged) keeps its rectangle; everything else is
    /// placed in (y, x) order and pushed down past whatever it hits.
    pub fn resolve(&mut self, pinned: Option<u32>) {
        let mut order: Vec<usize> = (0..self.tiles.len()).filter(|i| Some(self.tiles[*i].id) != pinned).collect();
        order.sort_by_key(|i| (self.tiles[*i].y, self.tiles[*i].x, self.tiles[*i].id));
        let mut placed: Vec<Rect> = pinned.and_then(|p| self.tile(p)).map(|t| vec![Rect::of(t)]).unwrap_or_default();
        for i in order {
            loop {
                let r = Rect::of(&self.tiles[i]);
                match placed.iter().filter(|p| p.overlaps(&r)).map(|p| p.bottom()).max() {
                    Some(b) => self.tiles[i].y = b,
                    None => break,
                }
            }
            placed.push(Rect::of(&self.tiles[i]));
        }
    }

    /// First spot (reading order) where a `w × h` tile fits without overlapping anything.
    pub fn find_free(&self, w: u32, h: u32) -> (u32, u32) {
        let w = w.min(self.cols);
        let rects: Vec<Rect> = self.tiles.iter().map(Rect::of).collect();
        let mut y = 0;
        loop {
            for x in 0..=(self.cols - w) {
                let r = Rect { x, y, w, h };
                if !rects.iter().any(|o| o.overlaps(&r)) {
                    return (x, y);
                }
            }
            y += 1;
        }
    }

    /// Add a tile of `widget` at the default size in the first free spot (or at `at`, pushing
    /// others down). Returns its id.
    pub fn add(&mut self, widget: Widget, at: Option<(u32, u32)>) -> u32 {
        let id = self.next_id();
        let mut t = Tile::new(id, widget);
        t.w = t.w.min(self.cols);
        match at {
            Some((x, y)) => {
                t.x = x.min(self.cols - t.w);
                t.y = y;
            }
            None => {
                let (x, y) = self.find_free(t.w, t.h);
                t.x = x;
                t.y = y;
            }
        }
        self.tiles.push(t);
        self.resolve(Some(id));
        id
    }

    /// Add a tile covering exactly `r` (a rectangle dragged on empty space), pushing others down.
    pub fn add_rect(&mut self, widget: Widget, r: Rect) -> u32 {
        let id = self.next_id();
        let mut t = Tile::new(id, widget);
        t.x = r.x;
        t.y = r.y;
        t.w = r.w;
        t.h = r.h;
        Self::clamp_tile(self.cols, &mut t);
        self.tiles.push(t);
        self.resolve(Some(id));
        id
    }

    pub fn remove(&mut self, id: u32) -> bool {
        let n = self.tiles.len();
        self.tiles.retain(|t| t.id != id);
        self.tiles.len() != n
    }

    /// Move a tile's top-left corner to `(x, y)`; others make room.
    pub fn move_to(&mut self, id: u32, x: u32, y: u32) {
        let cols = self.cols;
        let Some(t) = self.tile_mut(id) else { return };
        t.x = x.min(cols - t.w);
        t.y = y;
        self.resolve(Some(id));
    }

    /// Give a tile a new rectangle (a corner / edge drag); others make room.
    pub fn resize_to(&mut self, id: u32, r: Rect) {
        let cols = self.cols;
        let Some(t) = self.tile_mut(id) else { return };
        t.x = r.x;
        t.y = r.y;
        t.w = r.w;
        t.h = r.h;
        Self::clamp_tile(cols, t);
        self.resolve(Some(id));
    }

    /// Two tiles exchange places (each takes the other's whole rectangle, grown to its own
    /// minimum size if needed).
    pub fn swap(&mut self, a: u32, b: u32) -> bool {
        if a == b {
            return false;
        }
        let (Some(ra), Some(rb)) = (self.tile(a).map(Rect::of), self.tile(b).map(Rect::of)) else { return false };
        for (id, r) in [(a, rb), (b, ra)] {
            if let Some(t) = self.tile_mut(id) {
                t.x = r.x;
                t.y = r.y;
                t.w = r.w;
                t.h = r.h;
            }
        }
        self.normalize();
        true
    }

    /// Drop `moving` on an edge of `target`: the target gives up half its width (left / right) or
    /// height (top / bottom) and `moving` takes that half. Refused (returns false, nothing
    /// changes) when either half would fall under its widget's minimum size.
    pub fn split_drop(&mut self, moving: u32, target: u32, edge: Edge) -> bool {
        if moving == target {
            return false;
        }
        let (Some(m), Some(t)) = (self.tile(moving), self.tile(target)) else { return false };
        let (mmin, tmin) = (min_size(m.widget), min_size(t.widget));
        let tr = Rect::of(t);
        let (new_m, new_t) = match edge {
            Edge::Left | Edge::Right => {
                let mw = tr.w / 2;
                let tw = tr.w - mw;
                if mw < mmin.0 || tw < tmin.0 || tr.h < mmin.1 {
                    return false;
                }
                if edge == Edge::Left {
                    (Rect { x: tr.x, y: tr.y, w: mw, h: tr.h }, Rect { x: tr.x + mw, y: tr.y, w: tw, h: tr.h })
                } else {
                    (Rect { x: tr.x + tw, y: tr.y, w: mw, h: tr.h }, Rect { x: tr.x, y: tr.y, w: tw, h: tr.h })
                }
            }
            Edge::Top | Edge::Bottom => {
                let mh = tr.h / 2;
                let th = tr.h - mh;
                if mh < mmin.1 || th < tmin.1 || tr.w < mmin.0 {
                    return false;
                }
                if edge == Edge::Top {
                    (Rect { x: tr.x, y: tr.y, w: tr.w, h: mh }, Rect { x: tr.x, y: tr.y + mh, w: tr.w, h: th })
                } else {
                    (Rect { x: tr.x, y: tr.y + th, w: tr.w, h: mh }, Rect { x: tr.x, y: tr.y, w: tr.w, h: th })
                }
            }
        };
        for (id, r) in [(moving, new_m), (target, new_t)] {
            if let Some(t) = self.tile_mut(id) {
                t.x = r.x;
                t.y = r.y;
                t.w = r.w;
                t.h = r.h;
            }
        }
        true
    }

    /// Add one tile per `(point, widget, units)` for points not already on the dashboard, then
    /// tidy everything with [`Layout::auto_arrange`]. Returns the new tile ids.
    pub fn add_points(&mut self, points: &[(String, Widget, String)]) -> Vec<u32> {
        let shown: std::collections::BTreeSet<&str> = self
            .tiles
            .iter()
            .flat_map(|t| std::iter::once(t.point.as_str()).chain(t.series.iter().map(|s| s.point.as_str())))
            .filter(|p| !p.is_empty())
            .collect();
        let fresh: Vec<&(String, Widget, String)> = points.iter().filter(|p| !shown.contains(p.0.as_str())).collect();
        let mut ids = Vec::new();
        for (point, widget, units) in fresh.into_iter().take(500usize.saturating_sub(self.tiles.len())) {
            let id = self.next_id();
            let mut t = Tile::new(id, *widget);
            t.point = point.clone();
            t.units = units.clone();
            if *widget == Widget::Bar && units.trim() == "%" {
                t.min = Some(0.0);
                t.max = Some(100.0);
            }
            t.y = self.rows();
            self.tiles.push(t);
            ids.push(id);
        }
        if !ids.is_empty() {
            self.auto_arrange();
        }
        ids
    }

    /// Tidy the whole dashboard: tiles grouped by widget type (headings, then indicators, numbers
    /// and text sharing rows, then gauges and bars, then charts), packed into rows left to right,
    /// each row's tiles given the same height and the row stretched
    /// (up to twice a tile's width) so its right edge lines up with the page. Order inside a
    /// group follows the current reading order, so a manual arrangement is respected.
    pub fn auto_arrange(&mut self) {
        let cols = self.cols;
        let rank = |w: Widget| match w {
            Widget::Label => 0,
            Widget::Indicator => 1,
            Widget::Numeric => 2,
            Widget::Text => 3,
            Widget::Gauge => 4,
            Widget::Bar => 5,
            Widget::Chart => 6,
        };
        let mut order: Vec<usize> = (0..self.tiles.len()).collect();
        order.sort_by_key(|i| {
            let t = &self.tiles[*i];
            (rank(t.widget), t.y, t.x, t.id)
        });
        for i in &order {
            Self::clamp_tile(cols, &mut self.tiles[*i]);
        }
        let mut y = 0;
        let mut k = 0;
        while k < order.len() {
            // One row: same size class (headings / small readouts / gauges & bars / charts), as
            // many as fit side by side.
            let class = |w: Widget| match rank(w) {
                0 => 0,
                1..=3 => 1,
                4 | 5 => 2,
                _ => 3,
            };
            let group = class(self.tiles[order[k]].widget);
            let mut row = vec![order[k]];
            let mut used = self.tiles[order[k]].w;
            while k + row.len() < order.len() {
                let next = order[k + row.len()];
                if class(self.tiles[next].widget) != group || used + self.tiles[next].w > cols {
                    break;
                }
                used += self.tiles[next].w;
                row.push(next);
            }
            k += row.len();
            // Stretch to fill the width, capped at 2× the default sizes so a lone tile doesn't
            // become a banner (and arranging twice gives the same result).
            let natural: u32 = row.iter().map(|i| default_size(self.tiles[*i].widget).0).sum();
            let target = cols.min(used.max(natural * 2));
            let mut widths: Vec<u32> = row.iter().map(|i| self.tiles[*i].w * target / used).collect();
            let mut spare = target - widths.iter().sum::<u32>();
            for w in widths.iter_mut() {
                if spare == 0 {
                    break;
                }
                *w += 1;
                spare -= 1;
            }
            let h = row.iter().map(|i| self.tiles[*i].h).max().unwrap_or(1);
            let mut x = 0;
            for (i, w) in row.iter().zip(widths) {
                let t = &mut self.tiles[*i];
                t.x = x;
                t.y = y;
                t.w = w;
                t.h = h;
                x += w;
            }
            y += h;
        }
    }

    /// Tile ids in reading order (top to bottom, left to right), for the small-screen stack.
    pub fn reading_order(&self) -> Vec<u32> {
        let mut v: Vec<&Tile> = self.tiles.iter().collect();
        v.sort_by_key(|t| (t.y, t.x, t.id));
        v.into_iter().map(|t| t.id).collect()
    }

    /// New tile size for a widget (re-exported for callers that only hold a layout).
    pub fn default_tile_size(widget: Widget) -> (u32, u32) {
        default_size(widget)
    }
}

/// Which edge of `r` a point at `(fx, fy)` (fractions 0..1 inside it) is closest to, or `None`
/// when it is in the middle (the swap zone).
pub fn drop_zone(fx: f32, fy: f32) -> Option<Edge> {
    const BAND: f32 = 0.28;
    let (dl, dr, dt, db) = (fx, 1.0 - fx, fy, 1.0 - fy);
    let m = dl.min(dr).min(dt).min(db);
    if m > BAND {
        return None;
    }
    Some(if m == dl {
        Edge::Left
    } else if m == dr {
        Edge::Right
    } else if m == dt {
        Edge::Top
    } else {
        Edge::Bottom
    })
}

/// Snap a pixel distance to whole cells (rounding to nearest).
pub fn snap_cells(px: f32, cell: f32) -> i32 {
    (px / cell).round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> Layout {
        Layout::default()
    }

    #[test]
    fn auto_arrange_groups_by_type_and_fills_rows() {
        let mut l = layout();
        let pts: Vec<(String, Widget, String)> = (0..7)
            .map(|i| (format!("n{i}"), Widget::Numeric, String::new()))
            .chain([("run".to_string(), Widget::Indicator, String::new()), ("lvl".to_string(), Widget::Bar, "%".to_string())])
            .collect();
        let ids = l.add_points(&pts);
        assert_eq!(ids.len(), 9);
        no_overlaps(&l);
        // Indicator first, then numerics in the same row while they fit (3 + 5 × 4 = 23 → 24).
        let run = l.tiles.iter().find(|t| t.point == "run").unwrap();
        assert_eq!((run.x, run.y), (0, 0));
        let row0: Vec<&Tile> = l.tiles.iter().filter(|t| t.y == 0).collect();
        assert_eq!(row0.len(), 6);
        assert_eq!(row0.iter().map(|t| t.w).sum::<u32>(), 24, "row filled edge to edge");
        assert!(row0.iter().all(|t| t.h == 3), "one height per row");
        // The two numerics left over share the next row, stretched to at most twice their width.
        let row1: Vec<&Tile> = l.tiles.iter().filter(|t| t.y == 3).collect();
        assert_eq!((row1.len(), row1.iter().map(|t| t.w).sum::<u32>()), (2, 16));
        let bar = l.tiles.iter().find(|t| t.point == "lvl").unwrap();
        assert_eq!((bar.min, bar.max), (Some(0.0), Some(100.0)));
        assert!(bar.y >= l.tiles.iter().filter(|t| t.widget == Widget::Numeric).map(|t| t.y + t.h).max().unwrap());
        // Adding again skips points already shown.
        assert!(l.add_points(&pts).is_empty());
        let before = l.clone();
        l.auto_arrange();
        assert_eq!(l, before, "arranging an arranged layout changes nothing");
    }

    fn no_overlaps(l: &Layout) {
        for (i, a) in l.tiles.iter().enumerate() {
            assert!(a.x + a.w <= l.cols, "tile {} sticks out: {a:?}", a.id);
            for b in &l.tiles[i + 1..] {
                assert!(!Rect::of(a).overlaps(&Rect::of(b)), "{} overlaps {}: {a:?} {b:?}", a.id, b.id);
            }
        }
    }

    #[test]
    fn new_tiles_take_the_first_free_spot() {
        let mut l = layout();
        let a = l.add(Widget::Numeric, None); // 4×3 at 0,0
        let b = l.add(Widget::Numeric, None);
        assert_eq!((l.tile(a).unwrap().x, l.tile(a).unwrap().y), (0, 0));
        assert_eq!((l.tile(b).unwrap().x, l.tile(b).unwrap().y), (4, 0));
        for _ in 0..5 {
            l.add(Widget::Numeric, None); // four more fill the row (6 × 4 = 24), the fifth wraps
        }
        let last = l.tiles.last().unwrap();
        assert_eq!((last.x, last.y), (0, 3));
        no_overlaps(&l);
    }

    #[test]
    fn gaps_are_kept_and_found_again() {
        let mut l = layout();
        let a = l.add(Widget::Numeric, Some((0, 0)));
        let b = l.add(Widget::Numeric, Some((8, 0)));
        assert_eq!(l.tile(b).unwrap().x, 8, "free placement keeps the gap at 4..8");
        assert_eq!(l.find_free(4, 3), (4, 0), "…and a new tile drops into it");
        l.remove(a);
        assert_eq!(l.find_free(4, 3), (0, 0));
    }

    #[test]
    fn dropping_on_a_tile_pushes_it_down() {
        let mut l = layout();
        let a = l.add(Widget::Numeric, Some((0, 0)));
        let b = l.add(Widget::Numeric, Some((0, 3)));
        let c = l.add(Widget::Numeric, Some((10, 0)));
        l.move_to(c, 0, 0); // lands on a; a goes below c, b below a
        assert_eq!((l.tile(c).unwrap().x, l.tile(c).unwrap().y), (0, 0));
        assert_eq!(l.tile(a).unwrap().y, 3);
        assert_eq!(l.tile(b).unwrap().y, 6, "cascade");
        no_overlaps(&l);
    }

    #[test]
    fn moves_are_clamped_to_the_columns() {
        let mut l = layout();
        let a = l.add(Widget::Numeric, Some((0, 0)));
        l.move_to(a, 99, 2);
        assert_eq!(l.tile(a).unwrap().x, 24 - 4);
    }

    #[test]
    fn resize_enforces_minimums_and_pushes() {
        let mut l = layout();
        let g = l.add(Widget::Gauge, Some((0, 0)));
        let n = l.add(Widget::Numeric, Some((0, 4)));
        l.resize_to(g, Rect { x: 0, y: 0, w: 1, h: 1 });
        assert_eq!((l.tile(g).unwrap().w, l.tile(g).unwrap().h), (3, 3), "gauge keeps its minimum");
        l.resize_to(g, Rect { x: 0, y: 0, w: 6, h: 6 });
        assert!(l.tile(n).unwrap().y >= 6, "numeric pushed below the grown gauge");
        l.resize_to(g, Rect { x: 20, y: 0, w: 10, h: 4 });
        assert!(l.tile(g).unwrap().x + l.tile(g).unwrap().w <= 24);
        no_overlaps(&l);
    }

    #[test]
    fn swap_exchanges_rectangles_and_respects_minimums() {
        let mut l = layout();
        let n = l.add(Widget::Numeric, Some((0, 0))); // 4×3
        let c = l.add(Widget::Chart, Some((6, 0))); // 8×5
        assert!(l.swap(n, c));
        assert_eq!((l.tile(n).unwrap().x, l.tile(n).unwrap().w), (6, 8));
        assert_eq!((l.tile(c).unwrap().x, l.tile(c).unwrap().w), (0, 4));
        // Chart minimum is 4×3: it fit exactly, but a 2×2 slot would grow.
        let tiny = l.add(Widget::Indicator, Some((0, 10))); // 3×2
        l.tile_mut(tiny).unwrap().w = 2;
        assert!(l.swap(c, tiny));
        assert!(l.tile(c).unwrap().w >= 4 && l.tile(c).unwrap().h >= 3);
        no_overlaps(&l);
        assert!(!l.swap(n, n));
    }

    #[test]
    fn split_drop_shares_the_target_space() {
        let mut l = layout();
        let big = l.add_rect(Widget::Chart, Rect { x: 0, y: 0, w: 12, h: 6 });
        let n = l.add(Widget::Numeric, Some((14, 0)));
        assert!(l.split_drop(n, big, Edge::Right));
        let (b, m) = (l.tile(big).unwrap(), l.tile(n).unwrap());
        assert_eq!((b.x, b.w, b.h), (0, 6, 6));
        assert_eq!((m.x, m.w, m.y, m.h), (6, 6, 0, 6), "numeric takes the right half");
        no_overlaps(&l);
        // Top / bottom edges halve the height.
        let n2 = l.add(Widget::Numeric, Some((14, 0)));
        assert!(l.split_drop(n2, big, Edge::Bottom));
        let (b, m) = (l.tile(big).unwrap(), l.tile(n2).unwrap());
        assert_eq!((b.h, m.y, m.h), (3, 3, 3));
        no_overlaps(&l);
    }

    #[test]
    fn split_drop_is_refused_when_too_small() {
        let mut l = layout();
        let small = l.add_rect(Widget::Gauge, Rect { x: 0, y: 0, w: 4, h: 4 });
        let n = l.add(Widget::Numeric, Some((10, 0)));
        let before = l.clone();
        assert!(!l.split_drop(n, small, Edge::Left), "2 + 2 columns can't hold a gauge (min 3)");
        assert_eq!(l, before);
        assert!(!l.split_drop(n, n, Edge::Left));
    }

    #[test]
    fn reading_order_and_zones() {
        let mut l = layout();
        let a = l.add(Widget::Numeric, Some((10, 0)));
        let b = l.add(Widget::Numeric, Some((0, 0)));
        let c = l.add(Widget::Numeric, Some((0, 5)));
        assert_eq!(l.reading_order(), vec![b, a, c]);
        assert_eq!(drop_zone(0.5, 0.5), None);
        assert_eq!(drop_zone(0.05, 0.5), Some(Edge::Left));
        assert_eq!(drop_zone(0.95, 0.5), Some(Edge::Right));
        assert_eq!(drop_zone(0.5, 0.1), Some(Edge::Top));
        assert_eq!(drop_zone(0.5, 0.9), Some(Edge::Bottom));
        assert_eq!(snap_cells(49.0, 25.0), 2);
        assert_eq!(snap_cells(-30.0, 25.0), -1);
    }

    #[test]
    fn random_edits_never_leave_overlaps() {
        // Deterministic pseudo-random stress: any sequence of operations keeps the invariant.
        let mut l = layout();
        let mut s = 12345u32;
        let mut rnd = |n: u32| {
            s = s.wrapping_mul(1664525).wrapping_add(1013904223);
            (s >> 8) % n.max(1)
        };
        for step in 0..400 {
            let ids: Vec<u32> = l.tiles.iter().map(|t| t.id).collect();
            match rnd(6) {
                0 | 1 => {
                    let w = Widget::ALL[rnd(7) as usize];
                    l.add(w, if rnd(2) == 0 { None } else { Some((rnd(24), rnd(12))) });
                }
                2 if !ids.is_empty() => l.move_to(ids[rnd(ids.len() as u32) as usize], rnd(30), rnd(20)),
                3 if !ids.is_empty() => l.resize_to(ids[rnd(ids.len() as u32) as usize], Rect { x: rnd(24), y: rnd(10), w: rnd(14), h: rnd(8) }),
                4 if ids.len() > 1 => {
                    l.swap(ids[rnd(ids.len() as u32) as usize], ids[rnd(ids.len() as u32) as usize]);
                }
                5 if ids.len() > 1 => {
                    let e = [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom][rnd(4) as usize];
                    l.split_drop(ids[rnd(ids.len() as u32) as usize], ids[rnd(ids.len() as u32) as usize], e);
                }
                _ => {}
            }
            no_overlaps(&l);
            l.validate().unwrap_or_else(|e| panic!("step {step}: {e}"));
        }
    }
}
