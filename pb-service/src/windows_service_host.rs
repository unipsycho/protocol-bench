//! Windows Service scaffold (ship path). Linux IPC is tested first.
//!
//! Register later via MSI / sc.exe. For now this module compiles on Windows
//! and runs the same foreground host when started with `--windows-service`
//! until a full `windows-service` crate dispatcher is wired.

#![cfg(windows)]

pub fn run() -> Result<(), String> {
    eprintln!(
        "pb-service: Windows Service entry — running foreground host (full SCM dispatcher TBD)"
    );
    crate::run_foreground()
}
