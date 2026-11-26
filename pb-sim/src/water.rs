//! Water lift station: a DNP3 RTU (wet well, two duty pumps, discharge header) and a chlorine
//! analyzer on Modbus TCP.
//!
//! The wet well fills with inflow; pump 1 starts at 8 ft, pump 2 assists at 11 ft, both stop
//! at 4 ft. A start command on binary output 0 / 1 forces that pump on (hand mode) until it
//! is turned off.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use pb_modbus::codec::Area;
use pb_proto::dnp3::Outstation;

use crate::devices::{bank, spawn_dnp3, ModbusDev, Noise};

pub fn start(stop: Arc<AtomicBool>) -> Vec<JoinHandle<()>> {
    println!("water lift station:");
    let mut handles = Vec::new();
    let mut o = Outstation::new(20);
    for i in 0..5 {
        o.analogs.insert(i, 0.0);
    }
    for i in 0..4 {
        o.binaries.insert(i, false);
    }
    o.counters.insert(0, 1520);
    o.counters.insert(1, 1498);
    o.binary_outputs.insert(0, false);
    o.binary_outputs.insert(1, false);
    let rtu = Arc::new(Mutex::new(o));
    handles.push(spawn_dnp3("Lift station RTU", 20001, rtu.clone(), stop.clone()));
    let (cl2, h) = ModbusDev::spawn("Chlorine analyzer", 5041, 1, bank(&[(Area::Input, 0, 8)]), stop.clone());
    handles.push(h);
    let st = stop.clone();
    handles.push(std::thread::spawn(move || {
        let mut n = Noise::new(5);
        let mut level = 6.0f64;
        let (mut p1, mut p2) = (false, false);
        let mut t = 0.0f64;
        while !st.load(Ordering::SeqCst) {
            t += 0.5;
            let inflow = 900.0 + 400.0 * (t / 300.0).sin() + n.next() * 40.0; // gpm
            let (hand1, hand2) = rtu
                .lock()
                .map(|o| (o.binary_outputs.get(&0).copied().unwrap_or(false), o.binary_outputs.get(&1).copied().unwrap_or(false)))
                .unwrap_or((false, false));
            let was = (p1, p2);
            if level > 8.0 {
                p1 = true;
            }
            if level > 11.0 {
                p2 = true;
            }
            if level < 4.0 {
                p1 = false;
                p2 = false;
            }
            let run1 = p1 || hand1;
            let run2 = p2 || hand2;
            let out = if run1 { 1150.0 } else { 0.0 } + if run2 { 1050.0 } else { 0.0 };
            // 12 ft x 12 ft wet well ≈ 1077 gal / ft.
            level = (level + (inflow - out) / 1077.0 * 0.5 / 60.0 * 10.0).clamp(0.5, 14.0);
            if let Ok(mut o) = rtu.lock() {
                if run1 && !was.0 {
                    *o.counters.entry(0).or_insert(0) += 1;
                }
                if run2 && !was.1 {
                    *o.counters.entry(1).or_insert(0) += 1;
                }
                o.analogs.insert(0, level);
                o.analogs.insert(1, if out > 0.0 { 38.0 + out / 200.0 + n.next() } else { 6.0 });
                o.analogs.insert(2, if out > 0.0 { out + n.next() * 15.0 } else { 0.0 });
                o.analogs.insert(3, if run1 { n.jitter(42.0, 2.0) } else { 0.0 });
                o.analogs.insert(4, if run2 { n.jitter(39.0, 2.0) } else { 0.0 });
                o.binaries.insert(0, run1);
                o.binaries.insert(1, run2);
                o.binaries.insert(2, level > 12.5);
                o.binaries.insert(3, false);
            }
            cl2.input_f32(0, 1.2 + 0.15 * (t / 200.0).sin() + n.next() * 0.02);
            cl2.input_f32(2, 7.2 + n.next() * 0.05);
            cl2.input_f32(4, 0.18 + n.next().abs() * 0.05);
            cl2.input_f32(6, 14.0 + n.next() * 0.2);
            std::thread::sleep(Duration::from_millis(500));
        }
    }));
    handles
}
