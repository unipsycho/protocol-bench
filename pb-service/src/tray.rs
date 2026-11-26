//! Status-area tray icon: shows the active project and which hubs are live.
//!
//! Lives in the service (not the UI) so it stays up when the window is closed.
//! Linux uses a StatusNotifierItem over D-Bus (waybar / KDE / GNOME-with-extension).
//! Other platforms are a no-op for now.

use std::sync::{Arc, Mutex};

use crate::runtime::Runtime;

#[cfg(target_os = "linux")]
pub fn spawn(runtime: Arc<Mutex<Runtime>>) {
    use ksni::blocking::TrayMethods;
    use std::{thread, time::Duration};

    // Headless runs (tests, CI, perf harness) skip the status-area icon.
    if std::env::var_os("PB_NO_TRAY").is_some() {
        return;
    }
    let tray = HubTray {
        runtime: Arc::clone(&runtime),
        project: String::new(),
        running: Vec::new(),
    };
    let handle = match tray.spawn() {
        Ok(h) => h,
        Err(err) => {
            eprintln!("pb-service: tray unavailable: {err}");
            return;
        }
    };
    thread::spawn(move || loop {
        thread::sleep(Duration::from_secs(2));
        let (project, running) = match runtime.lock() {
            Ok(rt) => {
                let st = rt.status();
                (st.project_path.clone(), st.running_hubs())
            }
            Err(_) => continue,
        };
        if handle
            .update(|t| {
                if t.project != project || t.running != running {
                    t.project = project;
                    t.running = running;
                }
            })
            .is_none()
        {
            break; // tray shut down
        }
    });
}

#[cfg(not(target_os = "linux"))]
pub fn spawn(_runtime: Arc<Mutex<Runtime>>) {}

#[cfg(target_os = "linux")]
struct HubTray {
    runtime: Arc<Mutex<Runtime>>,
    project: String,
    running: Vec<String>,
}

#[cfg(target_os = "linux")]
impl HubTray {
    fn project_name(&self) -> String {
        std::path::Path::new(&self.project)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("no project")
            .to_string()
    }
}

#[cfg(target_os = "linux")]
impl ksni::Tray for HubTray {
    fn id(&self) -> String {
        "protocol-bench-service".into()
    }

    fn title(&self) -> String {
        "Protocol Bench".into()
    }

    fn status(&self) -> ksni::Status {
        // Passive hides the item on some trays when nothing is live.
        if self.running.is_empty() {
            ksni::Status::Passive
        } else {
            ksni::Status::Active
        }
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        let live = !self.running.is_empty();
        vec![mark_icon(64, live)]
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        let body = if self.running.is_empty() {
            "No hubs running".to_string()
        } else {
            self.running.join("\n")
        };
        ksni::ToolTip {
            title: format!("Protocol Bench — {}", self.project_name()),
            description: body,
            icon_name: String::new(),
            icon_pixmap: vec![mark_icon(64, !self.running.is_empty())],
        }
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;
        let mut items: Vec<MenuItem<Self>> = vec![StandardItem {
            label: format!("Project: {}", self.project_name()),
            enabled: false,
            ..Default::default()
        }
        .into()];
        if self.running.is_empty() {
            items.push(
                StandardItem {
                    label: "No hubs running".into(),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            );
        } else {
            for line in &self.running {
                items.push(
                    StandardItem {
                        label: line.clone(),
                        enabled: false,
                        ..Default::default()
                    }
                    .into(),
                );
            }
        }
        items.push(MenuItem::Separator);
        items.push(
            StandardItem {
                label: "Stop all hubs".into(),
                activate: Box::new(|t: &mut Self| {
                    if let Ok(mut rt) = t.runtime.lock() {
                        rt.stop_all_hubs();
                    }
                }),
                ..Default::default()
            }
            .into(),
        );
        items.push(
            StandardItem {
                label: "Quit service".into(),
                activate: Box::new(|t: &mut Self| {
                    if let Ok(mut rt) = t.runtime.lock() {
                        rt.stop_all_hubs();
                    }
                    std::process::exit(0);
                }),
                ..Default::default()
            }
            .into(),
        );
        items
    }
}

/// The nav-bar mark (pb-ui `art::paint_mark`, Copper accent) rasterised for the tray, plus a
/// small status dot bottom-right: green when hubs are live, grey when idle.
/// Geometry mirrors the egui painter; keep in step if the mark changes.
#[cfg(target_os = "linux")]
fn mark_icon(size: i32, live: bool) -> ksni::Icon {
    type Rgb = (u8, u8, u8);
    const BG: Rgb = (0x0c, 0x12, 0x18);
    const ACCENT: Rgb = (0xe0, 0x7a, 0x3d);
    const STREAMS: Rgb = (0x79, 0xb8, 0xff);
    const MODBUS: Rgb = (0xf0, 0xb4, 0x29);
    const MQTT: Rgb = (0x2e, 0xc9, 0xb8);

    enum Sh {
        RectFill { c: (f32, f32), h: f32, r: f32, col: Rgb },
        RectStroke { c: (f32, f32), h: f32, r: f32, w: f32, col: Rgb },
        Seg { a: (f32, f32), b: (f32, f32), w: f32, col: Rgb },
        Circle { c: (f32, f32), r: f32, col: Rgb },
    }

    fn rrect_sdf(p: (f32, f32), c: (f32, f32), h: f32, r: f32) -> f32 {
        let q = ((p.0 - c.0).abs() - (h - r), (p.1 - c.1).abs() - (h - r));
        let outside = (q.0.max(0.0).powi(2) + q.1.max(0.0).powi(2)).sqrt();
        outside + q.0.max(q.1).min(0.0) - r
    }
    fn seg_dist(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
        let (abx, aby) = (b.0 - a.0, b.1 - a.1);
        let len2 = (abx * abx + aby * aby).max(1e-6);
        let t = (((p.0 - a.0) * abx + (p.1 - a.1) * aby) / len2).clamp(0.0, 1.0);
        ((p.0 - (a.0 + abx * t)).powi(2) + (p.1 - (a.1 + aby * t)).powi(2)).sqrt()
    }
    fn hit(sh: &Sh, p: (f32, f32)) -> Option<Rgb> {
        match *sh {
            Sh::RectFill { c, h, r, col } => (rrect_sdf(p, c, h, r) <= 0.0).then_some(col),
            Sh::RectStroke { c, h, r, w, col } => {
                (rrect_sdf(p, c, h, r).abs() <= w * 0.5).then_some(col)
            }
            Sh::Seg { a, b, w, col } => (seg_dist(p, a, b) <= w * 0.5).then_some(col),
            Sh::Circle { c, r, col } => {
                (((p.0 - c.0).powi(2) + (p.1 - c.1).powi(2)).sqrt() <= r).then_some(col)
            }
        }
    }

    let side = size as f32;
    // The egui mark clamps strokes/radii in points for ~28px slots; scale them to this size.
    let k = side / 32.0;
    let c = (side * 0.5, side * 0.5);
    let sw = (side * 0.038).clamp(1.05 * k, 2.3 * k);
    let mut shapes = vec![
        Sh::RectFill { c, h: side * 0.5, r: side * 0.17, col: BG },
        Sh::RectStroke {
            c,
            h: side * 0.5,
            r: side * 0.22,
            w: (side * 0.045).clamp(1.2 * k, 2.8 * k),
            col: ACCENT,
        },
    ];
    let inner_min = side * 0.20;
    let inner_w = side * 0.60;
    let inner_h = inner_w;
    let gap = inner_h * 0.10;
    let lane_h = (inner_h - gap * 2.0) / 3.0;
    let (x0, x1) = (inner_min, inner_min + inner_w);

    // Lane 0: serial wave.
    let (cy0, amp, cycles, steps) = (inner_min + lane_h * 0.5, lane_h * 0.28, 1.6f32, 28);
    let pt = |i: i32| {
        let t = i as f32 / steps as f32;
        (x0 + (x1 - x0) * t, cy0 + (t * cycles * std::f32::consts::TAU).sin() * amp)
    };
    for i in 0..steps {
        shapes.push(Sh::Seg { a: pt(i), b: pt(i + 1), w: sw, col: STREAMS });
    }

    // Lane 1: five Modbus cells, odd ones filled.
    let lane1_y = inner_min + lane_h + gap;
    let cell = (lane_h * 0.62).min((x1 - x0) / (5.0 * 1.35));
    let cgap = cell * 0.32;
    let total = 5.0 * cell + 4.0 * cgap;
    let mut x = (x0 + x1) * 0.5 - total * 0.5;
    let top = lane1_y + (lane_h - cell) * 0.5;
    for i in 0..5 {
        let cc = (x + cell * 0.5, top + cell * 0.5);
        if i % 2 == 1 {
            shapes.push(Sh::RectFill { c: cc, h: cell * 0.5, r: cell * 0.22, col: MODBUS });
        } else {
            shapes.push(Sh::RectStroke {
                c: cc,
                h: cell * 0.5,
                r: cell * 0.22,
                w: sw * 0.85,
                col: MODBUS,
            });
        }
        x += cell + cgap;
    }

    // Lane 2: three linked MQTT nodes.
    let cy = lane1_y + lane_h + gap + lane_h * 0.5;
    let radius = (lane_h * 0.20).clamp(1.4 * k, 5.5 * k);
    let left = (x0 + inner_w * 0.18, cy);
    let mid = (side * 0.5, cy);
    let right = (x1 - inner_w * 0.18, cy);
    shapes.push(Sh::Seg { a: left, b: mid, w: sw, col: MQTT });
    shapes.push(Sh::Seg { a: mid, b: right, w: sw, col: MQTT });
    for n in [left, mid, right] {
        shapes.push(Sh::Circle { c: n, r: radius, col: MQTT });
    }

    // Status dot with a dark ring so it reads on any panel colour.
    let status: Rgb = if live { (0x2e, 0xcc, 0x71) } else { (0x80, 0x80, 0x80) };
    let dc = (side * 0.84, side * 0.84);
    shapes.push(Sh::Circle { c: dc, r: side * 0.17, col: BG });
    shapes.push(Sh::Circle { c: dc, r: side * 0.125, col: status });

    // 4x4 supersampling; premultiplied-free ARGB32 (network byte order).
    const SS: i32 = 4;
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            let (mut r, mut g, mut b, mut covered) = (0u32, 0u32, 0u32, 0u32);
            for sy in 0..SS {
                for sx in 0..SS {
                    let p = (
                        px as f32 + (sx as f32 + 0.5) / SS as f32,
                        py as f32 + (sy as f32 + 0.5) / SS as f32,
                    );
                    if let Some(col) = shapes.iter().rev().find_map(|s| hit(s, p)) {
                        r += col.0 as u32;
                        g += col.1 as u32;
                        b += col.2 as u32;
                        covered += 1;
                    }
                }
            }
            let n = (SS * SS) as u32;
            if covered == 0 {
                data.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                data.extend_from_slice(&[
                    (covered * 255 / n) as u8,
                    (r / covered) as u8,
                    (g / covered) as u8,
                    (b / covered) as u8,
                ]);
            }
        }
    }
    ksni::Icon { width: size, height: size, data }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::mark_icon;

    #[test]
    fn icon_has_expected_size_and_status_colour() {
        let live = mark_icon(64, true);
        let idle = mark_icon(64, false);
        assert_eq!(live.data.len(), 64 * 64 * 4);
        assert_ne!(live.data, idle.data);
        // Corner is outside the rounded badge → transparent.
        assert_eq!(&live.data[0..4], &[0, 0, 0, 0]);
    }
}
