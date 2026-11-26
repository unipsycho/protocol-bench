//! Protocol Bench always-on service (PB-96).
//!
//! Owns `project.json`, loads packs via `pb-engine`, listens on 127.0.0.1 JSON IPC.
//!
//! Usage:
//!   PB_MODULES=./modules cargo run -p pb-service
//!   PB_IPC_PORT=7500 PB_MODULES=./modules cargo run -p pb-service
//!
//! A status-area tray icon (Linux) shows the active project and which hubs are live; see `tray`.
//!
//! Endpoint/token: `…/ProtocolBench/ipc.json` + `ipc.token`
//! Windows Service entry is scaffolded under `windows_service` (follows Linux IPC).

mod mcp;
mod runtime;
mod server;
mod tray;

#[cfg(windows)]
mod windows_service_host;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use pb_ipc::default_project_path;
use runtime::Runtime;

fn main() {
    #[cfg(windows)]
    {
        if std::env::args().any(|a| a == "--windows-service") {
            if let Err(err) = windows_service_host::run() {
                eprintln!("pb-service: windows service error: {err}");
                std::process::exit(1);
            }
            return;
        }
    }

    if let Err(err) = run_foreground() {
        eprintln!("pb-service: {err}");
        std::process::exit(1);
    }
}

pub(crate) fn run_foreground() -> Result<(), String> {
    let modules = std::env::var_os("PB_MODULES").map(PathBuf::from);
    // PB_PROJECT, else the project open when the service last ran, else the default.
    let marker = pb_ipc::last_project_marker();
    let project = pb_ipc::startup_project_path(
        std::env::var_os("PB_PROJECT").map(PathBuf::from),
        marker.as_deref(),
        default_project_path(),
    )
    .ok_or_else(|| "no project path".to_string())?;
    let port: u16 = std::env::var("PB_IPC_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0); // 0 → ephemeral

    eprintln!("pb-service: project {}", project.display());
    if let Some(m) = &modules {
        eprintln!("pb-service: modules {}", m.display());
    }

    let runtime = Arc::new(Mutex::new(Runtime::start(modules, project)));
    let rt_status = Arc::clone(&runtime);
    thread::spawn(move || loop {
        thread::sleep(Duration::from_secs(5));
        if let Ok(rt) = rt_status.lock() {
            let st = rt.status();
            eprintln!(
                "pb-service: uptime={}ms packs={:?} points={} mqtt_hub={} opcua_hub={}",
                st.uptime_ms,
                st.packs_loaded,
                st.point_count,
                st.mqtt_hub_running,
                st.opcua_hub_running
            );
        }
    });

    tray::spawn(Arc::clone(&runtime));
    mcp::spawn(Arc::clone(&runtime));
    server::serve(runtime, port)
}
