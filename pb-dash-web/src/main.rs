//! Dashboard browser app (egui compiled to wasm), served by the `pb-dashboard` pack.
//! The same code runs natively as a demo window with synthetic data (see `demo.rs`).

mod app;
mod chart;
mod demo;
mod editor;
mod grid_view;
mod net;
mod state;
mod theme;
mod widgets;

#[cfg(target_arch = "wasm32")]
fn main() {
    use std::cell::RefCell;
    use std::rc::Rc;
    use wasm_bindgen::JsCast;
    wasm_bindgen_futures::spawn_local(async {
        let doc = web_sys::window().and_then(|w| w.document()).expect("no document");
        let canvas = doc
            .get_element_by_id("pb_dash_canvas")
            .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
            .expect("canvas #pb_dash_canvas missing");
        let started = eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|cc| {
                    let shared = Rc::new(RefCell::new(state::Shared::new()));
                    net::start(&shared, &cc.egui_ctx);
                    let (s2, ctx) = (shared.clone(), cc.egui_ctx.clone());
                    let backend: Box<dyn FnMut(&mut state::Shared, f64)> = Box::new(move |sh, _| net::pump(&s2, &ctx, sh));
                    Ok(Box::new(app::DashApp::new(cc, shared, backend, net::is_kiosk(), net::initial_name())))
                }),
            )
            .await;
        if let Some(l) = doc.get_element_by_id("loading") {
            match started {
                Ok(()) => l.remove(),
                Err(e) => l.set_text_content(Some(&format!("Could not start the dashboard: {e:?}"))),
            }
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result<()> {
    use std::cell::RefCell;
    use std::rc::Rc;
    let args: Vec<String> = std::env::args().collect();
    let (empty, read_only) = (args.iter().any(|a| a == "--empty"), args.iter().any(|a| a == "--read-only"));
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]).with_title("Dashboard (demo)"),
        ..Default::default()
    };
    eframe::run_native(
        "pb-dash-web demo",
        options,
        Box::new(move |cc| {
            let shared = Rc::new(RefCell::new(state::Shared::new()));
            let theme = args.iter().find_map(|a| a.strip_prefix("--theme=")).unwrap_or("").to_string();
            let mut d = demo::Demo::new(empty, read_only);
            d.theme = theme;
            let ctx = cc.egui_ctx.clone();
            let backend: Box<dyn FnMut(&mut state::Shared, f64)> = Box::new(move |sh, now| {
                d.pump(sh, now);
                ctx.request_repaint_after(std::time::Duration::from_millis(250));
            });
            Ok(Box::new(app::DashApp::new(cc, shared, backend, false, None)))
        }),
    )
}
