//! Modbus RTU over real serial devices (a linked pty pair).
//!
//! Ignored by default — it needs two connected serial ports. Run it with
//! `scripts/modbus-rtu-smoke.sh`, which creates the pair and sets `PB_RTU_PORTS=<a>,<b>`.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pb_modbus::bank::RegisterBank;
use pb_modbus::client::{open_serial, LinkConfig, Session};
use pb_modbus::codec::{self, Area, ByteOrder, DataType, Value};
use pb_modbus::frame::{ModbusError, Request, Response};
use pb_modbus::server::{serve_rtu, ServeCtx};

fn ports() -> Option<(String, String)> {
    let v = std::env::var("PB_RTU_PORTS").ok()?;
    let (a, b) = v.split_once(',')?;
    Some((a.trim().to_string(), b.trim().to_string()))
}

fn link(path: &str, parity: &str) -> LinkConfig {
    LinkConfig {
        transport: "rtu".into(),
        host: String::new(),
        port: 502,
        serial_port: path.into(),
        baud: 19_200,
        data_bits: 8,
        parity: parity.into(),
        stop_bits: 1,
        timeout_ms: 400,
    }
}

/// Start an RTU slave (unit 5) on `path`; returns its bank and a stop handle.
fn slave(path: &str, parity: &str) -> (Arc<Mutex<RegisterBank>>, ServeCtx, std::thread::JoinHandle<()>) {
    let mut bank = RegisterBank::default();
    bank.define(Area::Holding, 0, 300);
    bank.define(Area::Coil, 0, 64);
    bank.define(Area::Input, 0, 4);
    bank.set_regs(Area::Holding, 0, &[0xE979, 0x42F6, 7]); // f32 123.456 (CDAB), u16 7
    bank.set_regs(Area::Input, 0, &[0x0100, 0x0000, 0x0001, 0x86A0]);
    let bank = Arc::new(Mutex::new(bank));
    let ctx = ServeCtx::new(bank.clone(), 5);
    let mut port = open_serial(&link(path, parity)).expect("open slave port");
    port.set_timeout(Duration::from_millis(20)).unwrap();
    let c = ctx.clone();
    let handle = std::thread::spawn(move || {
        let _ = serve_rtu(&mut port, &c, Duration::from_millis(100));
    });
    (bank, ctx, handle)
}

#[test]
#[ignore = "needs a linked serial pair: run scripts/modbus-rtu-smoke.sh"]
fn rtu_master_and_slave_over_real_serial_ports() {
    let Some((master_path, slave_path)) = ports() else {
        panic!("PB_RTU_PORTS=<master>,<slave> not set — run scripts/modbus-rtu-smoke.sh");
    };
    for parity in ["none", "even"] {
        let (bank, ctx, handle) = slave(&slave_path, parity);
        let mut m = Session::open(&link(&master_path, parity)).expect("open master port");

        // Typed reads: f32 CDAB from holding, i16 DCBA from input, u32 from input.
        let w = m.read_regs(5, Area::Holding, 0, 2).unwrap();
        assert_eq!(codec::decode(DataType::F32, ByteOrder::Cdab, &w).unwrap(), Value::Float(123.456f32 as f64));
        let w = m.read_regs(5, Area::Input, 0, 1).unwrap();
        assert_eq!(codec::decode(DataType::I16, ByteOrder::Dcba, &w).unwrap(), Value::Int(1));
        let w = m.read_regs(5, Area::Input, 2, 2).unwrap();
        assert_eq!(codec::decode(DataType::U32, ByteOrder::Abcd, &w).unwrap(), Value::UInt(100_000));

        // Writes: single register, coil, multi-register, multi-coil.
        assert_eq!(m.transact(5, &Request::WriteReg { addr: 10, value: 0xBEEF }).unwrap(), Response::Written);
        assert_eq!(m.read_regs(5, Area::Holding, 10, 1).unwrap(), vec![0xBEEF]);
        m.transact(5, &Request::WriteCoil { addr: 3, on: true }).unwrap();
        m.transact(5, &Request::WriteCoils { start: 8, values: vec![true, false, true, true, false, true, false, true, true] }).unwrap();
        assert_eq!(m.read_bits(5, Area::Coil, 3, 1).unwrap(), vec![true]);
        assert_eq!(
            m.read_bits(5, Area::Coil, 8, 9).unwrap(),
            vec![true, false, true, true, false, true, false, true, true]
        );

        // Maximum-size frames survive the serial driver: 125 registers each way.
        let big: Vec<u16> = (0..120).map(|i| i * 3 + 1).collect();
        m.transact(5, &Request::WriteRegs { start: 100, values: big.clone() }).unwrap();
        assert_eq!(m.read_regs(5, Area::Holding, 100, 120).unwrap(), big);
        assert_eq!(m.read_regs(5, Area::Holding, 0, 125).unwrap().len(), 125);

        // Exceptions come back as exceptions, not link errors; the line stays usable.
        assert_eq!(m.read_regs(5, Area::Holding, 5000, 1), Err(ModbusError::Exception(2)));
        assert_eq!(m.read_regs(5, Area::Holding, 2, 1).unwrap(), vec![7]);

        // A different unit id gets silence (timeout), like a real bus.
        assert_eq!(m.read_regs(9, Area::Holding, 0, 1), Err(ModbusError::Timeout));
        assert_eq!(m.read_regs(5, Area::Holding, 2, 1).unwrap(), vec![7], "recovers after a timeout");

        // Broadcast writes are applied and never answered.
        assert_eq!(m.transact(0, &Request::WriteReg { addr: 20, value: 99 }).unwrap(), Response::Written);
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(bank.lock().unwrap().regs(Area::Holding, 20, 1), Some(vec![99]));

        // Soak: back-to-back requests, every one must answer correctly.
        for i in 0..200u16 {
            m.transact(5, &Request::WriteReg { addr: 30, value: i }).unwrap();
            assert_eq!(m.read_regs(5, Area::Holding, 30, 1).unwrap(), vec![i], "round {i}");
        }

        ctx.stop.store(true, Ordering::SeqCst);
        handle.join().unwrap();
        drop(m);
        println!("RTU over pty OK ({parity} parity)");
    }
}
