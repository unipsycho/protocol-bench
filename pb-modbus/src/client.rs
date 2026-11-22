//! Blocking Modbus master session over TCP or a serial line (RTU).

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::codec::Area;
use crate::frame::{
    mbap_frame, mbap_header, rtu_frame, rtu_response_rest, rtu_split, ModbusError, Request,
    Response,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkConfig {
    /// `tcp` | `rtu`.
    pub transport: String,
    #[serde(default)]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub serial_port: String,
    #[serde(default = "default_baud")]
    pub baud: u32,
    #[serde(default = "default_data_bits")]
    pub data_bits: u8,
    /// `none` | `even` | `odd`.
    #[serde(default = "default_parity")]
    pub parity: String,
    #[serde(default = "default_stop_bits")]
    pub stop_bits: u8,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u32,
}

fn default_port() -> u16 {
    502
}
fn default_baud() -> u32 {
    9600
}
fn default_data_bits() -> u8 {
    8
}
fn default_parity() -> String {
    "none".into()
}
fn default_stop_bits() -> u8 {
    1
}
fn default_timeout_ms() -> u32 {
    1000
}

impl LinkConfig {
    /// True for serial RTU framing (CRC, no MBAP header).
    pub fn is_rtu(&self) -> bool {
        self.transport.eq_ignore_ascii_case("rtu")
    }

    /// Response timeout as a `Duration`.
    pub fn timeout(&self) -> Duration {
        Duration::from_millis(self.timeout_ms.max(20) as u64)
    }

    /// `host:port` or serial device, for messages.
    pub fn endpoint(&self) -> String {
        if self.is_rtu() {
            self.serial_port.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

/// Byte transport under a session. Implemented for TCP and serial; tests supply a mock.
pub trait Io: Send {
    fn read_exact_buf(&mut self, buf: &mut [u8]) -> Result<(), ModbusError>;
    fn write_all_buf(&mut self, buf: &[u8]) -> Result<(), ModbusError>;
    /// Drop stale input (RTU, before a request).
    fn purge(&mut self) {}
}

fn map_io(e: std::io::Error) -> ModbusError {
    match e.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => ModbusError::Timeout,
        _ => ModbusError::Io(e.to_string()),
    }
}

struct TcpIo(TcpStream);

impl Io for TcpIo {
    fn read_exact_buf(&mut self, buf: &mut [u8]) -> Result<(), ModbusError> {
        self.0.read_exact(buf).map_err(map_io)
    }
    fn write_all_buf(&mut self, buf: &[u8]) -> Result<(), ModbusError> {
        self.0.write_all(buf).map_err(map_io)
    }
}

struct SerialIo(Box<dyn serialport::SerialPort>);

impl Io for SerialIo {
    fn read_exact_buf(&mut self, buf: &mut [u8]) -> Result<(), ModbusError> {
        self.0.read_exact(buf).map_err(map_io)
    }
    fn write_all_buf(&mut self, buf: &[u8]) -> Result<(), ModbusError> {
        self.0.write_all(buf).map_err(map_io)?;
        self.0.flush().map_err(map_io)
    }
    fn purge(&mut self) {
        let _ = self.0.clear(serialport::ClearBuffer::Input);
    }
}

/// Open the serial line described by `cfg` (shared by the master session and the RTU slave).
pub fn open_serial(cfg: &LinkConfig) -> Result<Box<dyn serialport::SerialPort>, ModbusError> {
    if cfg.serial_port.trim().is_empty() {
        return Err(ModbusError::Io("no serial port set".into()));
    }
    let parity = match cfg.parity.to_ascii_lowercase().as_str() {
        "even" => serialport::Parity::Even,
        "odd" => serialport::Parity::Odd,
        _ => serialport::Parity::None,
    };
    let data_bits = match cfg.data_bits {
        5 => serialport::DataBits::Five,
        6 => serialport::DataBits::Six,
        7 => serialport::DataBits::Seven,
        _ => serialport::DataBits::Eight,
    };
    let stop_bits = if cfg.stop_bits == 2 {
        serialport::StopBits::Two
    } else {
        serialport::StopBits::One
    };
    serialport::new(cfg.serial_port.trim(), cfg.baud)
        .data_bits(data_bits)
        .parity(parity)
        .stop_bits(stop_bits)
        .flow_control(serialport::FlowControl::None)
        .timeout(cfg.timeout())
        .open()
        .map_err(|e| ModbusError::Io(format!("{}: {e}", cfg.serial_port)))
}

pub struct Session {
    io: Box<dyn Io>,
    rtu: bool,
    tid: u16,
}

impl Session {
    /// Open the link described by `cfg`.
    pub fn open(cfg: &LinkConfig) -> Result<Self, ModbusError> {
        let timeout = cfg.timeout();
        if cfg.is_rtu() {
            let port = open_serial(cfg)?;
            return Ok(Self::with_io(Box::new(SerialIo(port)), true));
        }
        if cfg.host.trim().is_empty() {
            return Err(ModbusError::Io("no host set".into()));
        }
        let addr = (cfg.host.trim(), cfg.port)
            .to_socket_addrs()
            .map_err(|e| ModbusError::Io(format!("{}: {e}", cfg.endpoint())))?
            .next()
            .ok_or_else(|| ModbusError::Io(format!("{}: no address", cfg.endpoint())))?;
        let stream = TcpStream::connect_timeout(&addr, timeout)
            .map_err(|e| ModbusError::Io(format!("{}: {e}", cfg.endpoint())))?;
        stream.set_read_timeout(Some(timeout)).map_err(map_io)?;
        stream.set_write_timeout(Some(timeout)).map_err(map_io)?;
        let _ = stream.set_nodelay(true);
        Ok(Self::with_io(Box::new(TcpIo(stream)), false))
    }

    /// Wrap an arbitrary byte stream (tests inject a loopback; production uses TCP or a serial port).
    pub fn with_io(io: Box<dyn Io>, rtu: bool) -> Self {
        Self { io, rtu, tid: 0 }
    }

    /// One request / response round trip to `unit`.
    pub fn transact(&mut self, unit: u8, req: &Request) -> Result<Response, ModbusError> {
        let pdu = req.pdu();
        if self.rtu {
            self.io.purge();
            self.io.write_all_buf(&rtu_frame(unit, &pdu))?;
            if unit == 0 && req.function() >= 5 {
                return Ok(Response::Written); // broadcast write: no reply
            }
            let resp = self.read_rtu_frame()?;
            let (ru, rpdu) = rtu_split(&resp)?;
            if ru != unit {
                return Err(ModbusError::Protocol(format!("reply from unit {ru}, expected {unit}")));
            }
            return req.parse_response(rpdu);
        }
        self.tid = self.tid.wrapping_add(1);
        let tid = self.tid;
        self.io.write_all_buf(&mbap_frame(tid, unit, &pdu))?;
        // Skip stale replies to earlier (timed-out) requests.
        for _ in 0..4 {
            let mut h = [0u8; 7];
            self.io.read_exact_buf(&mut h)?;
            let (rtid, rest, runit) = mbap_header(&h)?;
            let mut body = vec![0u8; rest];
            self.io.read_exact_buf(&mut body)?;
            if rtid != tid {
                continue;
            }
            if runit != unit {
                return Err(ModbusError::Protocol(format!("reply from unit {runit}, expected {unit}")));
            }
            return req.parse_response(&body);
        }
        Err(ModbusError::Protocol("no reply matched the transaction id".into()))
    }

    fn read_rtu_frame(&mut self) -> Result<Vec<u8>, ModbusError> {
        let mut head = [0u8; 2];
        self.io.read_exact_buf(&mut head)?;
        let fc = head[1];
        let mut frame = head.to_vec();
        let count = if fc & 0x80 == 0 && (1..=4).contains(&fc) {
            let mut c = [0u8; 1];
            self.io.read_exact_buf(&mut c)?;
            frame.push(c[0]);
            Some(c[0])
        } else {
            None
        };
        let rest = rtu_response_rest(fc, count)
            .ok_or_else(|| ModbusError::Protocol(format!("unexpected function {fc:#04x}")))?;
        let mut tail = vec![0u8; rest];
        self.io.read_exact_buf(&mut tail)?;
        frame.extend(tail);
        Ok(frame)
    }

    /// Read coils / discrete inputs (`count` ≤ 2000). Returns one bool per bit.
    pub fn read_bits(&mut self, unit: u8, area: Area, start: u16, count: u16) -> Result<Vec<bool>, ModbusError> {
        match self.transact(unit, &Request::ReadBits { area, start, count })? {
            Response::Bits(b) => Ok(b),
            _ => Err(ModbusError::Protocol("unexpected response".into())),
        }
    }

    /// Read holding / input registers (`count` ≤ 125). Returns raw 16-bit words.
    pub fn read_regs(&mut self, unit: u8, area: Area, start: u16, count: u16) -> Result<Vec<u16>, ModbusError> {
        match self.transact(unit, &Request::ReadRegs { area, start, count })? {
            Response::Regs(r) => Ok(r),
            _ => Err(ModbusError::Protocol("unexpected response".into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bank::RegisterBank;
    use crate::frame::{exception_pdu, EX_ILLEGAL_ADDRESS};
    use crate::server;
    use std::sync::{Arc, Mutex};

    fn tcp_pair(bank: RegisterBank) -> (Session, Arc<Mutex<RegisterBank>>) {
        let bank = Arc::new(Mutex::new(bank));
        let addr = server::spawn_tcp_for_tests(bank.clone(), 1);
        let cfg = LinkConfig {
            transport: "tcp".into(),
            host: "127.0.0.1".into(),
            port: addr.port(),
            serial_port: String::new(),
            baud: 9600,
            data_bits: 8,
            parity: "none".into(),
            stop_bits: 1,
            timeout_ms: 1000,
        };
        (Session::open(&cfg).unwrap(), bank)
    }

    #[test]
    fn tcp_reads_and_writes_against_a_fake_device() {
        let mut b = RegisterBank::default();
        b.define(Area::Holding, 0, 10);
        b.define(Area::Coil, 0, 4);
        b.define(Area::Input, 5, 2);
        b.set_regs(Area::Holding, 0, &[0x42F6, 0xE979, 7]);
        b.set_regs(Area::Input, 5, &[11, 22]);
        let (mut s, bank) = tcp_pair(b);
        assert_eq!(s.read_regs(1, Area::Holding, 0, 3).unwrap(), vec![0x42F6, 0xE979, 7]);
        assert_eq!(s.read_regs(1, Area::Input, 5, 2).unwrap(), vec![11, 22]);
        assert_eq!(s.transact(1, &Request::WriteRegs { start: 4, values: vec![1, 2] }).unwrap(), Response::Written);
        assert_eq!(s.read_regs(1, Area::Holding, 4, 2).unwrap(), vec![1, 2]);
        s.transact(1, &Request::WriteCoil { addr: 2, on: true }).unwrap();
        assert_eq!(s.read_bits(1, Area::Coil, 0, 4).unwrap(), vec![false, false, true, false]);
        assert!(bank.lock().unwrap().coils[&2]);
        // Unmapped address -> exception, link stays usable.
        assert_eq!(
            s.read_regs(1, Area::Holding, 100, 1),
            Err(ModbusError::Exception(EX_ILLEGAL_ADDRESS))
        );
        assert_eq!(s.read_regs(1, Area::Holding, 0, 1).unwrap(), vec![0x42F6]);
    }

    #[test]
    fn tcp_other_unit_is_ignored_so_the_request_times_out() {
        let mut b = RegisterBank::default();
        b.define(Area::Holding, 0, 1);
        let bank = Arc::new(Mutex::new(b));
        let addr = server::spawn_tcp_for_tests(bank, 1);
        let cfg = LinkConfig {
            transport: "tcp".into(),
            host: "127.0.0.1".into(),
            port: addr.port(),
            serial_port: String::new(),
            baud: 9600,
            data_bits: 8,
            parity: "none".into(),
            stop_bits: 1,
            timeout_ms: 150,
        };
        let mut s = Session::open(&cfg).unwrap();
        assert_eq!(s.read_regs(7, Area::Holding, 0, 1), Err(ModbusError::Timeout));
    }

    /// RTU session over an in-memory "device": every written frame is answered from a bank.
    struct MockRtu {
        bank: RegisterBank,
        unit: u8,
        reply: Vec<u8>,
        corrupt: bool,
    }

    impl Io for MockRtu {
        fn write_all_buf(&mut self, buf: &[u8]) -> Result<(), ModbusError> {
            let (unit, pdu) = rtu_split(buf)?;
            self.reply.clear();
            if unit != self.unit {
                return Ok(()); // not us: silence
            }
            let req = Request::parse(pdu).unwrap();
            let rpdu = match self.bank.handle(&req) {
                Ok(r) => req.response_pdu(&r),
                Err(c) => exception_pdu(req.function(), c),
            };
            self.reply = rtu_frame(unit, &rpdu);
            if self.corrupt {
                let n = self.reply.len();
                self.reply[n - 1] ^= 0xFF;
            }
            Ok(())
        }
        fn read_exact_buf(&mut self, buf: &mut [u8]) -> Result<(), ModbusError> {
            if self.reply.len() < buf.len() {
                self.reply.clear();
                return Err(ModbusError::Timeout);
            }
            buf.copy_from_slice(&self.reply[..buf.len()]);
            self.reply.drain(..buf.len());
            Ok(())
        }
    }

    fn rtu_session(corrupt: bool) -> Session {
        let mut bank = RegisterBank::default();
        bank.define(Area::Holding, 0, 8);
        bank.define(Area::Coil, 0, 16);
        bank.set_regs(Area::Holding, 0, &[100, 200, 300]);
        Session::with_io(Box::new(MockRtu { bank, unit: 5, reply: Vec::new(), corrupt }), true)
    }

    #[test]
    fn rtu_round_trips_and_reports_exceptions() {
        let mut s = rtu_session(false);
        assert_eq!(s.read_regs(5, Area::Holding, 0, 3).unwrap(), vec![100, 200, 300]);
        s.transact(5, &Request::WriteReg { addr: 1, value: 9 }).unwrap();
        assert_eq!(s.read_regs(5, Area::Holding, 1, 1).unwrap(), vec![9]);
        s.transact(5, &Request::WriteCoils { start: 0, values: vec![true, false, true] }).unwrap();
        assert_eq!(s.read_bits(5, Area::Coil, 0, 3).unwrap(), vec![true, false, true]);
        assert_eq!(s.read_regs(5, Area::Holding, 50, 1), Err(ModbusError::Exception(EX_ILLEGAL_ADDRESS)));
    }

    #[test]
    fn rtu_silent_unit_times_out_and_bad_crc_is_rejected() {
        let mut s = rtu_session(false);
        assert_eq!(s.read_regs(9, Area::Holding, 0, 1), Err(ModbusError::Timeout));
        let mut bad = rtu_session(true);
        assert!(matches!(bad.read_regs(5, Area::Holding, 0, 1), Err(ModbusError::Protocol(_))));
    }
}
