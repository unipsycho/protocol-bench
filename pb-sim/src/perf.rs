//! `pb-sim perf` — CPU and memory of running processes, read from `/proc` (Linux).
//!
//! ```text
//! pb-sim perf <pid> [pid …] [--secs N] [--label TEXT]
//! ```
//! Samples once a second for N seconds (default 10) and prints one line per process:
//! average and peak CPU % (of one core), RSS at the end and peak, thread count, open fds.
//! `--label` is echoed at the start of each line so runs can be collected into a table.

use std::time::{Duration, Instant};

/// Fields of `/proc/<pid>/stat` we need. `comm` may contain spaces and parentheses,
/// so fields are counted from the last `)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stat {
    pub utime: u64,
    pub stime: u64,
    pub threads: u64,
    pub rss_pages: u64,
}

pub fn parse_stat(text: &str) -> Option<Stat> {
    let rest = &text[text.rfind(')')? + 1..];
    let f: Vec<&str> = rest.split_whitespace().collect();
    // After ")": state(0) ppid(1) … utime(11) stime(12) … num_threads(17) … rss(21)
    Some(Stat {
        utime: f.get(11)?.parse().ok()?,
        stime: f.get(12)?.parse().ok()?,
        threads: f.get(17)?.parse().ok()?,
        rss_pages: f.get(21)?.parse().ok()?,
    })
}

/// CPU % of one core used between two samples `dt` apart (clock ticks → seconds).
pub fn cpu_percent(a: &Stat, b: &Stat, dt: Duration, ticks_per_sec: u64) -> f64 {
    let used = (b.utime + b.stime).saturating_sub(a.utime + a.stime) as f64 / ticks_per_sec as f64;
    let secs = dt.as_secs_f64();
    if secs <= 0.0 { 0.0 } else { used / secs * 100.0 }
}

#[derive(Debug, Default, Clone)]
pub struct Summary {
    pub cpu_avg: f64,
    pub cpu_peak: f64,
    pub rss_mib: f64,
    pub rss_peak_mib: f64,
    pub threads: u64,
    pub fds: usize,
}

const TICKS: u64 = 100; // USER_HZ on Linux
const PAGE: f64 = 4096.0;

fn read(pid: u32) -> Option<Stat> {
    parse_stat(&std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)
}

fn fds(pid: u32) -> usize {
    std::fs::read_dir(format!("/proc/{pid}/fd")).map(|d| d.count()).unwrap_or(0)
}

fn name(pid: u32) -> String {
    std::fs::read_to_string(format!("/proc/{pid}/comm")).map(|s| s.trim().to_string()).unwrap_or_else(|_| "?".into())
}

pub fn sample(pids: &[u32], secs: u64) -> Vec<Option<Summary>> {
    let mut last: Vec<Option<(Stat, Instant)>> = pids.iter().map(|p| read(*p).map(|s| (s, Instant::now()))).collect();
    let mut sums: Vec<Summary> = vec![Summary::default(); pids.len()];
    let first: Vec<Option<(Stat, Instant)>> = last.clone();
    for _ in 0..secs.max(1) {
        std::thread::sleep(Duration::from_secs(1));
        for (i, pid) in pids.iter().enumerate() {
            let (Some((prev, t0)), Some(now)) = (last[i], read(*pid)) else { continue };
            let t1 = Instant::now();
            let cpu = cpu_percent(&prev, &now, t1 - t0, TICKS);
            let s = &mut sums[i];
            s.cpu_peak = s.cpu_peak.max(cpu);
            s.rss_mib = now.rss_pages as f64 * PAGE / 1048576.0;
            s.rss_peak_mib = s.rss_peak_mib.max(s.rss_mib);
            s.threads = now.threads;
            last[i] = Some((now, t1));
        }
    }
    pids.iter()
        .enumerate()
        .map(|(i, pid)| {
            let ((a, t0), (b, t1)) = (first[i]?, last[i]?);
            let mut s = sums[i].clone();
            s.cpu_avg = cpu_percent(&a, &b, t1 - t0, TICKS);
            s.fds = fds(*pid);
            Some(s)
        })
        .collect()
}

pub fn run(args: Vec<String>) -> i32 {
    let mut pids = Vec::new();
    let mut secs = 10;
    let mut label = String::new();
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--secs" => secs = it.next().and_then(|v| v.parse().ok()).unwrap_or(10),
            "--label" => label = it.next().unwrap_or_default(),
            other => match other.parse::<u32>() {
                Ok(p) => pids.push(p),
                Err(_) => {
                    eprintln!("pb-sim perf: not a pid: {other}");
                    return 2;
                }
            },
        }
    }
    if pids.is_empty() {
        eprintln!("usage: pb-sim perf <pid> [pid …] [--secs N] [--label TEXT]");
        return 2;
    }
    let names: Vec<String> = pids.iter().map(|p| name(*p)).collect();
    for ((pid, n), s) in pids.iter().zip(names).zip(sample(&pids, secs)) {
        match s {
            Some(s) => println!(
                "{label}\t{n}\tpid={pid}\tcpu_avg={:.1}%\tcpu_peak={:.1}%\trss={:.1}MiB\trss_peak={:.1}MiB\tthreads={}\tfds={}",
                s.cpu_avg, s.cpu_peak, s.rss_mib, s.rss_peak_mib, s.threads, s.fds
            ),
            None => println!("{label}\t{n}\tpid={pid}\tgone"),
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: &str = "1234 (pb service (x)) S 1 1234 1234 0 -1 4194560 5000 0 0 0 250 50 0 0 20 0 17 0 900 123456789 2560 18446744073709551615";

    #[test]
    fn parses_stat_even_with_parens_in_the_name() {
        let s = parse_stat(LINE).unwrap();
        assert_eq!(s, Stat { utime: 250, stime: 50, threads: 17, rss_pages: 2560 });
        assert!(parse_stat("garbage").is_none());
        assert!(parse_stat("1 (x) S 1 2").is_none(), "too few fields");
    }

    #[test]
    fn cpu_percent_is_of_one_core() {
        let a = Stat { utime: 100, stime: 0, threads: 1, rss_pages: 0 };
        let b = Stat { utime: 150, stime: 25, threads: 1, rss_pages: 0 };
        // 75 ticks = 0.75 s of CPU over 1.5 s wall = 50 %.
        let p = cpu_percent(&a, &b, Duration::from_millis(1500), 100);
        assert!((p - 50.0).abs() < 1e-9, "{p}");
        assert_eq!(cpu_percent(&b, &a, Duration::from_secs(1), 100), 0.0, "counter going back reads 0");
        assert_eq!(cpu_percent(&a, &b, Duration::ZERO, 100), 0.0);
    }

    #[test]
    fn samples_this_process() {
        let me = std::process::id();
        let out = sample(&[me], 1);
        let s = out[0].as_ref().expect("own process readable");
        assert!(s.rss_mib > 0.1 && s.threads >= 1 && s.fds > 0, "{s:?}");
        assert!(sample(&[u32::MAX - 1], 1)[0].is_none(), "missing pid");
    }
}
