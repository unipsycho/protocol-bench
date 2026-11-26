//! CAN signals (DBC-style bit fields) and a SocketCAN raw socket (Linux).
//!
//! A signal is `start` bit + `len` bits in the 8-byte payload, Intel (little-endian, start =
//! LSB) or Motorola (big-endian, DBC numbering, start = MSB), unsigned / signed / float32, with
//! `value = raw × scale + offset`.
//!
//! Frame ids are written as `0x123` (11-bit), `0x18FEF100` (29-bit, any id above 0x7FF is
//! extended), or `pgn:65262` — a J1939 parameter group matched on any priority / source address.

/// How a bind selects frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdMatch {
    /// Exact id; `extended` for 29-bit.
    Exact { id: u32, extended: bool },
    /// J1939 PGN (bits 8..25 of a 29-bit id; PDU1 PGNs ignore the destination byte).
    Pgn(u32),
}

impl IdMatch {
    pub fn parse(text: &str) -> Option<IdMatch> {
        let t = text.trim().to_ascii_lowercase();
        if let Some(p) = t.strip_prefix("pgn:") {
            let p = p.trim();
            let v = if let Some(h) = p.strip_prefix("0x") { u32::from_str_radix(h, 16).ok()? } else { p.parse().ok()? };
            return (v <= 0x3FFFF).then_some(IdMatch::Pgn(v));
        }
        let (v, hex_ext) = if let Some(h) = t.strip_prefix("0x") {
            (u32::from_str_radix(h.trim_end_matches('x'), 16).ok()?, h.len() > 3)
        } else {
            (t.trim_end_matches('x').parse().ok()?, false)
        };
        if v > 0x1FFF_FFFF {
            return None;
        }
        Some(IdMatch::Exact { id: v, extended: v > 0x7FF || hex_ext && t.len() > 5 || t.ends_with('x') })
    }

    pub fn matches(&self, id: u32, extended: bool) -> bool {
        match *self {
            IdMatch::Exact { id: want, extended: e } => want == id && e == extended,
            IdMatch::Pgn(pgn) => extended && pgn_of(id) == pgn,
        }
    }
}

/// J1939 PGN of a 29-bit id (PDU1 formats drop the destination address byte).
pub fn pgn_of(id: u32) -> u32 {
    let pf = (id >> 16) & 0xff;
    let pgn = (id >> 8) & 0x3FFFF;
    if pf < 240 {
        pgn & 0x3FF00
    } else {
        pgn
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    Intel,
    Motorola,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigType {
    Unsigned,
    Signed,
    Float,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Signal {
    pub start: u32,
    pub len: u32,
    pub order: Order,
    pub kind: SigType,
    pub scale: f64,
    pub offset: f64,
}

impl Signal {
    pub fn check(&self) -> Result<(), String> {
        if self.len == 0 || self.len > 64 {
            return Err(format!("length {} must be 1–64 bits", self.len));
        }
        if self.kind == SigType::Float && self.len != 32 {
            return Err("float signals are 32 bits".into());
        }
        if self.start > 63 {
            return Err(format!("start bit {} past 63", self.start));
        }
        if self.order == Order::Intel && self.start + self.len > 64 {
            return Err("signal runs past the 8-byte payload".into());
        }
        Ok(())
    }

    /// Bit positions in transmission order of significance (MSB first).
    fn positions(&self) -> Option<Vec<u32>> {
        let mut v = Vec::with_capacity(self.len as usize);
        match self.order {
            Order::Intel => {
                for i in (0..self.len).rev() {
                    v.push(self.start + i);
                }
            }
            Order::Motorola => {
                let mut pos = self.start as i64;
                for _ in 0..self.len {
                    if !(0..64).contains(&pos) {
                        return None;
                    }
                    v.push(pos as u32);
                    pos = if pos % 8 == 0 { pos + 15 } else { pos - 1 };
                }
            }
        }
        Some(v)
    }

    pub fn raw(&self, data: &[u8; 8]) -> Option<u64> {
        let mut r = 0u64;
        for p in self.positions()? {
            r = (r << 1) | ((data[(p / 8) as usize] >> (p % 8)) & 1) as u64;
        }
        Some(r)
    }

    /// Decode the engineering value.
    pub fn decode(&self, data: &[u8; 8]) -> Option<f64> {
        let raw = self.raw(data)?;
        let x = match self.kind {
            SigType::Unsigned => raw as f64,
            SigType::Signed => {
                let shift = 64 - self.len;
                ((raw << shift) as i64 >> shift) as f64
            }
            SigType::Float => f32::from_bits(raw as u32) as f64,
        };
        Some(x * self.scale + self.offset)
    }

    /// Write an engineering value into `data` (other bits untouched).
    pub fn encode(&self, data: &mut [u8; 8], value: f64) -> Option<()> {
        let x = (value - self.offset) / self.scale;
        let raw: u64 = match self.kind {
            SigType::Float => (x as f32).to_bits() as u64,
            SigType::Unsigned => {
                let max = if self.len == 64 { u64::MAX as f64 } else { ((1u64 << self.len) - 1) as f64 };
                x.round().clamp(0.0, max) as u64
            }
            SigType::Signed => {
                let half = 1i128 << (self.len - 1);
                let v = (x.round() as i128).clamp(-half, half - 1);
                (v as i64 as u64) & if self.len == 64 { u64::MAX } else { (1u64 << self.len) - 1 }
            }
        };
        let pos = self.positions()?;
        let n = pos.len();
        for (i, p) in pos.into_iter().enumerate() {
            let bit = (raw >> (n - 1 - i)) & 1;
            let byte = &mut data[(p / 8) as usize];
            if bit == 1 {
                *byte |= 1 << (p % 8);
            } else {
                *byte &= !(1 << (p % 8));
            }
        }
        Some(())
    }
}

/// One classic CAN frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    pub id: u32,
    pub extended: bool,
    pub dlc: u8,
    pub data: [u8; 8],
}

impl Frame {
    /// `18FEF100#0011223344556677`-style text (candump short form).
    pub fn display(&self) -> String {
        let id = if self.extended { format!("{:08X}", self.id) } else { format!("{:03X}", self.id) };
        let d: String = self.data[..self.dlc.min(8) as usize].iter().map(|b| format!("{b:02X}")).collect();
        format!("{id}#{d}")
    }
}

#[cfg(target_os = "linux")]
pub use socket::CanSocket;

#[cfg(target_os = "linux")]
mod socket {
    use super::Frame;
    use std::ffi::CString;
    use std::io;
    use std::time::Duration;

    const AF_CAN: i32 = 29;
    const CAN_RAW: i32 = 1;
    const EFF_FLAG: u32 = 0x8000_0000;
    const RTR_FLAG: u32 = 0x4000_0000;
    const ERR_FLAG: u32 = 0x2000_0000;

    #[repr(C)]
    struct SockAddrCan {
        family: u16,
        ifindex: i32,
        addr: [u8; 16],
    }

    #[repr(C)]
    struct RawFrame {
        id: u32,
        dlc: u8,
        pad: [u8; 3],
        data: [u8; 8],
    }

    /// A bound SocketCAN raw socket (`can0`, `vcan0`).
    pub struct CanSocket {
        fd: i32,
    }

    impl CanSocket {
        pub fn open(iface: &str) -> io::Result<CanSocket> {
            let name = CString::new(iface).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "bad interface name"))?;
            // SAFETY: plain libc calls with valid pointers / sizes; fd closed in Drop.
            unsafe {
                let idx = libc::if_nametoindex(name.as_ptr());
                if idx == 0 {
                    return Err(io::Error::new(io::ErrorKind::NotFound, format!("no CAN interface \"{iface}\"")));
                }
                let fd = libc::socket(AF_CAN, libc::SOCK_RAW, CAN_RAW);
                if fd < 0 {
                    return Err(io::Error::last_os_error());
                }
                let addr = SockAddrCan { family: AF_CAN as u16, ifindex: idx as i32, addr: [0; 16] };
                if libc::bind(fd, &addr as *const SockAddrCan as *const libc::sockaddr, std::mem::size_of::<SockAddrCan>() as u32) < 0 {
                    let e = io::Error::last_os_error();
                    libc::close(fd);
                    return Err(e);
                }
                let s = CanSocket { fd };
                s.set_timeout(Duration::from_millis(50))?;
                Ok(s)
            }
        }

        pub fn set_timeout(&self, d: Duration) -> io::Result<()> {
            let tv = libc::timeval { tv_sec: d.as_secs() as libc::time_t, tv_usec: d.subsec_micros() as libc::suseconds_t };
            // SAFETY: valid fd and timeval.
            let r = unsafe { libc::setsockopt(self.fd, libc::SOL_SOCKET, libc::SO_RCVTIMEO, &tv as *const _ as *const libc::c_void, std::mem::size_of::<libc::timeval>() as u32) };
            if r < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        }

        /// Next data frame, or `None` on timeout. Error / RTR frames are skipped.
        pub fn read(&self) -> io::Result<Option<Frame>> {
            let mut f = RawFrame { id: 0, dlc: 0, pad: [0; 3], data: [0; 8] };
            // SAFETY: reading at most size_of RawFrame into it.
            let n = unsafe { libc::read(self.fd, &mut f as *mut RawFrame as *mut libc::c_void, std::mem::size_of::<RawFrame>()) };
            if n < 0 {
                let e = io::Error::last_os_error();
                return match e.kind() {
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted => Ok(None),
                    _ => Err(e),
                };
            }
            if f.id & (RTR_FLAG | ERR_FLAG) != 0 {
                return Ok(None);
            }
            let extended = f.id & EFF_FLAG != 0;
            Ok(Some(Frame { id: f.id & if extended { 0x1FFF_FFFF } else { 0x7FF }, extended, dlc: f.dlc.min(8), data: f.data }))
        }

        pub fn write(&self, fr: &Frame) -> io::Result<()> {
            let f = RawFrame { id: fr.id | if fr.extended { EFF_FLAG } else { 0 }, dlc: fr.dlc.min(8), pad: [0; 3], data: fr.data };
            // SAFETY: writing a fully initialised RawFrame.
            let n = unsafe { libc::write(self.fd, &f as *const RawFrame as *const libc::c_void, std::mem::size_of::<RawFrame>()) };
            if n < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        }
    }

    impl Drop for CanSocket {
        fn drop(&mut self) {
            // SAFETY: fd owned by this socket.
            unsafe { libc::close(self.fd) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sig(start: u32, len: u32, order: Order, kind: SigType) -> Signal {
        Signal { start, len, order, kind, scale: 1.0, offset: 0.0 }
    }

    #[test]
    fn intel_and_motorola_round_trip() {
        // J1939 EEC1 engine speed: bytes 4-5 little-endian, 0.125 rpm/bit.
        let mut s = sig(24, 16, Order::Intel, SigType::Unsigned);
        s.scale = 0.125;
        let data = [0xFF, 0xFF, 0xFF, 0x68, 0x13, 0xFF, 0xFF, 0xFF];
        assert_eq!(s.decode(&data), Some(0x1368 as f64 * 0.125));
        let mut d = [0u8; 8];
        s.encode(&mut d, 1500.0).unwrap();
        assert_eq!(s.decode(&d), Some(1500.0));
        assert_eq!(d[3..5], [0xE0, 0x2E]);
        // Motorola: 12 bits starting at bit 7 (MSB of byte 0) → byte0 + high nibble of byte1.
        let m = sig(7, 12, Order::Motorola, SigType::Unsigned);
        let data = [0xAB, 0xC0, 0, 0, 0, 0, 0, 0];
        assert_eq!(m.raw(&data), Some(0xABC));
        let mut d = [0u8; 8];
        m.encode(&mut d, 0x123 as f64).unwrap();
        assert_eq!(d[..2], [0x12, 0x30]);
    }

    #[test]
    fn signed_float_and_clamping() {
        let s = sig(0, 8, Order::Intel, SigType::Signed);
        assert_eq!(s.decode(&[0xFE, 0, 0, 0, 0, 0, 0, 0]), Some(-2.0));
        let mut d = [0u8; 8];
        s.encode(&mut d, -300.0).unwrap();
        assert_eq!(d[0], 0x80, "clamped to -128");
        let f = sig(32, 32, Order::Intel, SigType::Float);
        let mut d = [0u8; 8];
        f.encode(&mut d, 21.5).unwrap();
        assert_eq!(f.decode(&d), Some(21.5));
        let u = sig(0, 4, Order::Intel, SigType::Unsigned);
        let mut d = [0xF0u8, 0, 0, 0, 0, 0, 0, 0];
        u.encode(&mut d, 99.0).unwrap();
        assert_eq!(d[0], 0xFF, "clamped to 15, upper nibble kept");
        assert!(sig(60, 8, Order::Intel, SigType::Unsigned).check().is_err());
        assert!(sig(0, 16, Order::Intel, SigType::Float).check().is_err());
        assert!(sig(0, 0, Order::Intel, SigType::Unsigned).check().is_err());
    }

    #[test]
    fn ids_and_pgns() {
        assert_eq!(IdMatch::parse("0x123"), Some(IdMatch::Exact { id: 0x123, extended: false }));
        assert_eq!(IdMatch::parse("0x18FEF100"), Some(IdMatch::Exact { id: 0x18FEF100, extended: true }));
        assert_eq!(IdMatch::parse("291"), Some(IdMatch::Exact { id: 291, extended: false }));
        assert_eq!(IdMatch::parse("pgn:61444"), Some(IdMatch::Pgn(61444)));
        assert_eq!(IdMatch::parse("nope"), None);
        // EEC1 (PGN 61444 = 0xF004) from source 0x00 at priority 3.
        let id = 0x0CF00400;
        assert!(IdMatch::Pgn(61444).matches(id, true));
        assert!(!IdMatch::Pgn(61444).matches(id, false));
        // PDU1 (TSC1, PGN 0): destination byte ignored.
        assert_eq!(pgn_of(0x0C000003), 0);
        let f = Frame { id: 0x123, extended: false, dlc: 2, data: [0xA, 0xB, 0, 0, 0, 0, 0, 0] };
        assert_eq!(f.display(), "123#0A0B");
    }
}
