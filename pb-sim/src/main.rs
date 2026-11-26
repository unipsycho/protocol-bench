//! `pb-sim` — field devices for the Protocol Bench demo projects (all Rust, no hardware).
//!
//! ```text
//! pb-sim frac       # frac spread: 4 frac pumps + blender (Modbus TCP), wellhead RTU (DNP3),
//!                   #   pressure-pumping WITS feed, pump 1 engine on CAN (vcan0, if present)
//! pb-sim bottling   # bottling line: line PLC, two VFDs, energy meter (Modbus TCP),
//!                   #   robot cell as a Sparkplug B edge node (needs the project's hub broker)
//! pb-sim water      # water lift station: RTU (DNP3) + chlorine analyzer (Modbus TCP)
//! pb-sim all        # everything at once
//! ```
//!
//! Ports are listed by `pb-sim --help` and in `protocol-bench-projects/README.md`. Values move
//! like the real process (stage cycles, pump curves, wet-well fill / draw), and commands the
//! hub writes back (pump enable, rate setpoint, ESD, line run, VFD speed, pump starts) change
//! what the devices report.

mod devices;
mod frac;
mod perf;
mod bottling;
mod watch;
mod water;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const HELP: &str = "pb-sim — Protocol Bench demo field devices

USAGE: pb-sim <frac | bottling | water | all>
       pb-sim watch [point …]           live point table from the running service
       pb-sim check <point …> [--timeout S]  exit 0 once every point is good (smokes / CI)
       pb-sim perf <pid …> [--secs N] [--label T]  CPU % / RSS / threads / fds from /proc (Linux)

frac      Frac pumps 1-4   Modbus TCP 127.0.0.1:5021-5024 (unit 1)
          Blender          Modbus TCP 127.0.0.1:5025
          Wellhead RTU     DNP3 TCP 127.0.0.1:20000 (outstation 10, master 1)
          WITS feed        TCP server 127.0.0.1:5101 (records every second)
          Pump 1 engine    CAN vcan0, J1939 EEC1 / ET1 (only if vcan0 exists)
bottling  Line PLC         Modbus TCP 127.0.0.1:5031
          Conveyor VFD     Modbus TCP 127.0.0.1:5032
          Filler VFD       Modbus TCP 127.0.0.1:5034
          Energy meter     Modbus TCP 127.0.0.1:5033
          Robot cell       Sparkplug B edge Bottling/Line2/Robot1 → MQTT 127.0.0.1:1886
water     Lift station RTU DNP3 TCP 127.0.0.1:20001 (outstation 20, master 1)
          Chlorine analyzer Modbus TCP 127.0.0.1:5041
";

fn main() {
    let arg = std::env::args().nth(1).unwrap_or_default();
    if arg == "perf" {
        std::process::exit(perf::run(std::env::args().skip(2).collect()));
    }
    if arg == "watch" || arg == "check" {
        std::process::exit(watch::run(arg == "check", std::env::args().skip(2).collect()));
    }
    let stop = Arc::new(AtomicBool::new(false));
    let s2 = stop.clone();
    let _ = ctrlc::set_handler(move || s2.store(true, Ordering::SeqCst));
    let mut handles = Vec::new();
    match arg.as_str() {
        "frac" => handles.extend(frac::start(stop.clone())),
        "bottling" => handles.extend(bottling::start(stop.clone())),
        "water" => handles.extend(water::start(stop.clone())),
        "all" => {
            handles.extend(frac::start(stop.clone()));
            handles.extend(bottling::start(stop.clone()));
            handles.extend(water::start(stop.clone()));
        }
        _ => {
            print!("{HELP}");
            std::process::exit(if arg == "--help" || arg == "-h" { 0 } else { 2 });
        }
    }
    println!("pb-sim {arg} running — Ctrl+C to stop");
    while !stop.load(Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    println!("stopping");
    for h in handles {
        let _ = h.join();
    }
}
