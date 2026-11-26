//! Live hub status: per-hub run timers, shared summary text, and the top-banner strip.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, Pos2, Rect, Rounding, Sense, Stroke, Vec2};

use crate::hubs::{HubModel, HubProto};
use crate::mqtt_panel::MqttPanelState;
use crate::opcua_panel::OpcUaPanelState;
use crate::theme::{font_mono, font_semi, font_ui, tint, ThemePalette};

/// Tracks when one hub last started / stopped. Observes the running flag every frame, so every
/// start path (pill, Project panel, auto-start, service attach) is timed the same way.
#[derive(Debug, Clone, Copy, Default)]
pub struct HubClock {
    seen: Option<bool>,
    since: Option<Instant>,
    /// True once this session saw the hub go from running to stopped.
    stopped_after_run: bool,
}

impl HubClock {
    pub fn observe(&mut self, running: bool) {
        if self.seen != Some(running) {
            if self.seen == Some(true) && !running {
                self.stopped_after_run = true;
            }
            self.since = Some(Instant::now());
            self.seen = Some(running);
        }
    }

    fn elapsed(&self) -> Duration {
        self.since.map(|t| t.elapsed()).unwrap_or_default()
    }

    /// "up 12:41" while running; "stopped 3m ago" after a run this session; else "stopped".
    pub fn detail(&self, running: bool) -> String {
        if running {
            format!("up {}", format_clock(self.elapsed()))
        } else if self.stopped_after_run {
            format!("stopped {}", format_ago(self.elapsed()))
        } else {
            "stopped".into()
        }
    }
}

pub fn format_clock(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

pub fn format_ago(d: Duration) -> String {
    let s = d.as_secs();
    if s < 5 {
        "just now".into()
    } else if s < 60 {
        format!("{s}s ago")
    } else if s < 3600 {
        format!("{}m ago", s / 60)
    } else {
        format!("{}h ago", s / 3600)
    }
}

/// One startable hub kind (MQTT broker / OPC UA server) as the user sees it.
#[derive(Debug, Clone)]
pub struct HubKind {
    pub label: &'static str,
    pub port: u16,
    pub running: bool,
    /// Start failure, when the last attempt left an error behind.
    pub error: Option<String>,
}

/// Hub kinds this project has (configured embedded hub or a hub of that protocol on the board).
pub fn kinds(mqtt: &MqttPanelState, opcua: &OpcUaPanelState, hubs: &HubModel) -> Vec<HubKind> {
    let mut out = Vec::new();
    if hubs.hubs.iter().any(|h| h.protocol == HubProto::Mqtt) || mqtt.embedded.enabled {
        let s = mqtt.broker_status.trim();
        let failed = !mqtt.broker_running && !s.is_empty() && s != "Stopped" && s != "Starting…";
        out.push(HubKind {
            label: "MQTT",
            port: mqtt.embedded.port,
            running: mqtt.broker_running,
            error: failed.then(|| s.to_owned()),
        });
    }
    if hubs.hubs.iter().any(|h| h.protocol == HubProto::OpcUa) || opcua.embedded.enabled {
        let s = opcua.server_status.trim();
        let failed = !opcua.server_running && !s.is_empty() && s != "Stopped" && s != "Starting…";
        out.push(HubKind {
            label: "OPC UA",
            port: opcua.embedded.port,
            running: opcua.server_running,
            error: failed.then(|| s.to_owned()),
        });
    }
    out
}

/// "2 hubs running" / "1 of 2 running" / "All hubs stopped" / "No hubs".
pub fn headline(kinds: &[HubKind]) -> String {
    let up = kinds.iter().filter(|k| k.running).count();
    match (kinds.len(), up) {
        (0, _) => "No hubs".into(),
        (_, 0) => "All hubs stopped".into(),
        (n, u) if u == n => format!("{u} hub{} running", if u == 1 { "" } else { "s" }),
        (n, u) => format!("{u} of {n} running"),
    }
}

/// "MQTT :1883 · OPC UA :4840" — only what is actually running.
pub fn subheading(kinds: &[HubKind]) -> String {
    let live: Vec<String> = kinds
        .iter()
        .filter(|k| k.running)
        .map(|k| format!("{} :{}", k.label, k.port))
        .collect();
    if live.is_empty() {
        "Nothing running".into()
    } else {
        live.join("  ·  ")
    }
}

/// What the banner strip asked for this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StripAction {
    /// Start everything (stopped) or stop everything (running).
    ToggleAll,
}

pub struct StripInput<'a> {
    pub kinds: &'a [HubKind],
    pub clocks: [&'a HubClock; 2],
    pub points: usize,
    pub project: &'a str,
}

fn text_w(ui: &egui::Ui, text: &str, font: egui::FontId) -> f32 {
    ui.painter()
        .layout_no_wrap(text.to_owned(), font, Color32::WHITE)
        .size()
        .x
}

/// Status strip drawn right-aligned in the free space of the banner. It never changes the
/// banner height: everything fits in `STRIP_H`. Parts drop out (headline, then points, then
/// chip detail) as the window narrows.
pub fn banner_strip(
    ui: &mut egui::Ui,
    input: &StripInput<'_>,
    p: &ThemePalette,
) -> Option<StripAction> {
    const STRIP_H: f32 = 38.0;
    const GAP: f32 = 10.0;
    let avail = ui.available_rect_before_wrap();
    let kinds = input.kinds;
    if kinds.is_empty() || avail.width() < 120.0 {
        return None;
    }
    let any_up = kinds.iter().any(|k| k.running);
    let t = ui.input(|i| i.time) as f32;
    if any_up {
        // Drives the live timers and the status-dot pulse.
        // 4 fps is plenty for a seconds timer and a slow pulse (100 ms cost ~10 redraws/s).
        ui.ctx().request_repaint_after(Duration::from_millis(250));
    }

    // Per-hub chip text.
    struct Chip {
        name: &'static str,
        detail: String,
        color: Color32,
        running: bool,
        w: f32,
        tip: String,
    }
    let chips: Vec<Chip> = kinds
        .iter()
        .enumerate()
        .map(|(i, k)| {
            let clock = input.clocks[i.min(1)];
            let (detail, color) = match (&k.error, k.running) {
                (_, true) => (clock.detail(true), p.live),
                (Some(e), false) => {
                    let mut e = if e.contains("already in use") {
                        "port in use".to_owned()
                    } else if e.starts_with("Could not start") {
                        "failed to start".to_owned()
                    } else {
                        e.clone()
                    };
                    if e.chars().count() > 26 {
                        e = e.chars().take(25).collect::<String>() + "…";
                    }
                    (e, p.err)
                }
                (None, false) => (clock.detail(false), p.muted),
            };
            let name_w = text_w(ui, k.label, font_semi(12.5));
            let det_w = text_w(ui, &detail, font_mono(10.0));
            let tip = match (&k.error, k.running) {
                (_, true) => format!("{} hub listening on :{} — {}", k.label, k.port, clock.detail(true)),
                (Some(e), false) => format!("{} hub failed: {e}", k.label),
                (None, false) => format!("{} hub — {}", k.label, clock.detail(false)),
            };
            Chip {
                name: k.label,
                w: 12.0 + 14.0 + name_w.max(det_w) + 12.0,
                detail,
                color,
                running: k.running,
                tip,
            }
        })
        .collect();

    let head = headline(kinds);
    let sub = subheading(kinds);
    let head_w = text_w(ui, &head, font_semi(12.5)).max(text_w(ui, &sub, font_ui(10.0)));
    let pts = input.points.to_string();
    let pts_w = text_w(ui, &pts, font_mono(13.0)).max(text_w(ui, "POINTS", font_ui(9.0)));
    let pill_h = 24.0;
    let pill_w = 74.0;

    // Project name block (left of the statuses), capped so long names truncate.
    let proj = input.project.trim();
    let proj_w = text_w(ui, proj, font_semi(13.0)).min(170.0).max(text_w(ui, "PROJECT", font_ui(9.0)));

    // Parts left to right: project | headline | chips | points | pill. Drop the least
    // important first (headline, points, project, then the pill) until it fits.
    let chips_w: f32 = chips.iter().map(|c| c.w).sum::<f32>() + GAP * (chips.len() as f32 - 1.0);
    let sep = GAP * 2.0 + 1.0;
    let width_of = |proj_on: bool, head_on: bool, pts_on: bool, pill_on: bool| -> f32 {
        let mut w = chips_w;
        if proj_on { w += proj_w + sep; }
        if head_on { w += head_w + sep; }
        if pts_on { w += sep + pts_w; }
        if pill_on { w += GAP + pill_w; }
        w
    };
    let room = avail.width() - 16.0;
    let (show_proj, show_head, show_pts, show_pill) = [
        (!proj.is_empty(), true, true, true),
        (!proj.is_empty(), false, true, true),
        (!proj.is_empty(), false, false, true),
        (false, false, false, true),
        (false, false, false, false),
    ]
    .into_iter()
    .find(|&(a, b, c, d)| width_of(a, b, c, d) <= room)?;

    let total = width_of(show_proj, show_head, show_pts, show_pill);
    let cy = avail.center().y;
    // Centered in the free banner space.
    let mut x = avail.center().x - total * 0.5;
    let mut action = None;
    let painter = ui.painter().clone();
    let divider = |painter: &egui::Painter, x: f32| {
        painter.line_segment(
            [Pos2::new(x, cy - 13.0), Pos2::new(x, cy + 13.0)],
            Stroke::new(1.0, tint(p.muted, 70)),
        );
    };

    if show_proj {
        let clip = Rect::from_min_max(Pos2::new(x, cy - 18.0), Pos2::new(x + proj_w, cy + 18.0));
        painter.with_clip_rect(clip).text(
            Pos2::new(x, cy - 6.0),
            Align2::LEFT_CENTER,
            proj,
            font_semi(13.0),
            p.text,
        );
        painter.text(
            Pos2::new(x, cy + 9.0),
            Align2::LEFT_CENTER,
            "PROJECT",
            font_ui(9.0),
            p.muted,
        );
        x += proj_w + GAP;
        divider(&painter, x);
        x += GAP + 1.0;
    }
    if show_head {
        painter.text(
            Pos2::new(x, cy - 7.0),
            Align2::LEFT_CENTER,
            &head,
            font_semi(12.5),
            if any_up { p.live } else { p.muted },
        );
        painter.text(
            Pos2::new(x, cy + 8.0),
            Align2::LEFT_CENTER,
            &sub,
            font_ui(10.0),
            p.muted,
        );
        x += head_w + GAP;
        divider(&painter, x);
        x += GAP + 1.0;
    }
    for chip in &chips {
        let r = Rect::from_min_size(Pos2::new(x, cy - STRIP_H * 0.5), Vec2::new(chip.w, STRIP_H));
        let resp = ui.interact(r, ui.id().with(("hub_chip", chip.name)), Sense::hover());
        let round = Rounding::same(8.0);
        let hot = resp.hovered();
        painter.rect_filled(
            r,
            round,
            tint(chip.color, if chip.running { if hot { 46 } else { 30 } } else if hot { 30 } else { 16 }),
        );
        painter.rect_stroke(
            r.shrink(0.5),
            round,
            Stroke::new(1.0, tint(chip.color, if chip.running { 150 } else { 80 })),
        );
        // Status dot: pulsing halo while live.
        let dot = Pos2::new(r.min.x + 12.0, r.center().y);
        if chip.running {
            let pulse = (t * 2.4).sin() * 0.5 + 0.5;
            painter.circle_filled(dot, 3.5 + 3.5 * pulse, tint(chip.color, (70.0 * (1.0 - pulse)) as u8));
        }
        painter.circle_filled(dot, 3.5, chip.color);
        let tx = r.min.x + 22.0;
        painter.text(
            Pos2::new(tx, r.center().y - 7.0),
            Align2::LEFT_CENTER,
            chip.name,
            font_semi(12.5),
            if chip.running { p.text } else { tint(p.text, 190) },
        );
        painter.text(
            Pos2::new(tx, r.center().y + 8.0),
            Align2::LEFT_CENTER,
            &chip.detail,
            font_mono(10.0),
            if chip.running || chip.color == p.err { chip.color } else { p.muted },
        );
        resp.on_hover_text(&chip.tip);
        x += chip.w + GAP;
    }
    x -= GAP;
    if show_pts {
        x += GAP;
        divider(&painter, x);
        x += GAP + 1.0;
        painter.text(
            Pos2::new(x + pts_w, cy - 6.0),
            Align2::RIGHT_CENTER,
            &pts,
            font_mono(13.0),
            p.text,
        );
        painter.text(
            Pos2::new(x + pts_w, cy + 9.0),
            Align2::RIGHT_CENTER,
            "POINTS",
            font_ui(9.0),
            p.muted,
        );
        x += pts_w;
    }
    if show_pill {
        x += GAP;
        let r = Rect::from_min_size(Pos2::new(x, cy - pill_h * 0.5), Vec2::new(pill_w, pill_h));
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(r)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        let tip = if any_up { "Stop all running hubs" } else { "Start all hubs" };
        if crate::art::run_pill(&mut child, any_up, true, tip, p, pill_h).clicked() {
            action = Some(StripAction::ToggleAll);
        }
    }
    action
}
