//! Helpers for packs that run one background thread per connection / file / target.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Sleep for `total`, waking every 20 ms so a stop request is honoured promptly.
/// Returns `true` when the full time elapsed and the worker should keep running.
pub fn sleep_stop(stop: &AtomicBool, total: Duration) -> bool {
    let end = Instant::now() + total;
    while Instant::now() < end && !stop.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(20).min(end.saturating_duration_since(Instant::now())));
    }
    !stop.load(Ordering::SeqCst)
}

/// Mutate a worker's shared status. A poisoned lock (a worker panicked) is skipped, not propagated:
/// status is display-only and must never take the pack down.
pub fn update_status<T>(status: &Arc<Mutex<T>>, f: impl FnOnce(&mut T)) {
    if let Ok(mut s) = status.lock() {
        f(&mut s);
    }
}

/// Make a connection / source id safe to use as a file name (`[A-Za-z0-9_-]`, else `_`).
pub fn sanitize_id(id: &str) -> String {
    id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sleep_stop_returns_early_when_stopped() {
        let stop = AtomicBool::new(true);
        let t = Instant::now();
        assert!(!sleep_stop(&stop, Duration::from_secs(5)));
        assert!(t.elapsed() < Duration::from_millis(500));
    }

    #[test]
    fn sleep_stop_reports_a_full_sleep() {
        assert!(sleep_stop(&AtomicBool::new(false), Duration::from_millis(30)));
    }

    #[test]
    fn status_update_and_poison_are_quiet() {
        let s = Arc::new(Mutex::new(1));
        update_status(&s, |v| *v += 1);
        assert_eq!(*s.lock().unwrap(), 2);
        let s2 = s.clone();
        let _ = std::thread::spawn(move || {
            let _g = s2.lock().unwrap();
            panic!("poison");
        })
        .join();
        update_status(&s, |v| *v += 1); // must not panic
    }

    #[test]
    fn ids_become_file_names() {
        assert_eq!(sanitize_id("well-a/1 x"), "well-a_1_x");
    }
}
