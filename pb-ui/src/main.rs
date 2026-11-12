#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// Stroke widths are f32. Rust 1.99 warns when an untyped float hits that generic slot.
#![allow(float_literal_f32_fallback)]

mod app;
mod art;
mod dnd;
mod dock;
mod hubs;
mod layout;
mod panels;
mod persist;
mod theme;

use eframe::egui;

fn main() -> eframe::Result<()> {
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size([1600.0, 980.0])
        .with_min_inner_size([1200.0, 760.0])
        .with_title(format!("Protocol Bench  {}", env!("CARGO_PKG_VERSION")));

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "protocol-bench",
        options,
        Box::new(|cc| Ok(Box::new(app::ProtocolBenchApp::new(cc)))),
    )
}
