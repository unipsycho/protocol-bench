//! Painted mark, rail icons, hero displays, and chrome glyphs. No image assets.

use egui::{Color32, Painter, Pos2, Rect, Rounding, Shape, Stroke, Vec2};

use crate::dock::DockPane;
use crate::theme::{self, font_ui, scope, tint, Proto, ThemePalette};

const TAU: f32 = std::f32::consts::TAU;

pub fn paint_mark(painter: &Painter, rect: Rect, p: &ThemePalette) {
    // Square badge. Three lanes: serial trace, Modbus cells, MQTT nodes.
    let side = rect.width().min(rect.height());
    let rect = Rect::from_center_size(rect.center(), Vec2::splat(side));
    let round = Rounding::same(side * 0.22);
    painter.rect_filled(rect, round, scope::BG);
    painter.rect_stroke(
        rect,
        round,
        Stroke::new((side * 0.045).clamp(1.2, 2.8), p.accent),
    );

    let inner = rect.shrink(side * 0.20);
    let gap = inner.height() * 0.10;
    let lane_h = (inner.height() - gap * 2.0) / 3.0;
    let stroke_w = (side * 0.038).clamp(1.05, 2.3);

    let lane0 = Rect::from_min_size(inner.min, Vec2::new(inner.width(), lane_h));
    wave(
        painter,
        lane0.min.x,
        lane0.max.x,
        lane0.center().y,
        lane_h * 0.28,
        1.6,
        Stroke::new(stroke_w, Proto::STREAMS),
    );

    let lane1_y = inner.min.y + lane_h + gap;
    paint_cells(
        painter,
        inner.min.x,
        inner.max.x,
        lane1_y,
        lane_h,
        stroke_w,
        Proto::MODBUS,
    );

    let lane2_y = lane1_y + lane_h + gap;
    let cy = lane2_y + lane_h * 0.5;
    let radius = (lane_h * 0.20).clamp(1.4, 5.5);
    let left = Pos2::new(inner.min.x + inner.width() * 0.18, cy);
    let mid = Pos2::new(inner.center().x, cy);
    let right = Pos2::new(inner.max.x - inner.width() * 0.18, cy);
    let link = Stroke::new(stroke_w, Proto::MQTT);
    painter.line_segment([left, mid], link);
    painter.line_segment([mid, right], link);
    for center in [left, mid, right] {
        painter.circle_filled(center, radius, Proto::MQTT);
    }
}

fn paint_cells(
    painter: &Painter,
    x0: f32,
    x1: f32,
    y: f32,
    lane_h: f32,
    stroke_w: f32,
    ink: Color32,
) {
    let count = 5;
    let cell = (lane_h * 0.62).min((x1 - x0) / (count as f32 * 1.35));
    let gap = cell * 0.32;
    let total = count as f32 * cell + (count - 1) as f32 * gap;
    let mut x = (x0 + x1) * 0.5 - total * 0.5;
    let top = y + (lane_h - cell) * 0.5;
    for i in 0..count {
        let rect = Rect::from_min_size(Pos2::new(x, top), Vec2::splat(cell.max(1.0)));
        let round = Rounding::same(cell * 0.22);
        if i % 2 == 1 {
            painter.rect_filled(rect, round, ink);
        } else {
            painter.rect_stroke(rect, round, Stroke::new(stroke_w * 0.85, ink));
        }
        x += cell + gap;
    }
}

pub fn paint_pane_icon(painter: &Painter, rect: Rect, pane: DockPane, ink: Color32) {
    let stroke = Stroke::new(
        (rect.width().min(rect.height()) * 0.075).clamp(1.15, 2.2),
        ink,
    );
    match pane {
        DockPane::Project => icon_folder(painter, rect, stroke),
        DockPane::Hubs => icon_hubs(painter, rect, stroke, ink),
        DockPane::Points => icon_hub(painter, rect, stroke, ink),
        DockPane::Calculations => icon_map(painter, rect, stroke, ink),
        DockPane::Mappings => icon_map(painter, rect, stroke, ink),
        DockPane::Traffic => icon_traffic(painter, rect, stroke),
        DockPane::Streams => icon_streams(painter, rect, stroke),
        DockPane::Modbus => icon_modbus(painter, rect, stroke, ink),
        DockPane::Mqtt => icon_mqtt(painter, rect, stroke, ink),
        DockPane::OpcUa => icon_opc(painter, rect, stroke),
        DockPane::Files => icon_file(painter, rect, stroke),
    }
}

pub fn paint_hero(painter: &Painter, pane: DockPane, rect: Rect) {
    match pane {
        DockPane::Hubs => hero_hubs(painter, rect),
        DockPane::Project => paint_mark(
            painter,
            Rect::from_center_size(rect.center(), Vec2::splat(rect.height() * 0.72)),
            &theme::AppTheme::Copper.palette(),
        ),
        DockPane::Points => hero_points(painter, rect),
        DockPane::Calculations => hero_mappings(painter, rect),
        DockPane::Mappings => hero_mappings(painter, rect),
        DockPane::Traffic => hero_traffic(painter, rect),
        DockPane::Streams => hero_streams(painter, rect),
        DockPane::Modbus => hero_modbus(painter, rect),
        DockPane::Mqtt => hero_mqtt(painter, rect),
        DockPane::OpcUa => hero_opc(painter, rect),
        DockPane::Files => hero_files(painter, rect),
    }
}

pub enum ChromeIcon {
    Gear,
    Close,
    Maximize,
    Restore,
}

pub fn chrome_button(
    ui: &mut egui::Ui,
    icon: ChromeIcon,
    tip: &str,
    active: bool,
    p: &ThemePalette,
    size: f32,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::click());
    let painter = ui.painter();
    let fill = if active {
        tint(p.accent, 42)
    } else if resp.hovered() {
        tint(p.accent, 18)
    } else {
        Color32::TRANSPARENT
    };
    if fill.a() > 0 {
        painter.rect_filled(rect, Rounding::same(6.0), fill);
    }
    let ink = if active || resp.hovered() {
        p.accent
    } else {
        p.text
    };
    let glyph = Rect::from_center_size(rect.center(), Vec2::splat(size * 0.52));
    match icon {
        ChromeIcon::Gear => paint_gear(painter, glyph, ink),
        ChromeIcon::Close => paint_close(painter, glyph, ink),
        ChromeIcon::Maximize => paint_max(painter, glyph, ink, false),
        ChromeIcon::Restore => paint_max(painter, glyph, ink, true),
    }
    resp.on_hover_text(tip)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn paint_check(painter: &Painter, center: Pos2, color: Color32) {
    let stroke = Stroke::new(1.7, color);
    painter.line_segment(
        [
            Pos2::new(center.x - 4.0, center.y),
            Pos2::new(center.x - 1.2, center.y + 3.2),
        ],
        stroke,
    );
    painter.line_segment(
        [
            Pos2::new(center.x - 1.2, center.y + 3.2),
            Pos2::new(center.x + 4.6, center.y - 3.4),
        ],
        stroke,
    );
}

fn hero_hubs(painter: &Painter, r: Rect) {
    let hub = Rect::from_center_size(r.center(), Vec2::new(r.width() * 0.28, r.height() * 0.42));
    painter.rect_stroke(hub, Rounding::same(6.0), Stroke::new(1.6, Proto::COPPER));
    painter.circle_filled(hub.center(), 4.5, Proto::COPPER);
    let left = Pos2::new(r.min.x + r.width() * 0.14, r.center().y);
    let right = Pos2::new(r.max.x - r.width() * 0.14, r.center().y);
    painter.circle_filled(left, 4.0, Proto::MODBUS);
    painter.circle_filled(right, 4.0, Proto::MQTT);
    painter.line_segment(
        [left, Pos2::new(hub.min.x, hub.center().y)],
        Stroke::new(1.4, Proto::MODBUS),
    );
    painter.line_segment(
        [Pos2::new(hub.max.x, hub.center().y), right],
        Stroke::new(1.4, Proto::MQTT),
    );
}

fn icon_folder(painter: &Painter, r: Rect, stroke: Stroke) {
    let pts = [
        n(r, 0.12, 0.36),
        n(r, 0.12, 0.82),
        n(r, 0.88, 0.82),
        n(r, 0.88, 0.36),
        n(r, 0.50, 0.36),
        n(r, 0.42, 0.18),
        n(r, 0.12, 0.18),
    ];
    for pair in pts.windows(2) {
        painter.line_segment([pair[0], pair[1]], stroke);
    }
    painter.line_segment([pts[6], pts[0]], stroke);
}

/// Sources into a hub, then out. Distinct from the points hub.
fn icon_hubs(painter: &Painter, r: Rect, stroke: Stroke, ink: Color32) {
    let hub = nrect(r, 0.34, 0.28, 0.32, 0.44);
    painter.rect_stroke(hub, Rounding::same(2.0), stroke);
    painter.circle_filled(hub.center(), hub.width().min(hub.height()) * 0.16, ink);
    let left = n(r, 0.12, 0.50);
    let right = n(r, 0.88, 0.50);
    painter.circle_filled(left, r.width() * 0.07, ink);
    painter.circle_filled(right, r.width() * 0.07, ink);
    painter.line_segment([left, Pos2::new(hub.min.x, hub.center().y)], stroke);
    painter.line_segment([Pos2::new(hub.max.x, hub.center().y), right], stroke);
}

fn icon_hub(painter: &Painter, r: Rect, stroke: Stroke, ink: Color32) {
    let c = n(r, 0.50, 0.50);
    let rad = r.width().min(r.height()) * 0.20;
    painter.circle_filled(c, rad * 0.42, ink);
    painter.circle_stroke(c, rad, stroke);
    for (x, y) in [(0.18, 0.22), (0.82, 0.24), (0.78, 0.80), (0.22, 0.78)] {
        let p = n(r, x, y);
        let dir = p - c;
        let mag = dir.length().max(0.001);
        let unit = dir / mag;
        painter.line_segment([c + unit * rad, p], stroke);
        painter.circle_filled(p, rad * 0.32, ink);
    }
}

fn icon_map(painter: &Painter, r: Rect, stroke: Stroke, ink: Color32) {
    let left = nrect(r, 0.04, 0.26, 0.30, 0.48);
    let right = nrect(r, 0.66, 0.26, 0.30, 0.48);
    painter.rect_stroke(left, Rounding::same(1.5), stroke);
    painter.rect_stroke(right, Rounding::same(1.5), stroke);
    let y = r.center().y;
    let tail = Pos2::new(left.max.x + 1.0, y);
    let tip = Pos2::new(right.min.x - 1.0, y);
    painter.line_segment([tail, tip], stroke);
    arrow_head(painter, tip, Vec2::new(1.0, 0.0), r.width() * 0.16, ink);
}

fn icon_traffic(painter: &Painter, r: Rect, stroke: Stroke) {
    for (i, w) in [0.78, 0.48, 0.66, 0.36].iter().enumerate() {
        let y = 0.22 + i as f32 * 0.18;
        painter.line_segment(
            [n(r, 0.12, y), n(r, 0.12 + w, y)],
            Stroke::new(stroke.width + 0.4, stroke.color),
        );
    }
}

fn icon_streams(painter: &Painter, r: Rect, stroke: Stroke) {
    painter.rect_stroke(
        nrect(r, 0.08, 0.28, 0.28, 0.44),
        Rounding::same(2.0),
        stroke,
    );
    for row in 0..2 {
        for col in 0..2 {
            painter.circle_filled(
                Pos2::new(
                    n(r, 0.16, 0.40).x + col as f32 * r.width() * 0.10,
                    n(r, 0.16, 0.40).y + row as f32 * r.height() * 0.14,
                ),
                1.2,
                stroke.color,
            );
        }
    }
    wave(
        painter,
        n(r, 0.40, 0.5).x,
        n(r, 0.92, 0.5).x,
        r.center().y,
        r.height() * 0.16,
        1.4,
        stroke,
    );
}

fn icon_modbus(painter: &Painter, r: Rect, stroke: Stroke, ink: Color32) {
    for row in 0..3 {
        for col in 0..3 {
            let cell = nrect(
                r,
                0.12 + col as f32 * 0.26,
                0.12 + row as f32 * 0.26,
                0.20,
                0.20,
            );
            if row == 1 && col == 1 {
                painter.rect_filled(cell, Rounding::same(1.5), ink);
            } else {
                painter.rect_stroke(cell, Rounding::same(1.5), stroke);
            }
        }
    }
}

fn icon_mqtt(painter: &Painter, r: Rect, stroke: Stroke, ink: Color32) {
    let nodes = [n(r, 0.50, 0.22), n(r, 0.20, 0.74), n(r, 0.80, 0.74)];
    painter.line_segment([nodes[0], nodes[1]], stroke);
    painter.line_segment([nodes[0], nodes[2]], stroke);
    painter.line_segment([nodes[1], nodes[2]], stroke);
    for (i, node) in nodes.iter().enumerate() {
        let rad = if i == 0 {
            r.width() * 0.12
        } else {
            r.width() * 0.09
        };
        painter.circle_stroke(*node, rad, stroke);
        painter.circle_filled(*node, rad * 0.35, ink);
    }
}

fn icon_opc(painter: &Painter, r: Rect, stroke: Stroke) {
    let root = nrect(r, 0.34, 0.12, 0.32, 0.22);
    painter.rect_stroke(root, Rounding::same(1.5), stroke);
    painter.line_segment([root.center(), n(r, 0.50, 0.48)], stroke);
    painter.line_segment([n(r, 0.24, 0.48), n(r, 0.76, 0.48)], stroke);
    painter.line_segment([n(r, 0.24, 0.48), n(r, 0.24, 0.62)], stroke);
    painter.line_segment([n(r, 0.76, 0.48), n(r, 0.76, 0.62)], stroke);
    painter.circle_stroke(n(r, 0.24, 0.72), r.width() * 0.10, stroke);
    painter.rect_stroke(
        nrect(r, 0.64, 0.62, 0.24, 0.22),
        Rounding::same(1.5),
        stroke,
    );
}

fn icon_file(painter: &Painter, r: Rect, stroke: Stroke) {
    let page = nrect(r, 0.22, 0.10, 0.52, 0.80);
    painter.rect_stroke(page, Rounding::same(1.5), stroke);
    let fold = [n(r, 0.56, 0.10), n(r, 0.74, 0.28), n(r, 0.56, 0.28)];
    painter.line_segment([fold[0], fold[1]], stroke);
    painter.line_segment([fold[1], fold[2]], stroke);
    for i in 0..3 {
        let y = 0.46 + i as f32 * 0.12;
        painter.line_segment([n(r, 0.32, y), n(r, 0.64, y)], stroke);
    }
}

fn hero_streams(painter: &Painter, r: Rect) {
    let plug = nrect(r, 0.06, 0.22, 0.18, 0.56);
    painter.rect_stroke(plug, Rounding::same(4.0), stroke_of(Proto::STREAMS, 1.6));
    for row in 0..3 {
        for col in 0..2 {
            painter.circle_filled(
                Pos2::new(
                    plug.min.x + plug.width() * (0.32 + col as f32 * 0.36),
                    plug.min.y + plug.height() * (0.25 + row as f32 * 0.25),
                ),
                2.4,
                Proto::MODBUS,
            );
        }
    }
    wave(
        painter,
        n(r, 0.28, 0.5).x,
        n(r, 0.70, 0.5).x,
        r.center().y,
        r.height() * 0.16,
        2.2,
        stroke_of(Proto::STREAMS, 1.8),
    );
    let sock = nrect(r, 0.74, 0.26, 0.20, 0.48);
    painter.rect_stroke(sock, Rounding::same(4.0), stroke_of(Proto::TEAL, 1.6));
    for i in 0..4 {
        let y = sock.min.y + sock.height() * (0.22 + i as f32 * 0.18);
        painter.rect_filled(
            Rect::from_center_size(
                Pos2::new(sock.center().x, y),
                Vec2::new(sock.width() * 0.46, 3.0),
            ),
            Rounding::same(1.0),
            Proto::TEAL,
        );
    }
    if r.width() > 280.0 {
        label_at(
            painter,
            Pos2::new(plug.center().x, plug.max.y + 12.0),
            "SERIAL",
        );
        label_at(
            painter,
            Pos2::new(sock.center().x, sock.max.y + 12.0),
            "TCP",
        );
    }
}

fn hero_modbus(painter: &Painter, r: Rect) {
    let cols = 8;
    let rows = 3;
    let gap = 4.0;
    let cell = ((r.width() * 0.72 - gap * (cols - 1) as f32) / cols as f32).min(r.height() * 0.22);
    let grid_w = cols as f32 * cell + (cols - 1) as f32 * gap;
    let grid_h = rows as f32 * cell + (rows - 1) as f32 * gap;
    let origin = Pos2::new(r.center().x - grid_w * 0.5, r.center().y - grid_h * 0.5);
    for row in 0..rows {
        for col in 0..cols {
            let cell_r = Rect::from_min_size(
                Pos2::new(
                    origin.x + col as f32 * (cell + gap),
                    origin.y + row as f32 * (cell + gap),
                ),
                Vec2::splat(cell),
            );
            let hot = matches!((col, row), (1, 0) | (4, 1) | (6, 2) | (2, 2));
            if hot {
                painter.rect_filled(cell_r, Rounding::same(2.0), Proto::MODBUS);
            } else {
                painter.rect_stroke(cell_r, Rounding::same(2.0), stroke_of(Proto::MODBUS, 1.2));
            }
            if (col, row) == (4, 1) {
                painter.circle_stroke(cell_r.center(), cell * 0.22, stroke_of(scope::BG, 1.4));
            }
        }
    }
}

fn hero_mqtt(painter: &Painter, r: Rect) {
    let broker = n(r, 0.50, 0.50);
    let rad = r.height() * 0.16;
    let nodes = [
        n(r, 0.16, 0.30),
        n(r, 0.84, 0.28),
        n(r, 0.78, 0.78),
        n(r, 0.18, 0.76),
    ];
    let node_r = rad * 0.38;
    for node in nodes {
        let dir = node - broker;
        let mag = dir.length();
        if mag < 1.0 {
            continue;
        }
        let unit = dir / mag;
        let start = broker + unit * (rad + 2.0);
        let end = node - unit * (node_r + 10.0);
        painter.line_segment([start, end], stroke_of(Proto::MQTT, 1.5));
        arrow_head(painter, end + unit * 8.0, unit, 8.0, Proto::MQTT);
    }
    painter.circle_filled(broker, rad, tint(Proto::MQTT, 50));
    painter.circle_stroke(broker, rad, stroke_of(Proto::MQTT, 1.8));
    painter.circle_filled(broker, rad * 0.28, Proto::MQTT);
    for node in nodes {
        painter.circle_filled(node, node_r, scope::BG);
        painter.circle_stroke(node, node_r, stroke_of(Proto::MQTT, 1.4));
    }
}

fn hero_opc(painter: &Painter, r: Rect) {
    let root = nrect(r, 0.38, 0.08, 0.24, 0.24);
    let tab = Rect::from_min_size(
        Pos2::new(root.min.x, root.min.y - r.height() * 0.06),
        Vec2::new(root.width() * 0.46, r.height() * 0.08),
    );
    painter.rect_filled(tab, Rounding::same(2.0), tint(Proto::OPC, 80));
    painter.rect_stroke(root, Rounding::same(3.0), stroke_of(Proto::OPC, 1.6));
    let join = Pos2::new(root.center().x, root.max.y + r.height() * 0.08);
    painter.line_segment([root.center(), join], stroke_of(Proto::OPC, 1.4));
    let left = n(r, 0.22, 0.62);
    let right = n(r, 0.78, 0.62);
    painter.line_segment(
        [Pos2::new(left.x, join.y), Pos2::new(right.x, join.y)],
        stroke_of(Proto::OPC, 1.4),
    );
    painter.line_segment(
        [Pos2::new(left.x, join.y), left],
        stroke_of(Proto::OPC, 1.4),
    );
    painter.line_segment(
        [Pos2::new(right.x, join.y), right],
        stroke_of(Proto::OPC, 1.4),
    );
    painter.circle_stroke(left, r.height() * 0.09, stroke_of(Proto::OPC, 1.5));
    painter.circle_filled(left, 2.4, Proto::OPC);
    let obj = Rect::from_center_size(right, Vec2::new(r.width() * 0.12, r.height() * 0.16));
    painter.rect_stroke(obj, Rounding::same(3.0), stroke_of(Proto::OPC, 1.5));
    let child = n(r, 0.22, 0.86);
    painter.line_segment([left, child], stroke_of(Proto::OPC, 1.3));
    painter.circle_filled(child, 4.0, Proto::OPC);
}

fn hero_files(painter: &Painter, r: Rect) {
    let page = nrect(r, 0.28, 0.10, 0.36, 0.78);
    painter.rect_filled(page, Rounding::same(3.0), tint(Proto::FILES, 18));
    painter.rect_stroke(page, Rounding::same(3.0), stroke_of(Proto::FILES, 1.6));
    let fold = Rect::from_min_max(
        Pos2::new(page.max.x - 16.0, page.min.y),
        Pos2::new(page.max.x, page.min.y + 16.0),
    );
    painter.line_segment(
        [fold.left_top(), fold.right_bottom()],
        stroke_of(Proto::FILES, 1.3),
    );
    for i in 0..5 {
        let y = page.min.y + 22.0 + i as f32 * 12.0;
        let x1 = if i == 4 {
            page.max.x - 28.0
        } else {
            page.max.x - 12.0
        };
        painter.line_segment(
            [Pos2::new(page.min.x + 12.0, y), Pos2::new(x1, y)],
            stroke_of(scope::TEXT, 1.2),
        );
    }
    // Tail marker on the last line — the offset file-follow will keep.
    let tail = Pos2::new(page.min.x - 10.0, page.min.y + 22.0 + 4.0 * 12.0);
    arrow_head(
        painter,
        Pos2::new(page.min.x + 4.0, tail.y),
        Vec2::new(1.0, 0.0),
        8.0,
        Proto::FILES,
    );
    if r.width() > 260.0 {
        pill(painter, n(r, 0.78, 0.38), "CSV", Proto::FILES);
        pill(painter, n(r, 0.78, 0.58), "JSON", Proto::TEAL);
    }
}

fn hero_points(painter: &Painter, r: Rect) {
    let c = r.center();
    let rad = r.height() * 0.14;
    painter.circle_stroke(c, rad, stroke_of(Proto::COPPER, 1.8));
    painter.circle_filled(c, rad * 0.28, Proto::COPPER);
    let nodes = [
        (n(r, 0.16, 0.28), Proto::STREAMS),
        (n(r, 0.84, 0.26), Proto::MQTT),
        (n(r, 0.82, 0.74), Proto::OPC),
        (n(r, 0.18, 0.76), Proto::MODBUS),
        (n(r, 0.50, 0.10), Proto::FILES),
    ];
    for (node, ink) in nodes {
        painter.line_segment([c, node], stroke_of(ink, 1.3));
        painter.circle_filled(node, 6.0, ink);
    }
}

fn hero_mappings(painter: &Painter, r: Rect) {
    let left = nrect(r, 0.08, 0.18, 0.30, 0.64);
    let right = nrect(r, 0.62, 0.18, 0.30, 0.64);
    card(painter, left, Proto::STREAMS);
    card(painter, right, Proto::MQTT);
    let from = Pos2::new(left.max.x + 6.0, left.center().y);
    let to = Pos2::new(right.min.x - 6.0, right.center().y);
    painter.line_segment([from, to], stroke_of(Proto::COPPER, 1.8));
    arrow_head(painter, to, to - from, 10.0, Proto::COPPER);
    painter.circle_filled(Pos2::new((from.x + to.x) * 0.5, from.y), 5.0, Proto::TEAL);
}

fn hero_traffic(painter: &Painter, r: Rect) {
    let widths = [0.92, 0.62, 0.84, 0.48, 0.74];
    let top = r.min.y + r.height() * 0.08;
    let row_h = r.height() * 0.14;
    let gap = r.height() * 0.04;
    for (i, width) in widths.iter().enumerate() {
        let y = top + i as f32 * (row_h + gap);
        let bar = Rect::from_min_size(
            Pos2::new(r.min.x + r.width() * 0.04, y),
            Vec2::new(r.width() * width, row_h),
        );
        paint_frame(painter, bar, i);
    }
}

fn paint_frame(painter: &Painter, bar: Rect, salt: usize) {
    painter.rect_filled(bar, Rounding::same(3.0), tint(scope::TEXT, 16));
    let pre = bar.width() * 0.10;
    let head = bar.width() * 0.16;
    painter.rect_filled(
        Rect::from_min_size(bar.min, Vec2::new(pre, bar.height())),
        Rounding {
            nw: 3.0,
            ne: 0.0,
            sw: 3.0,
            se: 0.0,
        },
        Proto::COPPER,
    );
    painter.rect_filled(
        Rect::from_min_size(
            Pos2::new(bar.min.x + pre + 2.0, bar.min.y),
            Vec2::new(head, bar.height()),
        ),
        Rounding::ZERO,
        Proto::STREAMS,
    );
    let mut x = bar.min.x + pre + head + 6.0;
    let inks = [Proto::MQTT, Proto::SAGE, Proto::OPC, Proto::MODBUS];
    let mut n = 0;
    while x + 10.0 < bar.max.x - 4.0 {
        let w = 10.0 + ((salt + n) % 3) as f32 * 6.0;
        if x + w > bar.max.x - 4.0 {
            break;
        }
        painter.rect_filled(
            Rect::from_min_size(
                Pos2::new(x, bar.min.y + 3.0),
                Vec2::new(w, bar.height() - 6.0),
            ),
            Rounding::same(2.0),
            inks[n % inks.len()],
        );
        x += w + 3.0;
        n += 1;
    }
}

fn card(painter: &Painter, rect: Rect, ink: Color32) {
    painter.rect_filled(rect, Rounding::same(6.0), tint(ink, 22));
    painter.rect_stroke(rect, Rounding::same(6.0), stroke_of(ink, 1.4));
    let cap = Rect::from_min_size(rect.min, Vec2::new(3.0, rect.height()));
    painter.rect_filled(
        cap,
        Rounding {
            nw: 6.0,
            ne: 0.0,
            sw: 6.0,
            se: 0.0,
        },
        ink,
    );
    for i in 0..3 {
        let y = rect.min.y + 16.0 + i as f32 * 14.0;
        if y > rect.max.y - 10.0 {
            break;
        }
        painter.line_segment(
            [
                Pos2::new(rect.min.x + 14.0, y),
                Pos2::new(rect.max.x - 12.0, y),
            ],
            stroke_of(scope::TEXT, 1.1),
        );
    }
}

fn pill(painter: &Painter, center: Pos2, text: &str, ink: Color32) {
    let w = 14.0 + text.len() as f32 * 7.0;
    let rect = Rect::from_center_size(center, Vec2::new(w, 18.0));
    painter.rect_filled(rect, Rounding::same(4.0), tint(ink, 36));
    painter.rect_stroke(rect, Rounding::same(4.0), Stroke::new(1.0, ink));
    painter.text(
        center,
        egui::Align2::CENTER_CENTER,
        text,
        font_ui(11.0),
        ink,
    );
}

fn label_at(painter: &Painter, pos: Pos2, text: &str) {
    painter.text(
        pos,
        egui::Align2::CENTER_TOP,
        text,
        font_ui(10.0),
        scope::TEXT,
    );
}

fn paint_gear(painter: &Painter, r: Rect, ink: Color32) {
    let c = r.center();
    let rad = r.width().min(r.height()) * 0.32;
    let stroke = Stroke::new((r.width() * 0.09).clamp(1.15, 1.8), ink);
    for i in 0..8 {
        let a = i as f32 / 8.0 * TAU;
        let dir = Vec2::new(a.cos(), a.sin());
        painter.line_segment([c + dir * rad * 0.85, c + dir * rad * 1.28], stroke);
    }
    painter.circle_stroke(c, rad, stroke);
    painter.circle_stroke(c, rad * 0.38, stroke);
}

fn paint_close(painter: &Painter, r: Rect, ink: Color32) {
    let inner = r.shrink(r.width() * 0.22);
    let stroke = Stroke::new((r.width() * 0.10).clamp(1.2, 1.8), ink);
    painter.line_segment([inner.left_top(), inner.right_bottom()], stroke);
    painter.line_segment([inner.right_top(), inner.left_bottom()], stroke);
}

fn paint_max(painter: &Painter, r: Rect, ink: Color32, restore: bool) {
    let stroke = Stroke::new((r.width() * 0.09).clamp(1.15, 1.7), ink);
    if !restore {
        painter.rect_stroke(r.shrink(r.width() * 0.16), Rounding::same(1.5), stroke);
        return;
    }
    let back = Rect::from_min_max(n(r, 0.30, 0.16), n(r, 0.88, 0.70));
    let front = Rect::from_min_max(n(r, 0.12, 0.34), n(r, 0.70, 0.88));
    painter.rect_stroke(back, Rounding::same(1.0), stroke);
    painter.rect_stroke(front, Rounding::same(1.0), stroke);
}

fn wave(painter: &Painter, x0: f32, x1: f32, y: f32, amp: f32, cycles: f32, stroke: Stroke) {
    let steps = 28;
    let mut prev = None;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let p = Pos2::new(egui::lerp(x0..=x1, t), y + (t * cycles * TAU).sin() * amp);
        if let Some(a) = prev {
            painter.line_segment([a, p], stroke);
        }
        prev = Some(p);
    }
}

fn arrow_head(painter: &Painter, tip: Pos2, dir: Vec2, len: f32, color: Color32) {
    let mag = dir.length();
    if mag < 0.001 || len < 0.5 {
        return;
    }
    let dir = dir / mag;
    let side = Vec2::new(-dir.y, dir.x);
    let base = tip - dir * len;
    painter.add(Shape::convex_polygon(
        vec![tip, base + side * len * 0.55, base - side * len * 0.55],
        color,
        Stroke::NONE,
    ));
}

fn stroke_of(color: Color32, width: f32) -> Stroke {
    Stroke::new(width, color)
}

fn n(r: Rect, x: f32, y: f32) -> Pos2 {
    Pos2::new(
        egui::lerp(r.min.x..=r.max.x, x),
        egui::lerp(r.min.y..=r.max.y, y),
    )
}

fn nrect(r: Rect, x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::from_min_size(n(r, x, y), Vec2::new(r.width() * w, r.height() * h))
}
