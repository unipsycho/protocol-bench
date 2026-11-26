//! Bottling line: line PLC, conveyor + filler VFDs, a 3-phase energy meter (Modbus TCP) and a
//! pick-and-place robot cell that reports as a Sparkplug B edge node.
//!
//! The line runs while the PLC run coil (coil 0) is on; a bottle jam stops it for ten seconds
//! about every two minutes. VFD speed follows the speed reference (holding 101, 0.01 Hz).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pb_modbus::codec::Area;
use pb_proto::sparkplug::{self as spb, dt, MValue, Metric, Payload};
use rumqttc::{Client, LastWill, MqttOptions, QoS};

use crate::devices::{approach, bank, ModbusDev, Noise};

pub const BROKER_PORT: u16 = 1886;

struct Vfd {
    dev: ModbusDev,
    hz: f64,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub fn start(stop: Arc<AtomicBool>) -> Vec<JoinHandle<()>> {
    println!("bottling line:");
    let mut handles = Vec::new();
    let (plc, h) = ModbusDev::spawn("Line PLC", 5031, 1, bank(&[(Area::Holding, 0, 12), (Area::Coil, 0, 2), (Area::Discrete, 0, 2)]), stop.clone());
    plc.coil(0, true);
    handles.push(h);
    let mut vfds = Vec::new();
    for (name, port, hz) in [("Conveyor VFD", 5032u16, 45.0), ("Filler VFD", 5034u16, 38.0)] {
        let (dev, h) = ModbusDev::spawn(name, port, 1, bank(&[(Area::Holding, 0, 8), (Area::Holding, 100, 2)]), stop.clone());
        dev.u16(101, hz * 100.0);
        dev.u16(5, 650.0);
        handles.push(h);
        vfds.push(Vfd { dev, hz: 0.0 });
    }
    let (meter, h) = ModbusDev::spawn("Energy meter", 5033, 1, bank(&[(Area::Input, 0, 22)]), stop.clone());
    handles.push(h);
    println!("  Robot cell         Sparkplug B Bottling/Line2/Robot1 → 127.0.0.1:{BROKER_PORT}");

    // Robot shared state for the Sparkplug thread.
    let robot = Arc::new(std::sync::Mutex::new((0.0f64, 0i64, 0.0f64, "IDLE".to_string(), false)));
    let st = stop.clone();
    let rb = robot.clone();
    handles.push(std::thread::spawn(move || {
        let t0 = Instant::now();
        let mut n = Noise::new(9);
        let (mut good, mut reject) = (182_340.0f64, 1_212.0f64);
        let mut bpm = 0.0f64;
        let mut kwh = 81_234.0f64;
        let mut picks = 40_112i64;
        let mut jam_until: Option<Instant> = None;
        while !st.load(Ordering::SeqCst) {
            let el = t0.elapsed().as_secs_f64();
            if jam_until.is_none() && (el as u64) % 120 == 60 && el.fract() < 0.5 {
                jam_until = Some(Instant::now() + Duration::from_secs(10));
            }
            let jam = jam_until.is_some_and(|t| Instant::now() < t);
            if jam_until.is_some_and(|t| Instant::now() >= t) {
                jam_until = None;
            }
            let run = plc.get_coil(0) && !jam;
            for v in vfds.iter_mut() {
                let sp = v.dev.get_u16(101) as f64 / 100.0;
                v.hz = approach(v.hz, if run { sp.min(60.0) } else { 0.0 }, 3.0);
                let running = v.hz > 0.5;
                let status = if running { 1 << 2 } else { 0 } | if jam { 1 << 3 } else { 0 };
                let d = &v.dev;
                d.u16(0, status as f64);
                d.u16(1, v.hz * 100.0);
                d.u16(2, v.hz * 29.4);
                d.u16(3, if running { n.jitter(v.hz * 0.31 + 2.0, 2.0) * 10.0 } else { 0.0 });
                d.i16(4, if running { n.jitter(62.0, 4.0) * 10.0 } else { 0.0 });
                d.u16(5, n.jitter(650.0, 0.5));
                d.u16(6, if jam { 23.0 } else { 0.0 });
            }
            let conveyor_hz = vfds[0].hz;
            bpm = approach(bpm, conveyor_hz / 45.0 * 400.0, 40.0);
            let made = bpm * 0.5 / 60.0;
            good += made * 0.993;
            reject += made * 0.007;
            let state = if jam { 2.0 } else if bpm > 5.0 { 1.0 } else { 0.0 };
            plc.u16(0, state);
            plc.u16(1, bpm);
            plc.u32(2, good);
            plc.u32(4, reject);
            plc.f32_cdab(6, n.jitter(if bpm > 5.0 { 4.5 } else { 6.0 }, 2.0));
            plc.f32_cdab(8, if bpm > 5.0 { n.jitter(2.4, 3.0) } else { 0.0 });
            plc.f32_cdab(10, (good / (good + reject) * 100.0 * (bpm / 400.0).min(1.0) * 0.97).max(0.0));
            plc.discrete(0, false);
            plc.discrete(1, jam);
            // Energy: base load + motors.
            let kw = 18.0 + vfds.iter().map(|v| v.hz * 0.35).sum::<f64>() + n.next() * 0.8;
            kwh += kw * 0.5 / 3600.0;
            let volts = [n.jitter(277.0, 0.4), n.jitter(276.0, 0.4), n.jitter(278.0, 0.4)];
            for (i, v) in volts.iter().enumerate() {
                meter.input_f32(i as u16 * 2, *v);
                meter.input_f32(6 + i as u16 * 2, kw * 1000.0 / (3.0 * v * 0.92));
            }
            meter.input_f32(12, kw);
            meter.input_f32(14, kw * 0.42);
            meter.input_f32(16, 0.92 + n.next() * 0.01);
            meter.input_f32(18, 60.0 + n.next() * 0.02);
            meter.input_f32(20, kwh);
            if bpm > 5.0 {
                picks += (bpm / 24.0 * 0.5 / 60.0 * 24.0).round() as i64;
            }
            if let Ok(mut r) = rb.lock() {
                *r = (if bpm > 5.0 { n.jitter(3.6, 3.0) } else { 0.0 }, picks, if bpm > 5.0 { n.jitter(41.0, 5.0) } else { 2.0 }, if jam { "FAULT".into() } else if bpm > 5.0 { "RUN".into() } else { "IDLE".into() }, bpm > 5.0);
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }));

    // Sparkplug B edge node for the robot cell.
    let st = stop.clone();
    handles.push(std::thread::spawn(move || {
        let (group, edge, dev) = ("Bottling", "Line2", "Robot1");
        let mut bd_seq = 0u64;
        while !st.load(Ordering::SeqCst) {
            let mut o = MqttOptions::new(format!("pb-sim-{edge}"), "127.0.0.1", BROKER_PORT);
            o.set_keep_alive(Duration::from_secs(10));
            let death = Payload { timestamp: Some(now_ms()), seq: None, metrics: vec![Metric::new("bdSeq", dt::UINT64, MValue::UInt(bd_seq))] };
            o.set_last_will(LastWill::new(spb::topic(group, "NDEATH", edge, ""), spb::encode(&death), QoS::AtLeastOnce, false));
            let (client, mut conn) = Client::new(o, 64);
            // Wait for CONNACK.
            let mut ok = false;
            let end = Instant::now() + Duration::from_secs(3);
            while Instant::now() < end {
                match conn.recv_timeout(Duration::from_millis(200)) {
                    Ok(Ok(rumqttc::Event::Incoming(rumqttc::Packet::ConnAck(_)))) => {
                        ok = true;
                        break;
                    }
                    Ok(Err(_)) => break,
                    _ => {}
                }
            }
            if !ok {
                std::thread::sleep(Duration::from_secs(3));
                continue;
            }
            let mut seq = 0u64;
            let send = |kind: &str, device: &str, metrics: Vec<Metric>, seq: &mut u64| {
                let p = Payload { timestamp: Some(now_ms()), seq: Some(*seq), metrics };
                *seq = (*seq + 1) % 256;
                client.try_publish(spb::topic(group, kind, edge, device), QoS::AtMostOnce, false, spb::encode(&p)).is_ok()
            };
            let metrics = |r: &(f64, i64, f64, String, bool), named: bool| {
                let m = |name: &str, alias: u64, t: u32, v: MValue| Metric { name: named.then(|| name.to_string()), alias: Some(alias), timestamp: Some(now_ms()), datatype: t, value: v };
                vec![
                    m("Cycle time", 1, dt::DOUBLE, MValue::Double(r.0)),
                    m("Picks", 2, dt::INT64, MValue::Int(r.1)),
                    m("Torque", 3, dt::DOUBLE, MValue::Double(r.2)),
                    m("State", 4, dt::STRING, MValue::Text(r.3.clone())),
                    m("Gripper closed", 5, dt::BOOLEAN, MValue::Bool(r.4)),
                ]
            };
            send("NBIRTH", "", vec![Metric::new("bdSeq", dt::UINT64, MValue::UInt(bd_seq)), Metric::new("Line/Name", dt::STRING, MValue::Text("Line 2 — 500 ml PET".into()))], &mut seq);
            let r = robot.lock().map(|r| r.clone()).unwrap_or_default();
            send("DBIRTH", dev, metrics(&r, true), &mut seq);
            let mut alive = true;
            while alive && !st.load(Ordering::SeqCst) {
                let r = robot.lock().map(|r| r.clone()).unwrap_or_default();
                send("DDATA", dev, metrics(&r, false), &mut seq);
                let end = Instant::now() + Duration::from_secs(1);
                while Instant::now() < end {
                    match conn.recv_timeout(Duration::from_millis(100)) {
                        Ok(Err(_)) | Err(rumqttc::RecvTimeoutError::Disconnected) => {
                            alive = false;
                            break;
                        }
                        _ => {}
                    }
                }
            }
            bd_seq = (bd_seq + 1) % 256;
            let _ = client.disconnect();
        }
    }));
    handles
}
