//! `pb-sim mqtt-check`: subscribe to a broker and verify what the hub publishes (smokes / CI).
//!
//! ```text
//! pb-sim mqtt-check <port> <filter> <topic …> [--moving <topic …>] [--timeout S]
//! ```
//!
//! Exit 0 once every listed topic has been seen at least once and every `--moving` topic has been
//! seen with at least two different payloads (proof that the data really changes, not just that
//! a retained value is sitting there).

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use rumqttc::{Client, Event, MqttOptions, Packet, QoS};

#[derive(Default)]
struct Seen {
    count: u64,
    values: BTreeSet<String>,
    last: String,
}

pub fn run(args: Vec<String>) -> i32 {
    let mut it = args.into_iter();
    let (Some(port), Some(filter)) = (it.next().and_then(|p| p.parse::<u16>().ok()), it.next()) else {
        eprintln!("usage: pb-sim mqtt-check <port> <filter> <topic …> [--moving <topic …>] [--timeout S]");
        return 2;
    };
    let (mut expect, mut moving) = (Vec::<String>::new(), Vec::<String>::new());
    let mut timeout = 45.0f64;
    let mut into_moving = false;
    while let Some(a) = it.next() {
        match a.as_str() {
            "--moving" => into_moving = true,
            "--timeout" => timeout = it.next().and_then(|t| t.parse().ok()).unwrap_or(timeout),
            _ if into_moving => moving.push(a),
            _ => expect.push(a),
        }
    }
    let mut opts = MqttOptions::new(format!("pb-sim-check-{}", std::process::id()), "127.0.0.1", port);
    opts.set_keep_alive(Duration::from_secs(5));
    let (client, mut conn) = Client::new(opts, 64);
    let end = Instant::now() + Duration::from_secs_f64(timeout);
    let mut seen: BTreeMap<String, Seen> = BTreeMap::new();
    let mut subscribed = false;
    loop {
        match conn.recv_timeout(Duration::from_millis(250)) {
            Ok(Ok(Event::Incoming(Packet::ConnAck(_)))) if !subscribed => {
                let _ = client.subscribe(filter.clone(), QoS::AtMostOnce);
                subscribed = true;
            }
            Ok(Ok(Event::Incoming(Packet::Publish(p)))) => {
                let s = seen.entry(p.topic.clone()).or_default();
                s.count += 1;
                s.last = String::from_utf8_lossy(&p.payload).into_owned();
                if s.values.len() < 8 {
                    s.values.insert(s.last.clone());
                }
            }
            Ok(Err(_)) => std::thread::sleep(Duration::from_millis(300)), // broker not up yet: rumqttc retries
            _ => {}
        }
        let missing: Vec<&String> = expect.iter().chain(moving.iter()).filter(|t| !seen.contains_key(*t)).collect();
        let still: Vec<&String> = moving.iter().filter(|t| seen.get(*t).is_none_or(|s| s.values.len() < 2)).collect();
        if missing.is_empty() && still.is_empty() && !(expect.is_empty() && moving.is_empty()) {
            for (t, s) in &seen {
                println!("{t:<34} {:<12} ×{}", s.last.chars().take(12).collect::<String>(), s.count);
            }
            println!("MQTT_CHECK_OK {} topics, {} moving", expect.len() + moving.len(), moving.len());
            let _ = client.disconnect();
            return 0;
        }
        if Instant::now() > end {
            eprintln!("mqtt-check failed after {timeout:.0} s");
            eprintln!("  never seen: {missing:?}");
            eprintln!("  not moving: {still:?}");
            for (t, s) in &seen {
                eprintln!("  {t:<34} {:<12} ×{}", s.last, s.count);
            }
            return 1;
        }
    }
}
