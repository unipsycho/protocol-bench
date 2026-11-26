//! Building blocks for the simulated devices.

use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use pb_modbus::bank::RegisterBank;
use pb_modbus::codec::{self, Area, ByteOrder, DataType, Value};
use pb_proto::dnp3::{self, Outstation};

/// A Modbus TCP device: a register bank served on a port.
#[derive(Clone)]
pub struct ModbusDev {
    pub bank: Arc<Mutex<RegisterBank>>,
}

impl ModbusDev {
    /// Serve `bank` on 127.0.0.1:`port` (every connection on its own thread).
    pub fn spawn(name: &str, port: u16, unit: u8, bank: RegisterBank, stop: Arc<AtomicBool>) -> (ModbusDev, JoinHandle<()>) {
        let bank = Arc::new(Mutex::new(bank));
        let dev = ModbusDev { bank: bank.clone() };
        let name = name.to_string();
        let h = std::thread::spawn(move || {
            let l = match TcpListener::bind(("0.0.0.0", port)) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("{name}: cannot listen on :{port}: {e}");
                    return;
                }
            };
            println!("  {name:<18} Modbus TCP :{port} unit {unit}");
            let mut ctx = pb_modbus::server::ServeCtx::new(bank, unit);
            ctx.stop = stop;
            pb_modbus::server::run_tcp_listener(l, ctx);
        });
        (dev, h)
    }

    pub fn put(&self, area: Area, addr: u16, dt: DataType, order: ByteOrder, v: Value) {
        if let Ok(words) = codec::encode(dt, order, &v) {
            if let Ok(mut b) = self.bank.lock() {
                b.set_regs(area, addr, &words);
            }
        }
    }
    pub fn f32_cdab(&self, addr: u16, x: f64) {
        self.put(Area::Holding, addr, DataType::F32, ByteOrder::Cdab, Value::Float(x));
    }
    pub fn input_f32(&self, addr: u16, x: f64) {
        self.put(Area::Input, addr, DataType::F32, ByteOrder::Abcd, Value::Float(x));
    }
    pub fn u16(&self, addr: u16, x: f64) {
        self.put(Area::Holding, addr, DataType::U16, ByteOrder::Abcd, Value::UInt(x.round().clamp(0.0, 65535.0) as u64));
    }
    pub fn i16(&self, addr: u16, x: f64) {
        self.put(Area::Holding, addr, DataType::I16, ByteOrder::Abcd, Value::Int(x.round().clamp(-32768.0, 32767.0) as i64));
    }
    pub fn u32(&self, addr: u16, x: f64) {
        self.put(Area::Holding, addr, DataType::U32, ByteOrder::Abcd, Value::UInt(x.max(0.0).round() as u64));
    }
    pub fn coil(&self, addr: u16, on: bool) {
        if let Ok(mut b) = self.bank.lock() {
            b.coils.insert(addr, on);
        }
    }
    pub fn discrete(&self, addr: u16, on: bool) {
        if let Ok(mut b) = self.bank.lock() {
            b.discrete.insert(addr, on);
        }
    }
    pub fn get_coil(&self, addr: u16) -> bool {
        self.bank.lock().ok().and_then(|b| b.coils.get(&addr).copied()).unwrap_or(false)
    }
    pub fn get_f32_cdab(&self, addr: u16) -> f64 {
        let w = self.bank.lock().ok().and_then(|b| b.regs(Area::Holding, addr, 2));
        match w.map(|w| codec::decode(DataType::F32, ByteOrder::Cdab, &w)) {
            Some(Ok(Value::Float(x))) => x,
            _ => 0.0,
        }
    }
    pub fn get_u16(&self, addr: u16) -> u16 {
        self.bank.lock().ok().and_then(|b| b.holding.get(&addr).copied()).unwrap_or(0)
    }
}

/// A bank with these ranges defined (zeroed).
pub fn bank(ranges: &[(Area, u16, u16)]) -> RegisterBank {
    let mut b = RegisterBank::default();
    for (a, s, n) in ranges {
        b.define(*a, *s, *n);
    }
    b
}

/// A DNP3 outstation served on a port (one master at a time).
pub fn spawn_dnp3(name: &str, port: u16, db: Arc<Mutex<Outstation>>, stop: Arc<AtomicBool>) -> JoinHandle<()> {
    let name = name.to_string();
    std::thread::spawn(move || {
        let l = match TcpListener::bind(("0.0.0.0", port)) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("{name}: cannot listen on :{port}: {e}");
                return;
            }
        };
        let addr = db.lock().map(|o| o.address).unwrap_or(0);
        println!("  {name:<18} DNP3 TCP :{port} outstation {addr}");
        let _ = l.set_nonblocking(true);
        while !stop.load(Ordering::SeqCst) {
            let Ok((mut s, _)) = l.accept() else {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            };
            let _ = s.set_nonblocking(false);
            let _ = s.set_read_timeout(Some(Duration::from_millis(500)));
            let mut seq = 0u8;
            while !stop.load(Ordering::SeqCst) {
                let (master, apdu) = match dnp3::read_fragment(&mut s, addr) {
                    Ok(x) => x,
                    Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => continue,
                    Err(_) => break,
                };
                let resp = db.lock().ok().and_then(|mut o| o.handle(&apdu));
                if let Some(r) = resp {
                    if s.write_all(&dnp3::encode_fragment(false, master, addr, &r, &mut seq)).is_err() {
                        break;
                    }
                }
            }
        }
    })
}

/// A TCP server that sends text to every connected client.
#[derive(Clone)]
pub struct Broadcast {
    clients: Arc<Mutex<Vec<TcpStream>>>,
}

impl Broadcast {
    pub fn spawn(name: &str, port: u16, stop: Arc<AtomicBool>) -> (Broadcast, JoinHandle<()>) {
        let clients: Arc<Mutex<Vec<TcpStream>>> = Arc::new(Mutex::new(Vec::new()));
        let c2 = clients.clone();
        let name = name.to_string();
        let h = std::thread::spawn(move || {
            let l = match TcpListener::bind(("0.0.0.0", port)) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("{name}: cannot listen on :{port}: {e}");
                    return;
                }
            };
            println!("  {name:<18} TCP :{port}");
            let _ = l.set_nonblocking(true);
            while !stop.load(Ordering::SeqCst) {
                match l.accept() {
                    Ok((s, _)) => {
                        let _ = s.set_nonblocking(false);
                        let _ = s.set_write_timeout(Some(Duration::from_secs(1)));
                        c2.lock().unwrap().push(s);
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(50)),
                }
            }
        });
        (Broadcast { clients }, h)
    }

    pub fn send(&self, text: &str) {
        if let Ok(mut cs) = self.clients.lock() {
            cs.retain_mut(|s| s.write_all(text.as_bytes()).is_ok());
        }
    }
}

/// Tiny deterministic noise source (xorshift).
pub struct Noise(u64);

impl Noise {
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }
    /// Uniform in [-1, 1).
    pub fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % 2_000_000) as f64 / 1_000_000.0 - 1.0
    }
    /// `x` with ±`pct` percent jitter.
    pub fn jitter(&mut self, x: f64, pct: f64) -> f64 {
        x * (1.0 + self.next() * pct / 100.0)
    }
}

/// Move `cur` toward `target` by at most `rate` per call.
pub fn approach(cur: f64, target: f64, rate: f64) -> f64 {
    if (target - cur).abs() <= rate {
        target
    } else if target > cur {
        cur + rate
    } else {
        cur - rate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_and_approach() {
        let mut n = Noise::new(7);
        for _ in 0..1000 {
            let x = n.next();
            assert!((-1.0..1.0).contains(&x));
        }
        assert_eq!(approach(0.0, 10.0, 3.0), 3.0);
        assert_eq!(approach(9.0, 10.0, 3.0), 10.0);
        assert_eq!(approach(10.0, 0.0, 4.0), 6.0);
    }

    #[test]
    fn modbus_dev_round_trips_values() {
        let stop = Arc::new(AtomicBool::new(true));
        let (d, h) = ModbusDev::spawn("t", 0, 1, bank(&[(Area::Holding, 0, 10), (Area::Coil, 0, 2)]), stop);
        let _ = h.join();
        d.f32_cdab(0, 12.5);
        assert_eq!(d.get_f32_cdab(0), 12.5);
        d.u16(4, 70000.0);
        assert_eq!(d.get_u16(4), 65535);
        d.coil(1, true);
        assert!(d.get_coil(1));
    }
}
