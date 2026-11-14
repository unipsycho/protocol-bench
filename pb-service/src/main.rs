//! Minimal headless host: load packs from `modules/`, run the engine thread.
//!
//! Usage:
//!   PB_MODULES=./modules cargo run -p pb-service
//!
//! Stop with Ctrl-C. Local IPC to the UI is follow-on work.

use pb_engine::{default_modules_dir, EngineHandle, EngineId, EngineOptions};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

fn main() {
    let modules = std::env::var_os("PB_MODULES")
        .map(PathBuf::from)
        .unwrap_or_else(default_modules_dir);

    eprintln!("pb-service: modules dir {}", modules.display());
    let mut host = EngineHandle::spawn_with(EngineOptions {
        modules_dir: Some(modules),
        points: None,
    });

    let running = Arc::new(AtomicBool::new(true));
    {
        let flag = Arc::clone(&running);
        ctrlc::set_handler(move || {
            flag.store(false, Ordering::SeqCst);
        })
        .expect("install Ctrl-C handler");
    }

    while running.load(Ordering::SeqCst) {
        let snap = host.snapshot();
        if !snap.alive {
            break;
        }
        eprintln!(
            "pb-service: uptime={}ms packs={:?} points={} mqtt={}",
            snap.uptime_ms,
            snap.packs_loaded,
            snap.point_count,
            snap.slot(EngineId::Mqtt)
                .map(|s| s.state.as_str())
                .unwrap_or("?")
        );
        thread::sleep(Duration::from_secs(2));
    }

    host.shutdown();
    eprintln!("pb-service: stopped");
}
