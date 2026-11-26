//! Frac spread: four triplex frac pumps and a blender (Modbus TCP), the wellhead RTU (DNP3),
//! a pressure-pumping WITS feed and pump 1's engine bus (CAN J1939, when vcan0 exists).
//!
//! A stage cycle lasts four minutes: idle → rate ramp → pad / sand ramp → flush → shut down.
//! Commands from the hub: pump enable (coil 1), rate setpoint (holding 20, 0 = follow the
//! stage plan), and the ESD binary output on the RTU, which trips every pump.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pb_modbus::codec::Area;
use pb_proto::dnp3::Outstation;

use crate::devices::{approach, bank, spawn_dnp3, Broadcast, ModbusDev, Noise};

const CYCLE: f64 = 240.0;

/// Stage plan at `t` seconds into the cycle: (rate per pump bbl/min, sand ppa, phase name).
pub fn plan(t: f64) -> (f64, f64, &'static str) {
    match t {
        t if t < 20.0 => (0.0, 0.0, "idle"),
        t if t < 60.0 => ((t - 20.0) / 40.0 * 22.0, 0.0, "ramp"),
        t if t < 90.0 => (22.0, 0.0, "pad"),
        t if t < 180.0 => (22.0, 0.5 + (t - 90.0) / 90.0 * 2.5, "sand"),
        t if t < 210.0 => (22.0, 0.0, "flush"),
        t => ((240.0 - t) / 30.0 * 22.0, 0.0, "shutdown"),
    }
}

struct Pump {
    dev: ModbusDev,
    rate: f64,
    rpm: f64,
    hours: f64,
    strokes: f64,
    coolant: f64,
    trans: f64,
    gear: f64,
}

pub fn start(stop: Arc<AtomicBool>) -> Vec<JoinHandle<()>> {
    println!("frac spread:");
    let mut handles = Vec::new();
    let mut pumps = Vec::new();
    for i in 0..4u16 {
        let b = bank(&[(Area::Holding, 0, 24), (Area::Coil, 0, 2), (Area::Discrete, 0, 1)]);
        let (dev, h) = ModbusDev::spawn(&format!("Frac pump {}", i + 1), 5021 + i, 1, b, stop.clone());
        dev.coil(1, true); // pump enabled
        handles.push(h);
        pumps.push(Pump { dev, rate: 0.0, rpm: 700.0, hours: 4200.0 + i as f64 * 310.0, strokes: 1.0e6 * (i + 1) as f64, coolant: 180.0, trans: 170.0, gear: 0.0 });
    }
    let (blender, h) = ModbusDev::spawn("Blender", 5025, 1, bank(&[(Area::Holding, 0, 10), (Area::Coil, 0, 1)]), stop.clone());
    handles.push(h);
    let mut o = Outstation::new(10);
    for i in 0..4 {
        o.analogs.insert(i, 0.0);
    }
    o.binaries.insert(0, true);
    o.binaries.insert(1, false);
    o.counters.insert(0, 1);
    o.binary_outputs.insert(0, false);
    let rtu = Arc::new(Mutex::new(o));
    handles.push(spawn_dnp3("Wellhead RTU", 20000, rtu.clone(), stop.clone()));
    let (wits, h) = Broadcast::spawn("WITS feed", 5101, stop.clone());
    handles.push(h);

    let st = stop.clone();
    handles.push(std::thread::spawn(move || {
        #[cfg(target_os = "linux")]
        let can = pb_proto::can::CanSocket::open("vcan0").ok();
        #[cfg(target_os = "linux")]
        if can.is_some() {
            println!("  Pump 1 engine      CAN vcan0 (J1939 EEC1 / ET1)");
        }
        let t0 = Instant::now();
        let mut noise = Noise::new(42);
        let mut stage = 1u32;
        let mut last_cycle = 0u64;
        let mut proppant = 0.0f64;
        let mut depth = 9850.0f64;
        let mut tub = 62.0f64;
        let mut last_wits = Instant::now();
        while !st.load(Ordering::SeqCst) {
            let el = t0.elapsed().as_secs_f64();
            let cycle = (el / CYCLE) as u64;
            if cycle != last_cycle {
                last_cycle = cycle;
                stage += 1;
                depth -= 210.0; // next plug, up-hole
                if depth < 7000.0 {
                    depth = 9850.0;
                }
            }
            let t = el % CYCLE;
            let (plan_rate, ppa, _phase) = plan(t);
            let esd = rtu.lock().map(|o| o.binary_outputs.get(&0).copied().unwrap_or(false)).unwrap_or(false);
            let mut total = 0.0;
            let mut max_dp: f64 = 0.0;
            for (i, p) in pumps.iter_mut().enumerate() {
                let enabled = p.dev.get_coil(1) && !esd;
                let sp = p.dev.get_f32_cdab(20);
                let target = if !enabled { 0.0 } else if sp > 0.0 { sp.min(30.0) } else { plan_rate * (1.0 + i as f64 * 0.015) };
                p.rate = approach(p.rate, target, 1.6);
                let running = p.rate > 0.3;
                p.gear = if running { (p.rate / 5.0).ceil().clamp(1.0, 7.0) } else { 0.0 };
                p.rpm = approach(p.rpm, if running { 1450.0 + p.rate * 18.0 } else if enabled { 700.0 } else { 0.0 }, 120.0);
                p.hours += 0.5 / 3600.0;
                p.strokes += p.rate * 0.5 / 60.0 * 42.0 / 0.09; // ~0.09 bbl / stroke
                p.coolant = approach(p.coolant, if running { 195.0 + p.rate * 0.4 } else { 175.0 }, 0.3);
                p.trans = approach(p.trans, if running { 185.0 + p.rate * 0.3 } else { 160.0 }, 0.25);
                total += p.rate;
            }
            // Treating pressure rises with rate and proppant friction.
            let treating = if total > 1.0 { 5200.0 + total * 38.0 + ppa * 260.0 + noise.next() * 60.0 } else { 300.0 + noise.next() * 20.0 };
            for p in pumps.iter_mut() {
                let dp = if p.rate > 0.3 { treating + 120.0 + noise.next() * 40.0 } else { 80.0 };
                max_dp = max_dp.max(dp);
                let d = &p.dev;
                d.f32_cdab(0, dp);
                d.f32_cdab(2, noise.jitter(if p.rate > 0.3 { 95.0 } else { 40.0 }, 3.0));
                d.f32_cdab(4, p.rate);
                d.u16(6, p.rpm);
                d.u16(7, p.gear);
                d.i16(8, p.coolant * 10.0);
                d.i16(9, p.trans * 10.0);
                d.u32(10, p.hours * 10.0);
                d.u32(12, p.strokes);
                d.coil(0, p.rpm > 100.0);
                d.discrete(0, dp > 9500.0);
            }
            proppant += total * 42.0 * ppa * 0.5 / 60.0;
            let slurry = total;
            let clean = slurry / (1.0 + ppa * 0.0456);
            tub = approach(tub, if slurry > 1.0 { 55.0 + noise.next() * 4.0 } else { 62.0 }, 0.4);
            blender.f32_cdab(0, clean);
            blender.f32_cdab(2, slurry);
            blender.f32_cdab(4, ppa);
            blender.f32_cdab(6, tub);
            blender.f32_cdab(8, 8.34 + ppa * 0.55);
            blender.coil(0, slurry > 0.5);
            if let Ok(mut o) = rtu.lock() {
                o.analogs.insert(0, treating);
                o.analogs.insert(1, 450.0 + noise.next() * 15.0);
                o.analogs.insert(2, 96.0 + total * 0.05);
                o.analogs.insert(3, total);
                o.binaries.insert(0, true);
                o.binaries.insert(1, esd);
                o.counters.insert(0, stage);
            }
            if last_wits.elapsed() >= Duration::from_secs(1) {
                last_wits = Instant::now();
                let block = pb_proto::wits::encode_block(&[
                    ("0108".into(), format!("{depth:.1}")),
                    ("0121".into(), format!("{treating:.0}")),
                    ("0130".into(), format!("{slurry:.2}")),
                    ("1101".into(), format!("{ppa:.2}")),
                    ("1102".into(), stage.to_string()),
                    ("1103".into(), format!("{proppant:.0}")),
                ]);
                wits.send(&block);
            }
            #[cfg(target_os = "linux")]
            if let Some(sock) = &can {
                use pb_proto::can::{Frame, Order, SigType, Signal};
                let mut eec1 = [0xFFu8; 8];
                let rpm = Signal { start: 24, len: 16, order: Order::Intel, kind: SigType::Unsigned, scale: 0.125, offset: 0.0 };
                let _ = rpm.encode(&mut eec1, pumps[0].rpm);
                let _ = sock.write(&Frame { id: 0x0CF00400, extended: true, dlc: 8, data: eec1 });
                let mut et1 = [0xFFu8; 8];
                let cool = Signal { start: 0, len: 8, order: Order::Intel, kind: SigType::Unsigned, scale: 1.0, offset: -40.0 };
                let _ = cool.encode(&mut et1, (pumps[0].coolant - 32.0) / 1.8);
                let _ = sock.write(&Frame { id: 0x18FEEE00, extended: true, dlc: 8, data: et1 });
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }));
    handles
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_plan_is_continuous_and_bounded() {
        let mut prev = plan(0.0).0;
        let mut t = 0.0;
        while t < CYCLE {
            let (r, ppa, _) = plan(t);
            assert!((0.0..=22.0).contains(&r), "{t}: {r}");
            assert!((0.0..=3.0).contains(&ppa));
            assert!((r - prev).abs() < 1.0, "jump at {t}");
            prev = r;
            t += 0.5;
        }
        assert_eq!(plan(120.0).2, "sand");
    }
}
