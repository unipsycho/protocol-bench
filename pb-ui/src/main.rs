#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// Stroke widths are f32. Rust 1.99 warns when an untyped float hits that generic slot.
#![allow(float_literal_f32_fallback)]

mod agg_panel;
mod app;
mod art;
mod brand_logo;
mod calc_panel;
mod dnd;
mod dock;
mod hubs;
mod layout;
mod mqtt_panel;
mod panels;
mod persist;
mod point_rows;
mod project;
mod theme;
mod topic_tree;

use eframe::egui;

fn main() -> eframe::Result<()> {
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size([1600.0, 980.0])
        .with_min_inner_size([1200.0, 760.0])
        .with_app_id("protocol-bench")
        // Product name only — Omarchy/Hyprland groups by title; version lives in About.
        .with_title("Protocol Bench");

    // Vsync swap waits on a Wayland frame callback. Hyprland does not send one
    // for a window on another workspace, so the wait blocks the event loop and
    // the compositor reports "not responding". Presentation is still paced by
    // the compositor; this only stops the GL swap from stalling the thread.
    let options = eframe::NativeOptions {
        viewport,
        vsync: false,
        ..Default::default()
    };

    eframe::run_native(
        "protocol-bench",
        options,
        Box::new(|cc| Ok(Box::new(app::ProtocolBenchApp::new(cc)))),
    )
}
