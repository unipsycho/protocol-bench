//! `pb-sim watch` / `pb-sim check`: read points from the running service over IPC.

use std::time::{Duration, Instant};

pub fn run(check: bool, args: Vec<String>) -> i32 {
    let mut names = Vec::new();
    let mut timeout = 60.0f64;
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        if a == "--timeout" {
            timeout = it.next().and_then(|t| t.parse().ok()).unwrap_or(timeout);
        } else {
            names.push(a);
        }
    }
    let end = Instant::now() + Duration::from_secs_f64(timeout);
    let handle = loop {
        match pb_ipc::RemoteHandle::connect() {
            Ok(h) => break h,
            Err(e) if Instant::now() > end => {
                eprintln!("cannot reach the service: {e}");
                return 2;
            }
            Err(_) => std::thread::sleep(Duration::from_millis(300)),
        }
    };
    loop {
        let snap = handle.snapshot();
        let rows: Vec<_> = snap.points.iter().filter(|p| names.is_empty() || names.contains(&p.name)).collect();
        if check {
            let missing: Vec<&String> = names.iter().filter(|n| !rows.iter().any(|p| &p.name == *n && p.quality == "good")).collect();
            if missing.is_empty() && !names.is_empty() {
                for p in &rows {
                    println!("{:<28} {:<14} {}", p.name, p.value, p.quality);
                }
                println!("CHECK_OK {} points good", names.len());
                return 0;
            }
            if Instant::now() > end {
                eprintln!("not good after {timeout:.0} s: {missing:?}");
                for p in &snap.points {
                    eprintln!("  {:<28} {:<14} {:<10} {}", p.name, p.value, p.quality, p.source);
                }
                for s in &snap.slots {
                    eprintln!("  slot {:?}: {} {}", s.id, s.state, s.last_error);
                }
                return 1;
            }
            std::thread::sleep(Duration::from_millis(500));
        } else {
            print!("\x1b[2J\x1b[H");
            println!("{} points  ·  {} slots", snap.points.len(), snap.slots.len());
            for p in rows {
                println!("{:<28} {:<16} {:<9} {}", p.name, p.value.chars().take(16).collect::<String>(), p.quality, p.source);
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    }
}
