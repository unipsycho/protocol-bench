//! Title-drag docking. The top of an area adds a tab; the other edges dock; the center swaps.

use egui::{Color32, Context, Id, LayerId, Order, Painter, Pos2, Rect, Rounding, Stroke, Vec2};

use crate::dock::DockPane;
use crate::layout::TileEdge;
use crate::theme::{
    self, tint, ThemePalette, DROP_THRESHOLD, EDGE_BAND, HEADER_H, MIN_PANE, RADIUS, SEP,
};

#[derive(Clone)]
pub struct PaneHit {
    /// Pane the area is showing. Drop commits name the area by this pane.
    pub pane: DockPane,
    /// Every tab in the area, including the active one.
    pub tabs: Vec<DockPane>,
    pub rect: Rect,
    /// Header strip. A drop here adds a tab.
    pub strip: Rect,
}

impl PaneHit {
    pub fn area(pane: DockPane, tabs: Vec<DockPane>, rect: Rect) -> Self {
        let strip = Rect::from_min_size(
            rect.min,
            Vec2::new(rect.width(), HEADER_H.min(rect.height())),
        );
        Self {
            pane,
            tabs,
            rect,
            strip,
        }
    }
}

#[derive(Clone, Copy)]
pub enum DropCommit {
    Swap {
        a: DockPane,
        b: DockPane,
    },
    /// Add `moving` to the tab strip of the area that holds `target`.
    Tab {
        moving: DockPane,
        target: DockPane,
    },
    Dock {
        moving: DockPane,
        target: DockPane,
        edge: TileEdge,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeZone {
    Center,
    West,
    East,
    North,
    South,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LandingKind {
    Swap,
    Tab,
    SplitLeft,
    SplitRight,
    SplitBottom,
    Blocked,
}

pub struct Landing {
    pub preview: Rect,
    pub label: &'static str,
    pub kind: LandingKind,
    pub commit: Option<DropCommit>,
}

const PAD: f32 = 32.0;
const INSET: f32 = 14.0;

/// Center swap pill plus the four dock pads.
pub fn favnyr_pads(rect: Rect) -> (Rect, [(EdgeZone, Rect); 4]) {
    let swap = Rect::from_center_size(rect.center(), Vec2::new(108.0, 34.0));
    let edges = [
        (
            EdgeZone::North,
            Rect::from_center_size(
                Pos2::new(rect.center().x, rect.top() + INSET + PAD * 0.5),
                Vec2::splat(PAD),
            ),
        ),
        (
            EdgeZone::South,
            Rect::from_center_size(
                Pos2::new(rect.center().x, rect.bottom() - INSET - PAD * 0.5),
                Vec2::splat(PAD),
            ),
        ),
        (
            EdgeZone::West,
            Rect::from_center_size(
                Pos2::new(rect.left() + INSET + PAD * 0.5, rect.center().y),
                Vec2::splat(PAD),
            ),
        ),
        (
            EdgeZone::East,
            Rect::from_center_size(
                Pos2::new(rect.right() - INSET - PAD * 0.5, rect.center().y),
                Vec2::splat(PAD),
            ),
        ),
    ];
    (swap, edges)
}

/// Pads win. The rest of the pane uses the nearest edge, unless the pointer is in the center.
pub fn favnyr_zone(rect: Rect, pos: Pos2) -> EdgeZone {
    let (swap, edges) = favnyr_pads(rect);
    for (zone, pad) in edges {
        if pad.expand(6.0).contains(pos) {
            return zone;
        }
    }
    if swap.expand(8.0).contains(pos) {
        return EdgeZone::Center;
    }
    let w = rect.width().max(1.0);
    let h = rect.height().max(1.0);
    let rx = ((pos.x - rect.min.x) / w).clamp(0.0, 1.0);
    let ry = ((pos.y - rect.min.y) / h).clamp(0.0, 1.0);
    if rx >= EDGE_BAND && rx <= 1.0 - EDGE_BAND && ry >= EDGE_BAND && ry <= 1.0 - EDGE_BAND {
        return EdgeZone::Center;
    }
    let mut best = EdgeZone::West;
    let mut best_d = rx;
    for (zone, dist) in [
        (EdgeZone::East, 1.0 - rx),
        (EdgeZone::North, ry),
        (EdgeZone::South, 1.0 - ry),
    ] {
        if dist < best_d {
            best = zone;
            best_d = dist;
        }
    }
    let _ = best_d;
    best
}

/// What a release would do if the pointer is over `hit`.
pub fn landing_at(moving: DockPane, hit: &PaneHit, pos: Pos2) -> Landing {
    let tabs = if hit.tabs.is_empty() {
        vec![hit.pane]
    } else {
        hit.tabs.clone()
    };
    let in_area = tabs.contains(&moving);
    // A lone pane has nowhere to dock onto itself.
    if in_area && tabs.len() == 1 {
        return blocked(hit.rect.shrink(4.0));
    }
    let strip = tab_strip(hit);
    let zone = favnyr_zone(hit.rect, pos);
    // The header and the top band both add a tab. They do not split the area.
    if strip.contains(pos) || zone == EdgeZone::North {
        if in_area {
            return blocked(strip);
        }
        return Landing {
            preview: strip,
            label: "Add tab",
            kind: LandingKind::Tab,
            commit: Some(DropCommit::Tab {
                moving,
                target: hit.pane,
            }),
        };
    }
    let too_small = match zone {
        EdgeZone::West | EdgeZone::East => hit.rect.width() < MIN_PANE * 2.0 + SEP,
        EdgeZone::South => hit.rect.height() < MIN_PANE * 2.0 + SEP,
        EdgeZone::North | EdgeZone::Center => false,
    };
    let anchor = if in_area {
        tabs.iter()
            .copied()
            .find(|pane| *pane != moving)
            .unwrap_or(hit.pane)
    } else {
        hit.pane
    };
    let (preview, label, kind, edge) = match zone {
        EdgeZone::Center => {
            if moving == hit.pane {
                return blocked(hit.rect.shrink(4.0));
            }
            (hit.rect.shrink(4.0), "Swap", LandingKind::Swap, None)
        }
        EdgeZone::South => (
            bottom_half(hit.rect),
            if too_small { "blocked" } else { "dock bottom" },
            if too_small {
                LandingKind::Blocked
            } else {
                LandingKind::SplitBottom
            },
            Some(TileEdge::South),
        ),
        EdgeZone::West => (
            left_half(hit.rect),
            if too_small { "blocked" } else { "dock left" },
            if too_small {
                LandingKind::Blocked
            } else {
                LandingKind::SplitLeft
            },
            Some(TileEdge::West),
        ),
        EdgeZone::East => (
            right_half(hit.rect),
            if too_small { "blocked" } else { "dock right" },
            if too_small {
                LandingKind::Blocked
            } else {
                LandingKind::SplitRight
            },
            Some(TileEdge::East),
        ),
        EdgeZone::North => return blocked(strip),
    };
    let commit = if kind == LandingKind::Blocked {
        None
    } else if kind == LandingKind::Swap {
        Some(DropCommit::Swap {
            a: moving,
            b: hit.pane,
        })
    } else if let Some(edge) = edge {
        Some(DropCommit::Dock {
            moving,
            target: anchor,
            edge,
        })
    } else {
        None
    };
    Landing {
        preview,
        label,
        kind,
        commit,
    }
}

fn blocked(preview: Rect) -> Landing {
    Landing {
        preview,
        label: "",
        kind: LandingKind::Blocked,
        commit: None,
    }
}

fn tab_strip(hit: &PaneHit) -> Rect {
    if hit.strip.width() > 1.0 && hit.strip.height() > 1.0 {
        hit.strip
    } else {
        Rect::from_min_size(
            hit.rect.min,
            Vec2::new(hit.rect.width(), HEADER_H.min(hit.rect.height())),
        )
    }
}

pub fn paint_ghost(ctx: &Context, pane: DockPane, pos: Pos2, src: Vec2, p: &ThemePalette) {
    let painter = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("pb_drag_ghost")));
    let ghost = Rect::from_min_size(
        Pos2::new(pos.x + 16.0, pos.y + 12.0),
        Vec2::new(src.x.clamp(220.0, 420.0), src.y.clamp(140.0, 280.0)),
    );
    let ink = pane.ink(p.is_light);
    painter.rect_filled(ghost, Rounding::same(RADIUS), tint(p.pane, 230));
    painter.rect_stroke(ghost, Rounding::same(RADIUS), Stroke::new(1.5, ink));
    let header = Rect::from_min_size(ghost.min, Vec2::new(ghost.width(), HEADER_H));
    painter.rect_filled(
        header,
        Rounding {
            nw: RADIUS,
            ne: RADIUS,
            sw: 0.0,
            se: 0.0,
        },
        p.header_top,
    );
    let cap = Rect::from_center_size(
        Pos2::new(header.min.x + 12.0, header.center().y),
        Vec2::new(3.0, 16.0),
    );
    painter.rect_filled(cap, Rounding::same(1.5), ink);
    let icon = Rect::from_center_size(
        Pos2::new(header.min.x + 32.0, header.center().y),
        Vec2::splat(16.0),
    );
    crate::art::paint_pane_icon(&painter, icon, pane, ink);
    painter.text(
        Pos2::new(header.min.x + 48.0, header.center().y),
        egui::Align2::LEFT_CENTER,
        pane.title(),
        theme::font_semi(15.0),
        p.text,
    );
}

pub fn paint_landing(ctx: &Context, hit: Rect, landing: &Landing, p: &ThemePalette) {
    let painter = ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("pb_favnyr")));
    let blocked = landing.kind == LandingKind::Blocked;
    let accent = if blocked { p.err } else { p.accent };
    let splitting = matches!(
        landing.kind,
        LandingKind::SplitLeft | LandingKind::SplitRight | LandingKind::SplitBottom
    );
    let marking = splitting || landing.kind == LandingKind::Tab;
    painter.rect_filled(
        hit,
        Rounding::same(RADIUS),
        Color32::from_black_alpha(if marking { 70 } else { 40 }),
    );
    if marking {
        painter.rect_filled(landing.preview, Rounding::same(6.0), tint(accent, 80));
        painter.rect_stroke(
            landing.preview,
            Rounding::same(6.0),
            Stroke::new(2.0, accent),
        );
    }
    let (swap, edges) = favnyr_pads(hit);
    for (zone, pad) in edges {
        let kind = match zone {
            EdgeZone::North => LandingKind::Tab,
            EdgeZone::South => LandingKind::SplitBottom,
            EdgeZone::West => LandingKind::SplitLeft,
            EdgeZone::East => LandingKind::SplitRight,
            EdgeZone::Center => LandingKind::Swap,
        };
        let active = landing.kind == kind;
        painter.circle_filled(
            pad.center(),
            PAD * 0.5,
            if active {
                tint(accent, 220)
            } else {
                tint(p.bar, 210)
            },
        );
        painter.circle_stroke(
            pad.center(),
            PAD * 0.5,
            Stroke::new(1.5, if active { accent } else { p.line }),
        );
        if zone == EdgeZone::North {
            paint_tab_glyph(
                &painter,
                pad.center(),
                if active { p.text } else { p.muted },
            );
        } else {
            paint_chevron(
                &painter,
                pad.center(),
                zone,
                if active { p.text } else { p.muted },
            );
        }
    }
    painter.rect_filled(
        swap,
        Rounding::same(8.0),
        if landing.kind == LandingKind::Swap {
            tint(accent, 220)
        } else {
            tint(p.bar, 230)
        },
    );
    painter.rect_stroke(
        swap,
        Rounding::same(8.0),
        Stroke::new(
            1.5,
            if landing.kind == LandingKind::Swap {
                accent
            } else {
                p.line
            },
        ),
    );
    painter.text(
        swap.center(),
        egui::Align2::CENTER_CENTER,
        "Swap",
        theme::font_semi(13.0),
        p.text,
    );
    if !landing.label.is_empty() && landing.kind != LandingKind::Swap {
        let galley = ctx.fonts(|fonts| {
            fonts.layout_no_wrap(landing.label.to_owned(), theme::font_ui(12.5), p.text)
        });
        let chip = Rect::from_center_size(
            landing.preview.center(),
            galley.size() + Vec2::new(20.0, 10.0),
        );
        painter.rect_filled(chip, Rounding::same(6.0), p.bar);
        painter.rect_stroke(chip, Rounding::same(6.0), Stroke::new(1.0, accent));
        painter.galley(chip.min + Vec2::new(10.0, 5.0), galley, p.text);
    }
}

/// Two sheets, the front one raised. This pad adds a tab instead of splitting.
fn paint_tab_glyph(painter: &Painter, center: Pos2, color: Color32) {
    let stroke = Stroke::new(1.4, color);
    let back = Rect::from_center_size(center + Vec2::new(2.0, 1.0), Vec2::new(11.0, 8.0));
    painter.rect_stroke(back, Rounding::same(1.5), stroke);
    let front = Rect::from_min_max(
        Pos2::new(center.x - 7.0, center.y - 5.0),
        Pos2::new(center.x + 3.0, center.y + 4.0),
    );
    painter.rect_filled(front, Rounding::same(1.5), color);
}

fn paint_chevron(painter: &Painter, center: Pos2, zone: EdgeZone, color: Color32) {
    let dir = match zone {
        EdgeZone::North => Vec2::new(0.0, -1.0),
        EdgeZone::South => Vec2::new(0.0, 1.0),
        EdgeZone::West => Vec2::new(-1.0, 0.0),
        EdgeZone::East => Vec2::new(1.0, 0.0),
        EdgeZone::Center => return,
    };
    let tip = center + dir * 6.0;
    let side = Vec2::new(-dir.y, dir.x);
    let base = center - dir * 3.0;
    painter.add(egui::Shape::convex_polygon(
        vec![tip, base + side * 5.0, base - side * 5.0],
        color,
        Stroke::NONE,
    ));
}

fn bottom_half(rect: Rect) -> Rect {
    Rect::from_min_max(Pos2::new(rect.min.x, rect.center().y), rect.max)
}
fn left_half(rect: Rect) -> Rect {
    Rect::from_min_max(rect.min, Pos2::new(rect.center().x, rect.max.y))
}
fn right_half(rect: Rect) -> Rect {
    Rect::from_min_max(Pos2::new(rect.center().x, rect.min.y), rect.max)
}

/// Drag distance accumulated for one pane. Returns true once the gesture is armed.
pub fn arm_drag(ctx: &Context, pane: DockPane, delta: Vec2, source: Vec2) -> bool {
    let key = Id::new(("pb_drag_acc", pane.id()));
    let mut acc = ctx.data(|data| data.get_temp::<f32>(key).unwrap_or(0.0));
    acc += delta.length();
    ctx.data_mut(|data| data.insert_temp(key, acc));
    if acc >= DROP_THRESHOLD {
        ctx.data_mut(|data| data.insert_temp(Id::new("pb_drag_src_size"), source));
        true
    } else {
        false
    }
}

pub fn clear_drag_arm(ctx: &Context, pane: DockPane) {
    ctx.data_mut(|data| data.remove_temp::<f32>(Id::new(("pb_drag_acc", pane.id()))));
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;

    fn hit() -> PaneHit {
        PaneHit::area(
            DockPane::Points,
            vec![DockPane::Points],
            Rect::from_min_max(pos2(0.0, 0.0), pos2(400.0, 300.0)),
        )
    }

    #[test]
    fn pads_and_center_pick_a_zone() {
        let rect = hit().rect;
        assert_eq!(favnyr_zone(rect, rect.center()), EdgeZone::Center);
        assert_eq!(favnyr_zone(rect, pos2(200.0, 30.0)), EdgeZone::North);
        assert_eq!(favnyr_zone(rect, pos2(30.0, 150.0)), EdgeZone::West);
        assert_eq!(favnyr_zone(rect, pos2(370.0, 150.0)), EdgeZone::East);
    }

    #[test]
    fn same_pane_does_not_commit() {
        let hit = hit();
        let landing = landing_at(DockPane::Points, &hit, hit.rect.center());
        assert!(landing.commit.is_none());
        assert_eq!(landing.kind, LandingKind::Blocked);
    }

    #[test]
    fn center_swaps_and_the_top_adds_a_tab() {
        let hit = PaneHit::area(
            DockPane::Points,
            vec![DockPane::Points],
            Rect::from_min_max(pos2(0.0, 0.0), pos2(500.0, 420.0)),
        );
        let swap = landing_at(DockPane::Modbus, &hit, hit.rect.center());
        assert!(matches!(swap.commit, Some(DropCommit::Swap { .. })));
        let tab = landing_at(DockPane::Modbus, &hit, pos2(250.0, 30.0));
        assert_eq!(tab.kind, LandingKind::Tab);
        assert!(matches!(
            tab.commit,
            Some(DropCommit::Tab {
                moving: DockPane::Modbus,
                target: DockPane::Points,
            })
        ));
        let below = landing_at(DockPane::Modbus, &hit, pos2(250.0, 390.0));
        assert!(matches!(
            below.commit,
            Some(DropCommit::Dock {
                edge: TileEdge::South,
                ..
            })
        ));
    }

    #[test]
    fn a_tab_strip_accepts_another_areas_pane() {
        let hit = PaneHit::area(
            DockPane::Points,
            vec![DockPane::Points, DockPane::Mappings],
            Rect::from_min_max(pos2(0.0, 0.0), pos2(500.0, 420.0)),
        );
        let onto = landing_at(DockPane::Modbus, &hit, hit.strip.center());
        assert!(matches!(onto.commit, Some(DropCommit::Tab { .. })));
        let already = landing_at(DockPane::Mappings, &hit, hit.strip.center());
        assert!(already.commit.is_none());
        let out = landing_at(DockPane::Mappings, &hit, pos2(250.0, 390.0));
        assert!(matches!(
            out.commit,
            Some(DropCommit::Dock {
                moving: DockPane::Mappings,
                target: DockPane::Points,
                edge: TileEdge::South,
            })
        ));
    }
}
