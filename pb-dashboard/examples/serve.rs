//! Standalone demo server for trying the dashboard in a browser without the service:
//! `cargo run -p pb-dashboard --example serve [port]` then open http://127.0.0.1:<port>/.
//! Feeds two made-up points (a sine and a counter).

use pb_extkit::{testhost, ExtPack};
use std::time::{Duration, Instant};

fn main() {
    let port: u16 = std::env::args().nth(1).and_then(|p| p.parse().ok()).unwrap_or(18780);
    let (_log, host) = testhost::host();
    let mut p = ExtPack::new(host, "dashboard", pb_dashboard::pack::run, pb_dashboard::pack::endpoint, pb_dashboard::pack::check);
    p.configure(
        &serde_json::json!({
            "connections":[{"id":"demo","host":"127.0.0.1","port":port,"title":"Demo dashboard"}],
            "outputs":[{"connection":"demo","interval_secs":0.25,"binds":[{"point":"pressure","units":"psi"},{"point":"count"}]}]
        })
        .to_string(),
    )
    .unwrap();
    println!("serving http://127.0.0.1:{port}/  (Ctrl-C to stop)");
    let t0 = Instant::now();
    let mut n = 0u64;
    loop {
        let t = t0.elapsed().as_secs_f64();
        p.write_point("pressure", &format!("{:.1}", 3000.0 + 400.0 * (t / 3.0).sin()));
        p.write_point("count", &n.to_string());
        n += 1;
        std::thread::sleep(Duration::from_millis(500));
    }
}
