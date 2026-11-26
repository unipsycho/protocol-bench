//! DNP3 (IEEE 1815) over TCP — the subset a SCADA master and a small outstation need.
//!
//! * Link layer: `05 64` frames, DNP CRC-16 per header and per 16-byte data block.
//! * Transport: one header byte (FIR / FIN / sequence), segments reassembled.
//! * Application: READ class 0 / 1 / 2 / 3, WRITE IIN (clear restart), DIRECT OPERATE with
//!   CROB (g12v1) and analog outputs (g41v1 / g41v3), CONFIRM; static and event objects for
//!   binary inputs (g1 / g2), binary output status (g10 / g11), counters (g20 / g22), analog
//!   inputs (g30 / g32) and analog output status (g40 / g42).
//! * [`Outstation`]: an in-memory point database that answers a master — used by the tests
//!   and by the `pb-sim` demo devices.

use std::collections::BTreeMap;
use std::io::{self, Read, Write};

/// Point families, as written in a bind's `kind` field.
pub const KINDS: [(&str, &str); 5] = [
    ("binary", "Binary input (g1 / g2)"),
    ("binary_output", "Binary output status (g10 / g11)"),
    ("counter", "Counter (g20 / g22)"),
    ("analog", "Analog input (g30 / g32)"),
    ("analog_output", "Analog output status (g40 / g42)"),
];

/// One decoded point value.
#[derive(Debug, Clone, PartialEq)]
pub enum PValue {
    Bool(bool),
    Int(i64),
    Float(f64),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PointUpdate {
    pub kind: &'static str,
    pub index: u16,
    pub value: PValue,
    /// ONLINE flag (true when the object carried no flags).
    pub online: bool,
}

// ---------------------------------------------------------------------------
// CRC + link layer
// ---------------------------------------------------------------------------

/// DNP3 CRC-16 (poly 0x3D65 reflected, init 0, complemented).
pub fn crc(data: &[u8]) -> u16 {
    let mut c: u16 = 0;
    for &b in data {
        c ^= b as u16;
        for _ in 0..8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0xA6BC } else { c >> 1 };
        }
    }
    !c
}

/// Max user data in one link frame.
pub const LINK_MAX: usize = 250;

/// Encode one link frame carrying `data` (≤ 250 bytes) with unconfirmed user data.
pub fn link_frame(from_master: bool, dest: u16, src: u16, data: &[u8]) -> Vec<u8> {
    assert!(data.len() <= LINK_MAX);
    let ctrl: u8 = if from_master { 0xC4 } else { 0x44 };
    let mut h = vec![0x05, 0x64, (5 + data.len()) as u8, ctrl];
    h.extend_from_slice(&dest.to_le_bytes());
    h.extend_from_slice(&src.to_le_bytes());
    let c = crc(&h);
    h.extend_from_slice(&c.to_le_bytes());
    for chunk in data.chunks(16) {
        h.extend_from_slice(chunk);
        h.extend_from_slice(&crc(chunk).to_le_bytes());
    }
    h
}

/// A received link frame.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkFrame {
    pub ctrl: u8,
    pub dest: u16,
    pub src: u16,
    pub data: Vec<u8>,
}

fn bad(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

/// Read one link frame (resynchronises on `05 64`).
pub fn read_link_frame(r: &mut impl Read) -> io::Result<LinkFrame> {
    let mut b = [0u8; 1];
    // Hunt for 0x05 0x64.
    let mut prev = 0u8;
    loop {
        r.read_exact(&mut b)?;
        if prev == 0x05 && b[0] == 0x64 {
            break;
        }
        prev = b[0];
    }
    let mut rest = [0u8; 8];
    r.read_exact(&mut rest)?;
    let mut h = vec![0x05, 0x64];
    h.extend_from_slice(&rest[..6]);
    if crc(&h) != u16::from_le_bytes([rest[6], rest[7]]) {
        return Err(bad("link header CRC"));
    }
    let len = rest[0] as usize;
    if len < 5 {
        return Err(bad("link length"));
    }
    let n = len - 5;
    let mut data = Vec::with_capacity(n);
    let mut left = n;
    while left > 0 {
        let k = left.min(16);
        let mut blk = vec![0u8; k + 2];
        r.read_exact(&mut blk)?;
        if crc(&blk[..k]) != u16::from_le_bytes([blk[k], blk[k + 1]]) {
            return Err(bad("link data CRC"));
        }
        data.extend_from_slice(&blk[..k]);
        left -= k;
    }
    Ok(LinkFrame { ctrl: rest[1], dest: u16::from_le_bytes([rest[2], rest[3]]), src: u16::from_le_bytes([rest[4], rest[5]]), data })
}

// ---------------------------------------------------------------------------
// Transport
// ---------------------------------------------------------------------------

/// Split an application fragment into link frames (transport header per segment).
pub fn encode_fragment(from_master: bool, dest: u16, src: u16, apdu: &[u8], seq: &mut u8) -> Vec<u8> {
    let mut out = Vec::new();
    let chunks: Vec<&[u8]> = if apdu.is_empty() { vec![&[][..]] } else { apdu.chunks(LINK_MAX - 1).collect() };
    let n = chunks.len();
    for (i, c) in chunks.into_iter().enumerate() {
        let mut th = *seq & 0x3f;
        if i == 0 {
            th |= 0x40;
        }
        if i == n - 1 {
            th |= 0x80;
        }
        *seq = (*seq + 1) & 0x3f;
        let mut d = vec![th];
        d.extend_from_slice(c);
        out.extend(link_frame(from_master, dest, src, &d));
    }
    out
}

/// Read link frames until a complete transport segment series (FIR … FIN) gives one fragment.
/// Frames for another address are skipped.
pub fn read_fragment(r: &mut impl Read, my_addr: u16) -> io::Result<(u16, Vec<u8>)> {
    let mut buf: Option<Vec<u8>> = None;
    loop {
        let f = read_link_frame(r)?;
        if f.dest != my_addr || f.data.is_empty() || f.ctrl & 0x0f != 4 && f.ctrl & 0x0f != 3 {
            continue;
        }
        let th = f.data[0];
        if th & 0x40 != 0 {
            buf = Some(Vec::new());
        }
        if let Some(b) = buf.as_mut() {
            b.extend_from_slice(&f.data[1..]);
            if b.len() > 64 * 1024 {
                return Err(bad("fragment too large"));
            }
        }
        if th & 0x80 != 0 {
            if let Some(b) = buf.take() {
                return Ok((f.src, b));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Application layer
// ---------------------------------------------------------------------------

pub mod func {
    pub const CONFIRM: u8 = 0;
    pub const READ: u8 = 1;
    pub const WRITE: u8 = 2;
    pub const DIRECT_OPERATE: u8 = 5;
    pub const RESPONSE: u8 = 0x81;
    pub const UNSOLICITED: u8 = 0x82;
}

/// IIN1 bits.
pub const IIN1_RESTART: u8 = 0x80;
pub const IIN1_CLASS_EVENTS: u8 = 0x0e;
/// IIN2: function not supported / object unknown.
pub const IIN2_NO_FUNC: u8 = 0x01;
pub const IIN2_OBJ_UNKNOWN: u8 = 0x02;

pub const CTRL_FIR: u8 = 0x80;
pub const CTRL_FIN: u8 = 0x40;
pub const CTRL_CON: u8 = 0x20;
pub const CTRL_UNS: u8 = 0x10;

/// READ class 0 (+ class 1 / 2 / 3 events when `events`).
pub fn read_classes(seq: u8, class0: bool, events: bool) -> Vec<u8> {
    let mut a = vec![CTRL_FIR | CTRL_FIN | (seq & 0x0f), func::READ];
    if events {
        for v in [2u8, 3, 4] {
            a.extend_from_slice(&[60, v, 0x06]);
        }
    }
    if class0 {
        a.extend_from_slice(&[60, 1, 0x06]);
    }
    a
}

/// WRITE g80v1 index 7 = 0 (clear DEVICE RESTART).
pub fn clear_restart(seq: u8) -> Vec<u8> {
    vec![CTRL_FIR | CTRL_FIN | (seq & 0x0f), func::WRITE, 80, 1, 0x00, 7, 7, 0x00]
}

pub fn confirm(seq: u8, unsolicited: bool) -> Vec<u8> {
    vec![CTRL_FIR | CTRL_FIN | if unsolicited { CTRL_UNS } else { 0 } | (seq & 0x0f), func::CONFIRM]
}

/// DIRECT OPERATE a CROB: LATCH ON (`0x03`) / LATCH OFF (`0x04`).
pub fn operate_binary(seq: u8, index: u16, on: bool) -> Vec<u8> {
    let mut a = vec![CTRL_FIR | CTRL_FIN | (seq & 0x0f), func::DIRECT_OPERATE, 12, 1, 0x28, 1, 0];
    a.extend_from_slice(&index.to_le_bytes());
    a.push(if on { 0x03 } else { 0x04 });
    a.push(1);
    a.extend_from_slice(&1000u32.to_le_bytes());
    a.extend_from_slice(&0u32.to_le_bytes());
    a.push(0);
    a
}

/// DIRECT OPERATE an analog output: g41v3 (f32) when `float`, else g41v1 (i32).
pub fn operate_analog(seq: u8, index: u16, value: f64, float: bool) -> Vec<u8> {
    let mut a = vec![CTRL_FIR | CTRL_FIN | (seq & 0x0f), func::DIRECT_OPERATE, 41, if float { 3 } else { 1 }, 0x28, 1, 0];
    a.extend_from_slice(&index.to_le_bytes());
    if float {
        a.extend_from_slice(&(value as f32).to_le_bytes());
    } else {
        a.extend_from_slice(&(value.round() as i32).to_le_bytes());
    }
    a.push(0);
    a
}

/// Parsed response header + points.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Response {
    pub ctrl: u8,
    pub func: u8,
    pub iin1: u8,
    pub iin2: u8,
    pub points: Vec<PointUpdate>,
    /// CROB / analog output status codes echoed by DIRECT OPERATE (non-zero = refused).
    pub control_status: Vec<u8>,
}

impl Response {
    pub fn seq(&self) -> u8 {
        self.ctrl & 0x0f
    }
    pub fn needs_confirm(&self) -> bool {
        self.ctrl & CTRL_CON != 0
    }
    pub fn unsolicited(&self) -> bool {
        self.ctrl & CTRL_UNS != 0
    }
}

struct Cur<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Cur<'a> {
    fn u8(&mut self) -> Result<u8, String> {
        let v = *self.b.get(self.i).ok_or("truncated")?;
        self.i += 1;
        Ok(v)
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let e = self.i.checked_add(n).filter(|e| *e <= self.b.len()).ok_or("truncated object")?;
        let s = &self.b[self.i..e];
        self.i = e;
        Ok(s)
    }
    fn u16(&mut self) -> Result<u16, String> {
        let s = self.take(2)?;
        Ok(u16::from_le_bytes([s[0], s[1]]))
    }
    fn done(&self) -> bool {
        self.i >= self.b.len()
    }
}

/// Size in bytes of one object (excluding any index prefix). `None` = packed bits / unknown.
fn obj_size(g: u8, v: u8) -> Option<usize> {
    Some(match (g, v) {
        (1, 2) | (10, 2) | (2, 1) | (11, 1) => 1,
        (2, 2) | (11, 2) => 7,
        (2, 3) => 3,
        (20, 1) | (22, 1) | (30, 1) | (32, 1) | (40, 1) | (42, 1) | (30, 5) | (32, 5) | (40, 3) | (42, 5) => 5,
        (20, 2) | (22, 2) | (30, 2) | (32, 2) | (40, 2) | (42, 2) => 3,
        (20, 5) | (30, 3) => 4,
        (20, 6) | (30, 4) => 2,
        (21, 1) => 5,
        (21, 2) => 3,
        (21, 5) | (22, 5) | (32, 3) | (42, 3) | (32, 7) | (42, 7) => 11,
        (21, 6) | (22, 6) | (32, 4) | (42, 4) => 9,
        (21, 9) => 4,
        (21, 10) => 2,
        (30, 6) | (32, 6) | (40, 4) | (42, 6) => 9,
        (32, 8) | (42, 8) => 15,
        (12, 1) => 11,
        (41, 1) | (41, 3) => 5,
        (41, 2) => 3,
        (41, 4) => 9,
        (50, 1) | (51, 1) | (51, 2) => 6,
        (52, 1) | (52, 2) => 2,
        _ => return None,
    })
}

fn kind_of(g: u8) -> Option<&'static str> {
    match g {
        1 | 2 => Some("binary"),
        10 | 11 => Some("binary_output"),
        20 | 22 => Some("counter"),
        30 | 32 => Some("analog"),
        40 | 42 => Some("analog_output"),
        _ => None,
    }
}

/// Decode one object's value (`flag` byte first when the variation has one).
fn decode_obj(g: u8, v: u8, d: &[u8]) -> Option<(PValue, bool)> {
    let flag_online = |f: u8| f & 0x01 != 0;
    let i32_at = |o: usize| i32::from_le_bytes(d[o..o + 4].try_into().unwrap()) as i64;
    let i16_at = |o: usize| i16::from_le_bytes(d[o..o + 2].try_into().unwrap()) as i64;
    let u32_at = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap()) as i64;
    let u16_at = |o: usize| u16::from_le_bytes(d[o..o + 2].try_into().unwrap()) as i64;
    let f32_at = |o: usize| {
        let x = f32::from_le_bytes(d[o..o + 4].try_into().unwrap());
        format!("{x}").parse::<f64>().unwrap_or(x as f64)
    };
    let f64_at = |o: usize| f64::from_le_bytes(d[o..o + 8].try_into().unwrap());
    Some(match (g, v) {
        (1, 2) | (10, 2) | (2, _) | (11, _) => (PValue::Bool(d[0] & 0x80 != 0), flag_online(d[0])),
        (20, 1) | (22, 1) | (22, 5) => (PValue::Int(u32_at(1)), flag_online(d[0])),
        (20, 2) | (22, 2) | (22, 6) => (PValue::Int(u16_at(1)), flag_online(d[0])),
        (20, 5) => (PValue::Int(u32_at(0)), true),
        (20, 6) => (PValue::Int(u16_at(0)), true),
        (30, 1) | (32, 1) | (32, 3) | (40, 1) | (42, 1) | (42, 3) => (PValue::Int(i32_at(1)), flag_online(d[0])),
        (30, 2) | (32, 2) | (32, 4) | (40, 2) | (42, 2) | (42, 4) => (PValue::Int(i16_at(1)), flag_online(d[0])),
        (30, 3) => (PValue::Int(i32_at(0)), true),
        (30, 4) => (PValue::Int(i16_at(0)), true),
        (30, 5) | (32, 5) | (32, 7) | (40, 3) | (42, 5) | (42, 7) => (PValue::Float(f32_at(1)), flag_online(d[0])),
        (30, 6) | (32, 6) | (32, 8) | (40, 4) | (42, 6) | (42, 8) => (PValue::Float(f64_at(1)), flag_online(d[0])),
        _ => return None,
    })
}

/// Parse a response / unsolicited fragment.
pub fn parse_response(apdu: &[u8]) -> Result<Response, String> {
    let mut c = Cur { b: apdu, i: 0 };
    let mut r = Response { ctrl: c.u8()?, func: c.u8()?, ..Default::default() };
    if r.func != func::RESPONSE && r.func != func::UNSOLICITED {
        return Err(format!("not a response (function {:#04x})", r.func));
    }
    r.iin1 = c.u8()?;
    r.iin2 = c.u8()?;
    while !c.done() {
        let g = c.u8()?;
        let v = c.u8()?;
        let q = c.u8()?;
        // Range → list of (index or None for "next in start/stop").
        let (indices, prefix): (Vec<u16>, usize) = match q {
            0x00 => {
                let (a, b) = (c.u8()? as u16, c.u8()? as u16);
                ((a..=b.max(a)).collect(), 0)
            }
            0x01 => {
                let (a, b) = (c.u16()?, c.u16()?);
                if b < a {
                    return Err("bad range".into());
                }
                ((a..=b).collect(), 0)
            }
            0x07 => ((0..c.u8()? as u16).collect(), 0),
            0x08 => ((0..c.u16()?).collect(), 0),
            0x17 => ((0..c.u8()? as u16).collect(), 1),
            0x28 => ((0..c.u16()?).collect(), 2),
            0x06 => (Vec::new(), 0),
            other => return Err(format!("unsupported qualifier {other:#04x}")),
        };
        // Packed bits (g1v1, g10v1, g80v1).
        if matches!((g, v), (1, 1) | (10, 1) | (80, 1)) && prefix == 0 {
            let n = indices.len();
            let bytes = c.take(n.div_ceil(8))?;
            if let Some(kind) = kind_of(g) {
                for (k, idx) in indices.iter().enumerate() {
                    let bit = bytes[k / 8] >> (k % 8) & 1 == 1;
                    r.points.push(PointUpdate { kind, index: *idx, value: PValue::Bool(bit), online: true });
                }
            }
            continue;
        }
        let Some(size) = obj_size(g, v) else {
            return Err(format!("unsupported object g{g}v{v}"));
        };
        for idx in indices {
            let index = match prefix {
                1 => c.u8()? as u16,
                2 => c.u16()?,
                _ => idx,
            };
            let d = c.take(size)?;
            match g {
                12 => r.control_status.push(d[10]),
                41 => r.control_status.push(d[size - 1]),
                _ => {
                    if let (Some(kind), Some((value, online))) = (kind_of(g), decode_obj(g, v, d)) {
                        r.points.push(PointUpdate { kind, index, value, online });
                    }
                }
            }
        }
    }
    Ok(r)
}

// ---------------------------------------------------------------------------
// Outstation (demo / test device)
// ---------------------------------------------------------------------------

/// In-memory outstation database. Static values are reported on class 0 reads; controls from
/// the master land in `binary_outputs` / `analog_outputs`.
#[derive(Debug, Clone, Default)]
pub struct Outstation {
    pub address: u16,
    pub binaries: BTreeMap<u16, bool>,
    pub binary_outputs: BTreeMap<u16, bool>,
    pub counters: BTreeMap<u16, u32>,
    pub analogs: BTreeMap<u16, f64>,
    pub analog_outputs: BTreeMap<u16, f64>,
    /// Indices reported with the ONLINE flag cleared.
    pub offline_analogs: Vec<u16>,
    pub restart: bool,
    tx_seq: u8,
}

impl Outstation {
    pub fn new(address: u16) -> Self {
        Self { address, restart: true, ..Default::default() }
    }

    /// Contiguous runs of a map's keys (start/stop ranges).
    fn runs<T>(m: &BTreeMap<u16, T>) -> Vec<(u16, u16)> {
        let mut out: Vec<(u16, u16)> = Vec::new();
        for &k in m.keys() {
            match out.last_mut() {
                Some((_, e)) if *e + 1 == k => *e = k,
                _ => out.push((k, k)),
            }
        }
        out
    }

    fn class0(&self, o: &mut Vec<u8>) {
        let hdr = |o: &mut Vec<u8>, g: u8, v: u8, a: u16, b: u16| {
            o.extend_from_slice(&[g, v, 0x01]);
            o.extend_from_slice(&a.to_le_bytes());
            o.extend_from_slice(&b.to_le_bytes());
        };
        for (a, b) in Self::runs(&self.binaries) {
            hdr(o, 1, 2, a, b);
            for i in a..=b {
                o.push(0x01 | if self.binaries[&i] { 0x80 } else { 0 });
            }
        }
        for (a, b) in Self::runs(&self.binary_outputs) {
            hdr(o, 10, 2, a, b);
            for i in a..=b {
                o.push(0x01 | if self.binary_outputs[&i] { 0x80 } else { 0 });
            }
        }
        for (a, b) in Self::runs(&self.counters) {
            hdr(o, 20, 1, a, b);
            for i in a..=b {
                o.push(0x01);
                o.extend_from_slice(&self.counters[&i].to_le_bytes());
            }
        }
        for (a, b) in Self::runs(&self.analogs) {
            hdr(o, 30, 5, a, b);
            for i in a..=b {
                o.push(if self.offline_analogs.contains(&i) { 0x00 } else { 0x01 });
                o.extend_from_slice(&(self.analogs[&i] as f32).to_le_bytes());
            }
        }
        for (a, b) in Self::runs(&self.analog_outputs) {
            hdr(o, 40, 3, a, b);
            for i in a..=b {
                o.push(0x01);
                o.extend_from_slice(&(self.analog_outputs[&i] as f32).to_le_bytes());
            }
        }
    }

    /// Answer one request fragment. `None` for a CONFIRM (no reply).
    pub fn handle(&mut self, req: &[u8]) -> Option<Vec<u8>> {
        if req.len() < 2 {
            return None;
        }
        let seq = req[0] & 0x0f;
        let f = req[1];
        if f == func::CONFIRM {
            return None;
        }
        let mut iin2 = 0u8;
        let mut body = Vec::new();
        match f {
            func::READ => {
                let mut c = Cur { b: &req[2..], i: 0 };
                while !c.done() {
                    let (Ok(g), Ok(v), Ok(q)) = (c.u8(), c.u8(), c.u8()) else { break };
                    if g == 60 && q == 0x06 {
                        if v == 1 {
                            self.class0(&mut body);
                        }
                    } else {
                        iin2 |= IIN2_OBJ_UNKNOWN;
                        break;
                    }
                }
            }
            func::WRITE => {
                if req.len() >= 8 && req[2] == 80 && req[3] == 1 && req[5] == 7 && req[7] & 0x80 == 0 {
                    self.restart = false;
                }
            }
            func::DIRECT_OPERATE => {
                // Echo the request objects with status codes (0 = success).
                let mut c = Cur { b: &req[2..], i: 0 };
                while !c.done() {
                    let (Ok(g), Ok(v), Ok(q)) = (c.u8(), c.u8(), c.u8()) else { break };
                    if q != 0x28 {
                        iin2 |= IIN2_OBJ_UNKNOWN;
                        break;
                    }
                    let Ok(n) = c.u16() else { break };
                    body.extend_from_slice(&[g, v, q]);
                    body.extend_from_slice(&n.to_le_bytes());
                    let Some(size) = obj_size(g, v) else {
                        iin2 |= IIN2_OBJ_UNKNOWN;
                        break;
                    };
                    for _ in 0..n {
                        let (Ok(idx), Ok(d)) = (c.u16(), c.take(size)) else { break };
                        let mut d = d.to_vec();
                        match (g, v) {
                            (12, 1) => {
                                let on = matches!(d[0] & 0x0f, 0x01 | 0x03);
                                self.binary_outputs.insert(idx, on);
                                d[10] = 0;
                            }
                            (41, 1) => {
                                self.analog_outputs.insert(idx, i32::from_le_bytes(d[0..4].try_into().unwrap()) as f64);
                                d[4] = 0;
                            }
                            (41, 3) => {
                                self.analog_outputs.insert(idx, f32::from_le_bytes(d[0..4].try_into().unwrap()) as f64);
                                d[4] = 0;
                            }
                            _ => {
                                let last = d.len() - 1;
                                d[last] = 4; // not supported
                            }
                        }
                        body.extend_from_slice(&idx.to_le_bytes());
                        body.extend_from_slice(&d);
                    }
                }
            }
            _ => iin2 |= IIN2_NO_FUNC,
        }
        let iin1 = if self.restart { IIN1_RESTART } else { 0 };
        let mut out = vec![CTRL_FIR | CTRL_FIN | seq, func::RESPONSE, iin1, iin2];
        out.extend(body);
        Some(out)
    }

    /// Serve one master over a stream until it disconnects.
    pub fn serve(&mut self, s: &mut (impl Read + Write)) -> io::Result<()> {
        loop {
            let (master, apdu) = read_fragment(s, self.address)?;
            if let Some(resp) = self.handle(&apdu) {
                let bytes = encode_fragment(false, master, self.address, &resp, &mut self.tx_seq);
                s.write_all(&bytes)?;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_matches_the_spec_example() {
        // Reset-link request from the DNP3 primer: 05 64 05 C0 01 00 00 04 | E9 21
        assert_eq!(crc(&[0x05, 0x64, 0x05, 0xC0, 0x01, 0x00, 0x00, 0x04]).to_le_bytes(), [0xE9, 0x21]);
    }

    #[test]
    fn frames_round_trip_and_long_fragments_split() {
        let apdu: Vec<u8> = (0..600u32).map(|i| i as u8).collect();
        let mut seq = 0;
        let bytes = encode_fragment(true, 10, 1, &apdu, &mut seq);
        assert_eq!(seq, 3, "600 bytes need three segments");
        let mut r = std::io::Cursor::new(bytes);
        let (src, back) = read_fragment(&mut r, 10).unwrap();
        assert_eq!(src, 1);
        assert_eq!(back, apdu);
        // A corrupted data byte is caught by the block CRC.
        let mut bytes = link_frame(true, 10, 1, &[0xC0, 1, 2, 3]);
        bytes[12] ^= 0xff;
        assert!(read_link_frame(&mut std::io::Cursor::new(bytes)).is_err());
    }

    #[test]
    fn outstation_answers_integrity_and_controls() {
        let mut o = Outstation::new(10);
        o.binaries.insert(0, true);
        o.binaries.insert(1, false);
        o.counters.insert(0, 123_456);
        o.analogs.insert(0, 3412.5);
        o.analogs.insert(1, -7.25);
        o.analogs.insert(5, 1.0);
        o.offline_analogs.push(5);
        let resp = parse_response(&o.handle(&read_classes(3, true, true)).unwrap()).unwrap();
        assert_eq!(resp.seq(), 3);
        assert_eq!(resp.iin1 & IIN1_RESTART, IIN1_RESTART);
        let find = |k: &str, i: u16| resp.points.iter().find(|p| p.kind == k && p.index == i).cloned().unwrap();
        assert_eq!(find("binary", 0).value, PValue::Bool(true));
        assert_eq!(find("binary", 1).value, PValue::Bool(false));
        assert_eq!(find("counter", 0).value, PValue::Int(123_456));
        assert_eq!(find("analog", 0).value, PValue::Float(3412.5));
        assert_eq!(find("analog", 1).value, PValue::Float(-7.25));
        assert!(!find("analog", 5).online);
        // Clear restart.
        let r = parse_response(&o.handle(&clear_restart(4)).unwrap()).unwrap();
        assert_eq!(r.iin1 & IIN1_RESTART, 0);
        // Controls land in the database and echo success.
        let r = parse_response(&o.handle(&operate_binary(5, 2, true)).unwrap()).unwrap();
        assert_eq!(r.control_status, vec![0]);
        assert_eq!(o.binary_outputs.get(&2), Some(&true));
        let r = parse_response(&o.handle(&operate_analog(6, 1, 55.5, true)).unwrap()).unwrap();
        assert_eq!(r.control_status, vec![0]);
        assert_eq!(o.analog_outputs.get(&1), Some(&55.5));
        o.handle(&operate_analog(7, 0, 12.6, false));
        assert_eq!(o.analog_outputs.get(&0), Some(&13.0));
        // Status of outputs is reported back on the next integrity poll.
        let resp = parse_response(&o.handle(&read_classes(8, true, false)).unwrap()).unwrap();
        assert!(resp.points.iter().any(|p| p.kind == "binary_output" && p.index == 2 && p.value == PValue::Bool(true)));
        assert!(resp.points.iter().any(|p| p.kind == "analog_output" && p.index == 1 && p.value == PValue::Float(55.5)));
        assert_eq!(o.handle(&confirm(1, false)), None);
    }

    #[test]
    fn events_packed_bits_and_errors() {
        // Response with g2v1 (index-prefixed binary event), g32v5 float event, g1v1 packed bits.
        let mut a = vec![0xC0, 0x81, 0x00, 0x00];
        a.extend_from_slice(&[2, 1, 0x17, 1, 4, 0x81]);
        a.extend_from_slice(&[32, 5, 0x28, 1, 0, 9, 0, 0x01]);
        a.extend_from_slice(&2.5f32.to_le_bytes());
        a.extend_from_slice(&[1, 1, 0x00, 0, 9, 0b0000_0101, 0b0000_0010]);
        let r = parse_response(&a).unwrap();
        assert_eq!(r.points[0], PointUpdate { kind: "binary", index: 4, value: PValue::Bool(true), online: true });
        assert_eq!(r.points[1].value, PValue::Float(2.5));
        assert_eq!(r.points[1].index, 9);
        let bits: Vec<bool> = r.points[2..].iter().map(|p| p.value == PValue::Bool(true)).collect();
        assert_eq!(bits, vec![true, false, true, false, false, false, false, false, false, true]);
        assert!(parse_response(&[0xC0, 0x01, 0, 0]).is_err());
        assert!(parse_response(&[0xC0, 0x81, 0, 0, 99, 9, 0x00, 0, 1]).is_err());
        assert!(parse_response(&[0xC0, 0x81, 0, 0, 30, 5, 0x00, 0, 3, 1]).is_err());
    }
}
