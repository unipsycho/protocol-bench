//! Modbus framing. Pure functions: request / response PDUs, MBAP (TCP) header, RTU CRC.

use crate::codec::Area;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModbusError {
    /// Socket / serial failure.
    Io(String),
    Timeout,
    /// Device answered with an exception code (link is fine, request was refused).
    Exception(u8),
    /// Malformed or mismatched frame.
    Protocol(String),
}

impl std::fmt::Display for ModbusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(m) => write!(f, "io: {m}"),
            Self::Timeout => f.write_str("timeout"),
            Self::Exception(c) => write!(f, "exception {c:#04x} ({})", exception_name(*c)),
            Self::Protocol(m) => write!(f, "protocol: {m}"),
        }
    }
}

impl std::error::Error for ModbusError {}

pub fn exception_name(code: u8) -> &'static str {
    match code {
        1 => "illegal function",
        2 => "illegal data address",
        3 => "illegal data value",
        4 => "server device failure",
        5 => "acknowledge",
        6 => "server busy",
        10 => "gateway path unavailable",
        11 => "gateway target failed to respond",
        _ => "unknown",
    }
}

pub const EX_ILLEGAL_FUNCTION: u8 = 1;
pub const EX_ILLEGAL_ADDRESS: u8 = 2;
pub const EX_ILLEGAL_VALUE: u8 = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    ReadBits { area: Area, start: u16, count: u16 },
    ReadRegs { area: Area, start: u16, count: u16 },
    WriteCoil { addr: u16, on: bool },
    WriteReg { addr: u16, value: u16 },
    WriteCoils { start: u16, values: Vec<bool> },
    WriteRegs { start: u16, values: Vec<u16> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    Bits(Vec<bool>),
    Regs(Vec<u16>),
    /// Write acknowledged (echo carries no data we need).
    Written,
}

impl Request {
    pub fn function(&self) -> u8 {
        match self {
            Self::ReadBits { area: Area::Coil, .. } => 1,
            Self::ReadBits { .. } => 2,
            Self::ReadRegs { area: Area::Holding, .. } => 3,
            Self::ReadRegs { .. } => 4,
            Self::WriteCoil { .. } => 5,
            Self::WriteReg { .. } => 6,
            Self::WriteCoils { .. } => 15,
            Self::WriteRegs { .. } => 16,
        }
    }

    /// Request PDU (function code + data).
    pub fn pdu(&self) -> Vec<u8> {
        let mut p = vec![self.function()];
        match self {
            Self::ReadBits { start, count, .. } | Self::ReadRegs { start, count, .. } => {
                p.extend_from_slice(&start.to_be_bytes());
                p.extend_from_slice(&count.to_be_bytes());
            }
            Self::WriteCoil { addr, on } => {
                p.extend_from_slice(&addr.to_be_bytes());
                p.extend_from_slice(&(if *on { 0xFF00u16 } else { 0 }).to_be_bytes());
            }
            Self::WriteReg { addr, value } => {
                p.extend_from_slice(&addr.to_be_bytes());
                p.extend_from_slice(&value.to_be_bytes());
            }
            Self::WriteCoils { start, values } => {
                p.extend_from_slice(&start.to_be_bytes());
                p.extend_from_slice(&(values.len() as u16).to_be_bytes());
                let packed = pack_bits(values);
                p.push(packed.len() as u8);
                p.extend_from_slice(&packed);
            }
            Self::WriteRegs { start, values } => {
                p.extend_from_slice(&start.to_be_bytes());
                p.extend_from_slice(&(values.len() as u16).to_be_bytes());
                p.push((values.len() * 2) as u8);
                for v in values {
                    p.extend_from_slice(&v.to_be_bytes());
                }
            }
        }
        p
    }

    /// Server side: parse a request PDU. `Err(code)` is the exception to send back.
    pub fn parse(pdu: &[u8]) -> Result<Self, u8> {
        let Some(&fc) = pdu.first() else {
            return Err(EX_ILLEGAL_FUNCTION);
        };
        let u16_at = |i: usize| -> Result<u16, u8> {
            pdu.get(i..i + 2)
                .map(|b| u16::from_be_bytes([b[0], b[1]]))
                .ok_or(EX_ILLEGAL_VALUE)
        };
        match fc {
            1..=4 => {
                let (start, count) = (u16_at(1)?, u16_at(3)?);
                let bit = fc <= 2;
                let max = if bit { crate::codec::MAX_READ_BITS } else { crate::codec::MAX_READ_REGS };
                if count == 0 || count > max {
                    return Err(EX_ILLEGAL_VALUE);
                }
                Ok(match fc {
                    1 => Self::ReadBits { area: Area::Coil, start, count },
                    2 => Self::ReadBits { area: Area::Discrete, start, count },
                    3 => Self::ReadRegs { area: Area::Holding, start, count },
                    _ => Self::ReadRegs { area: Area::Input, start, count },
                })
            }
            5 => {
                let (addr, v) = (u16_at(1)?, u16_at(3)?);
                match v {
                    0xFF00 => Ok(Self::WriteCoil { addr, on: true }),
                    0 => Ok(Self::WriteCoil { addr, on: false }),
                    _ => Err(EX_ILLEGAL_VALUE),
                }
            }
            6 => Ok(Self::WriteReg { addr: u16_at(1)?, value: u16_at(3)? }),
            15 => {
                let (start, count) = (u16_at(1)?, u16_at(3)?);
                let n = *pdu.get(5).ok_or(EX_ILLEGAL_VALUE)? as usize;
                let data = pdu.get(6..6 + n).ok_or(EX_ILLEGAL_VALUE)?;
                if count == 0 || count > 1968 || n != (count as usize).div_ceil(8) {
                    return Err(EX_ILLEGAL_VALUE);
                }
                Ok(Self::WriteCoils { start, values: unpack_bits(data, count as usize) })
            }
            16 => {
                let (start, count) = (u16_at(1)?, u16_at(3)?);
                let n = *pdu.get(5).ok_or(EX_ILLEGAL_VALUE)? as usize;
                let data = pdu.get(6..6 + n).ok_or(EX_ILLEGAL_VALUE)?;
                if count == 0 || count > 123 || n != count as usize * 2 {
                    return Err(EX_ILLEGAL_VALUE);
                }
                let values = data.chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
                Ok(Self::WriteRegs { start, values })
            }
            _ => Err(EX_ILLEGAL_FUNCTION),
        }
    }

    /// Server side: encode a normal response PDU for this request.
    pub fn response_pdu(&self, resp: &Response) -> Vec<u8> {
        let fc = self.function();
        match (self, resp) {
            (Self::ReadBits { .. }, Response::Bits(bits)) => {
                let packed = pack_bits(bits);
                let mut p = vec![fc, packed.len() as u8];
                p.extend(packed);
                p
            }
            (Self::ReadRegs { .. }, Response::Regs(regs)) => {
                let mut p = vec![fc, (regs.len() * 2) as u8];
                for r in regs {
                    p.extend_from_slice(&r.to_be_bytes());
                }
                p
            }
            // Writes echo the request header (first 5 bytes of the PDU).
            _ => self.pdu().into_iter().take(5).collect(),
        }
    }

    /// Client side: validate and decode the response PDU.
    pub fn parse_response(&self, pdu: &[u8]) -> Result<Response, ModbusError> {
        let fc = self.function();
        let Some(&got) = pdu.first() else {
            return Err(ModbusError::Protocol("empty response".into()));
        };
        if got == fc | 0x80 {
            return Err(ModbusError::Exception(*pdu.get(1).unwrap_or(&0)));
        }
        if got != fc {
            return Err(ModbusError::Protocol(format!("function {got:#04x}, expected {fc:#04x}")));
        }
        let short = || ModbusError::Protocol("short response".into());
        match self {
            Self::ReadBits { count, .. } => {
                let n = *pdu.get(1).ok_or_else(short)? as usize;
                if n != (*count as usize).div_ceil(8) {
                    return Err(ModbusError::Protocol(format!("byte count {n}")));
                }
                let data = pdu.get(2..2 + n).ok_or_else(short)?;
                Ok(Response::Bits(unpack_bits(data, *count as usize)))
            }
            Self::ReadRegs { count, .. } => {
                let n = *pdu.get(1).ok_or_else(short)? as usize;
                if n != *count as usize * 2 {
                    return Err(ModbusError::Protocol(format!("byte count {n}")));
                }
                let data = pdu.get(2..2 + n).ok_or_else(short)?;
                Ok(Response::Regs(
                    data.chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect(),
                ))
            }
            _ => {
                if pdu.len() < 5 {
                    return Err(short());
                }
                Ok(Response::Written)
            }
        }
    }
}

/// Exception response PDU.
pub fn exception_pdu(fc: u8, code: u8) -> Vec<u8> {
    vec![fc | 0x80, code]
}

pub fn pack_bits(bits: &[bool]) -> Vec<u8> {
    let mut out = vec![0u8; bits.len().div_ceil(8)];
    for (i, b) in bits.iter().enumerate() {
        if *b {
            out[i / 8] |= 1 << (i % 8);
        }
    }
    out
}

pub fn unpack_bits(data: &[u8], count: usize) -> Vec<bool> {
    (0..count).map(|i| data[i / 8] >> (i % 8) & 1 == 1).collect()
}

/// Modbus RTU CRC-16 (poly 0xA001, init 0xFFFF). Appended low byte first.
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0xFFFFu16;
    for &b in data {
        crc ^= b as u16;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xA001 } else { crc >> 1 };
        }
    }
    crc
}

/// RTU frame: unit + PDU + CRC.
pub fn rtu_frame(unit: u8, pdu: &[u8]) -> Vec<u8> {
    let mut f = Vec::with_capacity(pdu.len() + 3);
    f.push(unit);
    f.extend_from_slice(pdu);
    f.extend_from_slice(&crc16(&f).to_le_bytes());
    f
}

/// Verify an RTU frame's CRC and split it into `(unit, pdu)`.
pub fn rtu_split(frame: &[u8]) -> Result<(u8, &[u8]), ModbusError> {
    if frame.len() < 4 {
        return Err(ModbusError::Protocol("short rtu frame".into()));
    }
    let (body, crc) = frame.split_at(frame.len() - 2);
    if crc16(body).to_le_bytes() != [crc[0], crc[1]] {
        return Err(ModbusError::Protocol("crc mismatch".into()));
    }
    Ok((body[0], &body[1..]))
}

/// MBAP header + PDU.
pub fn mbap_frame(tid: u16, unit: u8, pdu: &[u8]) -> Vec<u8> {
    let mut f = Vec::with_capacity(pdu.len() + 7);
    f.extend_from_slice(&tid.to_be_bytes());
    f.extend_from_slice(&0u16.to_be_bytes());
    f.extend_from_slice(&((pdu.len() + 1) as u16).to_be_bytes());
    f.push(unit);
    f.extend_from_slice(pdu);
    f
}

/// Parse an MBAP header: `(tid, length_after_header, unit)` where `length_after_header` is the
/// PDU byte count still to read.
pub fn mbap_header(h: &[u8; 7]) -> Result<(u16, usize, u8), ModbusError> {
    let tid = u16::from_be_bytes([h[0], h[1]]);
    let pid = u16::from_be_bytes([h[2], h[3]]);
    let len = u16::from_be_bytes([h[4], h[5]]) as usize;
    if pid != 0 {
        return Err(ModbusError::Protocol(format!("protocol id {pid}")));
    }
    if !(2..=254).contains(&len) {
        return Err(ModbusError::Protocol(format!("length {len}")));
    }
    Ok((tid, len - 1, h[6]))
}

/// Bytes still to read after `[unit, fc]` of an RTU **response**, including the CRC.
/// For read functions the byte count is one more byte away, so pass it when known.
pub fn rtu_response_rest(fc: u8, byte_count: Option<u8>) -> Option<usize> {
    if fc & 0x80 != 0 {
        return Some(3); // exception code + CRC
    }
    match fc {
        1..=4 => byte_count.map(|n| n as usize + 2),
        5 | 6 | 15 | 16 => Some(6),
        _ => None,
    }
}

/// Total length of an RTU **request** frame (unit + PDU + CRC) once enough bytes have arrived to
/// know it, else `None` (keep reading). Unknown functions report a 4-byte minimum so the server
/// can answer with an exception and resync.
pub fn rtu_request_len(buf: &[u8]) -> Option<usize> {
    if buf.len() < 2 {
        return None;
    }
    match buf[1] {
        1..=6 => Some(8),
        15 | 16 => (buf.len() >= 7).then(|| 7 + buf[6] as usize + 2),
        _ => Some(4),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rtu_request_lengths() {
        assert_eq!(rtu_request_len(&[1]), None);
        assert_eq!(rtu_request_len(&[1, 3]), Some(8));
        assert_eq!(rtu_request_len(&[1, 16, 0, 1, 0, 2]), None);
        assert_eq!(rtu_request_len(&[1, 16, 0, 1, 0, 2, 4]), Some(13));
        assert_eq!(rtu_request_len(&[1, 0x2B]), Some(4));
    }

    #[test]
    fn crc_known_vector() {
        // Classic example: 01 03 00 00 00 0A -> CRC C5 CD (0xCDC5)
        assert_eq!(crc16(&[0x01, 0x03, 0x00, 0x00, 0x00, 0x0A]), 0xCDC5);
        let f = rtu_frame(1, &[0x03, 0x00, 0x00, 0x00, 0x0A]);
        assert_eq!(f, vec![0x01, 0x03, 0x00, 0x00, 0x00, 0x0A, 0xC5, 0xCD]);
        assert_eq!(rtu_split(&f).unwrap(), (1, &f[1..6]));
        let mut bad = f.clone();
        bad[2] ^= 1;
        assert!(rtu_split(&bad).is_err());
    }

    #[test]
    fn request_pdus_match_spec_examples() {
        assert_eq!(
            Request::ReadRegs { area: Area::Holding, start: 0x006B, count: 3 }.pdu(),
            vec![0x03, 0x00, 0x6B, 0x00, 0x03]
        );
        assert_eq!(Request::WriteCoil { addr: 0x00AC, on: true }.pdu(), vec![0x05, 0x00, 0xAC, 0xFF, 0x00]);
        assert_eq!(
            Request::WriteRegs { start: 1, values: vec![0x000A, 0x0102] }.pdu(),
            vec![0x10, 0x00, 0x01, 0x00, 0x02, 0x04, 0x00, 0x0A, 0x01, 0x02]
        );
        let coils = Request::WriteCoils {
            start: 0x0013,
            values: vec![true, false, true, true, false, false, true, true, true, false],
        };
        assert_eq!(coils.pdu(), vec![0x0F, 0x00, 0x13, 0x00, 0x0A, 0x02, 0xCD, 0x01]);
    }

    #[test]
    fn requests_round_trip_through_parse() {
        let reqs = [
            Request::ReadBits { area: Area::Coil, start: 5, count: 9 },
            Request::ReadBits { area: Area::Discrete, start: 0, count: 2000 },
            Request::ReadRegs { area: Area::Holding, start: 100, count: 125 },
            Request::ReadRegs { area: Area::Input, start: 1, count: 1 },
            Request::WriteCoil { addr: 7, on: false },
            Request::WriteReg { addr: 8, value: 0xBEEF },
            Request::WriteCoils { start: 3, values: vec![true, true, false] },
            Request::WriteRegs { start: 9, values: vec![1, 2, 3] },
        ];
        for r in reqs {
            assert_eq!(Request::parse(&r.pdu()).unwrap(), r);
        }
        assert_eq!(Request::parse(&[0x03, 0, 0, 0, 0]), Err(EX_ILLEGAL_VALUE)); // count 0
        assert_eq!(Request::parse(&[0x03, 0, 0, 0, 126]), Err(EX_ILLEGAL_VALUE));
        assert_eq!(Request::parse(&[0x2B]), Err(EX_ILLEGAL_FUNCTION));
        assert_eq!(Request::parse(&[0x05, 0, 1, 0x12, 0x34]), Err(EX_ILLEGAL_VALUE));
    }

    #[test]
    fn responses_decode_and_exceptions_surface() {
        let req = Request::ReadRegs { area: Area::Holding, start: 0, count: 2 };
        let resp = req.response_pdu(&Response::Regs(vec![0x1234, 0xABCD]));
        assert_eq!(resp, vec![0x03, 0x04, 0x12, 0x34, 0xAB, 0xCD]);
        assert_eq!(req.parse_response(&resp).unwrap(), Response::Regs(vec![0x1234, 0xABCD]));
        assert_eq!(req.parse_response(&exception_pdu(3, 2)), Err(ModbusError::Exception(2)));
        assert!(req.parse_response(&[0x04, 0x04, 0, 0, 0, 0]).is_err()); // wrong function
        assert!(req.parse_response(&[0x03, 0x02, 0, 0]).is_err()); // wrong byte count

        let bits = Request::ReadBits { area: Area::Coil, start: 0, count: 10 };
        let v = vec![true, false, true, true, false, false, true, true, true, false];
        let p = bits.response_pdu(&Response::Bits(v.clone()));
        assert_eq!(p, vec![0x01, 0x02, 0xCD, 0x01]);
        assert_eq!(bits.parse_response(&p).unwrap(), Response::Bits(v));

        let w = Request::WriteReg { addr: 1, value: 3 };
        assert_eq!(w.parse_response(&w.response_pdu(&Response::Written)).unwrap(), Response::Written);
    }

    #[test]
    fn mbap_round_trip() {
        let pdu = [0x03, 0x00, 0x00, 0x00, 0x02];
        let f = mbap_frame(0x1234, 7, &pdu);
        assert_eq!(&f[..7], &[0x12, 0x34, 0, 0, 0, 6, 7]);
        let (tid, rest, unit) = mbap_header(f[..7].try_into().unwrap()).unwrap();
        assert_eq!((tid, rest, unit), (0x1234, 5, 7));
        assert!(mbap_header(&[0, 0, 0, 1, 0, 6, 1]).is_err());
        assert!(mbap_header(&[0, 0, 0, 0, 0, 1, 1]).is_err());
    }

    #[test]
    fn rtu_response_lengths() {
        assert_eq!(rtu_response_rest(0x83, None), Some(3));
        assert_eq!(rtu_response_rest(3, None), None);
        assert_eq!(rtu_response_rest(3, Some(4)), Some(6));
        assert_eq!(rtu_response_rest(6, None), Some(6));
        assert_eq!(rtu_response_rest(0x2B, None), None);
    }
}
