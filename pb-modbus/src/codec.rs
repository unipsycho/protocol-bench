//! Register ↔ value codec. No I/O — everything here is unit-tested against known byte sequences.
//!
//! A Modbus register is 16 bits, sent big-endian on the wire. Multi-register values differ per
//! device in byte and word order, so every decode / encode takes a [`ByteOrder`]:
//!
//! | Order | 32-bit value `0x0A0B0C0D` arrives as words |
//! | --- | --- |
//! | `Abcd` (big-endian, high word first) | `0x0A0B 0x0C0D` |
//! | `Cdab` (word swap) | `0x0C0D 0x0A0B` |
//! | `Badc` (byte swap in each word) | `0x0B0A 0x0D0C` |
//! | `Dcba` (little-endian) | `0x0D0C 0x0B0A` |

use std::fmt;

/// Largest register read / write Modbus allows in one request.
pub const MAX_READ_REGS: u16 = 125;
/// Largest coil / discrete read in one request.
pub const MAX_READ_BITS: u16 = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteOrder {
    Abcd,
    Cdab,
    Badc,
    Dcba,
}

impl ByteOrder {
    pub const ALL: [ByteOrder; 4] = [Self::Abcd, Self::Cdab, Self::Badc, Self::Dcba];

    /// Wire name used in bind JSON (`abcd`, `cdab`, `badc`, `dcba`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Abcd => "abcd",
            Self::Cdab => "cdab",
            Self::Badc => "badc",
            Self::Dcba => "dcba",
        }
    }

    /// Name as shown in the UI.
    pub fn label(self) -> &'static str {
        match self {
            Self::Abcd => "ABCD  big-endian",
            Self::Cdab => "CDAB  word swap",
            Self::Badc => "BADC  byte swap",
            Self::Dcba => "DCBA  little-endian",
        }
    }

    /// Parse the wire name; `None` when unknown.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "abcd" | "be" => Some(Self::Abcd),
            "cdab" => Some(Self::Cdab),
            "badc" => Some(Self::Badc),
            "dcba" | "le" => Some(Self::Dcba),
            _ => None,
        }
    }

    fn swaps_bytes(self) -> bool {
        matches!(self, Self::Badc | Self::Dcba)
    }

    fn reverses_words(self) -> bool {
        matches!(self, Self::Cdab | Self::Dcba)
    }
}

/// Wire data type of one bind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataType {
    /// Coil / discrete input (bit area only).
    Bool,
    U16,
    I16,
    U32,
    I32,
    U64,
    I64,
    F32,
    F64,
    /// One bit (0 = least significant) of a single register.
    Bit(u8),
    /// ASCII text across `n` registers (2 chars each, NUL / space padded).
    Str(u16),
}

impl DataType {
    /// Parse the persisted form: `bool`, `u16`, `f32`, `bit3`, `str8`.
    pub fn parse(s: &str) -> Option<Self> {
        let t = s.trim().to_ascii_lowercase();
        Some(match t.as_str() {
            "bool" => Self::Bool,
            "u16" => Self::U16,
            "i16" => Self::I16,
            "u32" => Self::U32,
            "i32" => Self::I32,
            "u64" => Self::U64,
            "i64" => Self::I64,
            "f32" => Self::F32,
            "f64" => Self::F64,
            _ => {
                if let Some(n) = t.strip_prefix("bit") {
                    let b: u8 = n.parse().ok()?;
                    return (b < 16).then_some(Self::Bit(b));
                }
                if let Some(n) = t.strip_prefix("str") {
                    let n: u16 = n.parse().ok()?;
                    return (1..=MAX_READ_REGS).contains(&n).then_some(Self::Str(n));
                }
                return None;
            }
        })
    }

    /// Value rendered as point text.
    pub fn as_string(self) -> String {
        match self {
            Self::Bool => "bool".into(),
            Self::U16 => "u16".into(),
            Self::I16 => "i16".into(),
            Self::U32 => "u32".into(),
            Self::I32 => "i32".into(),
            Self::U64 => "u64".into(),
            Self::I64 => "i64".into(),
            Self::F32 => "f32".into(),
            Self::F64 => "f64".into(),
            Self::Bit(b) => format!("bit{b}"),
            Self::Str(n) => format!("str{n}"),
        }
    }

    /// Registers (or bits, for [`DataType::Bool`]) the value occupies.
    pub fn width(self) -> u16 {
        match self {
            Self::Bool | Self::U16 | Self::I16 | Self::Bit(_) => 1,
            Self::U32 | Self::I32 | Self::F32 => 2,
            Self::U64 | Self::I64 | Self::F64 => 4,
            Self::Str(n) => n,
        }
    }

    /// True when the order setting changes the result.
    pub fn uses_order(self) -> bool {
        !matches!(self, Self::Bool | Self::Bit(_))
    }
}

/// Decoded value. Maps onto host point kinds (bool / integer / float / string).
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    UInt(u64),
    Float(f64),
    Text(String),
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bool(b) => write!(f, "{b}"),
            Self::Int(n) => write!(f, "{n}"),
            Self::UInt(n) => write!(f, "{n}"),
            Self::Float(x) => write!(f, "{x}"),
            Self::Text(t) => f.write_str(t),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodecError {
    /// Wrong number of registers for the type.
    Width { want: u16, got: usize },
    /// Value cannot be represented (out of range / unparseable).
    Value(String),
    /// Type not valid here (e.g. `Bool` through the register path).
    Type(String),
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Width { want, got } => write!(f, "expected {want} registers, got {got}"),
            Self::Value(m) | Self::Type(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for CodecError {}

/// Wire words → big-endian numeric bytes for the given order.
fn words_to_be_bytes(words: &[u16], order: ByteOrder) -> Vec<u8> {
    let mut ws: Vec<u16> = words.to_vec();
    if order.reverses_words() {
        ws.reverse();
    }
    let mut out = Vec::with_capacity(ws.len() * 2);
    for w in ws {
        let w = if order.swaps_bytes() { w.swap_bytes() } else { w };
        out.extend_from_slice(&w.to_be_bytes());
    }
    out
}

/// Inverse of [`words_to_be_bytes`].
fn be_bytes_to_words(bytes: &[u8], order: ByteOrder) -> Vec<u16> {
    let mut ws: Vec<u16> = bytes
        .chunks(2)
        .map(|c| {
            let w = u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)]);
            if order.swaps_bytes() { w.swap_bytes() } else { w }
        })
        .collect();
    if order.reverses_words() {
        ws.reverse();
    }
    ws
}

/// Decode register words. Bit-area values use [`decode_bit`].
pub fn decode(dt: DataType, order: ByteOrder, words: &[u16]) -> Result<Value, CodecError> {
    if dt == DataType::Bool {
        return Err(CodecError::Type("bool is a coil / discrete type; use decode_bit".into()));
    }
    if words.len() != dt.width() as usize {
        return Err(CodecError::Width { want: dt.width(), got: words.len() });
    }
    if let DataType::Bit(b) = dt {
        return Ok(Value::Bool(words[0] >> b & 1 == 1));
    }
    if let DataType::Str(_) = dt {
        // Text is byte-stream, so only the byte-swap half of the order applies per word.
        let mut bytes = Vec::with_capacity(words.len() * 2);
        for &w in words {
            let w = if order.swaps_bytes() { w.swap_bytes() } else { w };
            bytes.extend_from_slice(&w.to_be_bytes());
        }
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        let text = String::from_utf8_lossy(&bytes[..end]).trim_end().to_string();
        return Ok(Value::Text(text));
    }
    let b = words_to_be_bytes(words, order);
    Ok(match dt {
        DataType::U16 => Value::UInt(u16::from_be_bytes([b[0], b[1]]) as u64),
        DataType::I16 => Value::Int(i16::from_be_bytes([b[0], b[1]]) as i64),
        DataType::U32 => Value::UInt(u32::from_be_bytes(b[..4].try_into().unwrap()) as u64),
        DataType::I32 => Value::Int(i32::from_be_bytes(b[..4].try_into().unwrap()) as i64),
        DataType::U64 => Value::UInt(u64::from_be_bytes(b[..8].try_into().unwrap())),
        DataType::I64 => Value::Int(i64::from_be_bytes(b[..8].try_into().unwrap())),
        DataType::F32 => Value::Float(f32::from_be_bytes(b[..4].try_into().unwrap()) as f64),
        DataType::F64 => Value::Float(f64::from_be_bytes(b[..8].try_into().unwrap())),
        DataType::Bool | DataType::Bit(_) | DataType::Str(_) => unreachable!(),
    })
}

/// A coil / discrete input is already a bool.
pub fn decode_bit(raw: bool) -> Value {
    Value::Bool(raw)
}

/// Encode a value to register words. `Bit` returns the single word with only that bit set /
/// cleared from zero — use [`merge_bit`] for read-modify-write against the current register.
pub fn encode(dt: DataType, order: ByteOrder, v: &Value) -> Result<Vec<u16>, CodecError> {
    let bad = |what: &str| CodecError::Value(format!("{v} is not a valid {what}"));
    let as_i = || -> Result<i128, CodecError> {
        match v {
            Value::Int(n) => Ok(*n as i128),
            Value::UInt(n) => Ok(*n as i128),
            Value::Bool(b) => Ok(*b as i128),
            Value::Float(x) if x.is_finite() && x.fract() == 0.0 => Ok(*x as i128),
            _ => Err(CodecError::Value(format!("{v} is not an integer"))),
        }
    };
    let as_f = || -> Result<f64, CodecError> {
        match v {
            Value::Float(x) => Ok(*x),
            Value::Int(n) => Ok(*n as f64),
            Value::UInt(n) => Ok(*n as f64),
            Value::Bool(b) => Ok(*b as u8 as f64),
            Value::Text(_) => Err(CodecError::Value(format!("{v} is not a number"))),
        }
    };
    let ranged = |lo: i128, hi: i128, what: &str| -> Result<i128, CodecError> {
        let n = as_i()?;
        if n < lo || n > hi {
            Err(bad(what))
        } else {
            Ok(n)
        }
    };
    let bytes: Vec<u8> = match dt {
        DataType::Bool => {
            return Err(CodecError::Type("bool is a coil type; use encode_bit".into()))
        }
        DataType::Bit(b) => {
            let on = encode_bit(v)?;
            return Ok(vec![(on as u16) << b]);
        }
        DataType::U16 => (ranged(0, u16::MAX as i128, "u16")? as u16).to_be_bytes().to_vec(),
        DataType::I16 => (ranged(i16::MIN as i128, i16::MAX as i128, "i16")? as i16)
            .to_be_bytes()
            .to_vec(),
        DataType::U32 => (ranged(0, u32::MAX as i128, "u32")? as u32).to_be_bytes().to_vec(),
        DataType::I32 => (ranged(i32::MIN as i128, i32::MAX as i128, "i32")? as i32)
            .to_be_bytes()
            .to_vec(),
        DataType::U64 => (ranged(0, u64::MAX as i128, "u64")? as u64).to_be_bytes().to_vec(),
        DataType::I64 => (ranged(i64::MIN as i128, i64::MAX as i128, "i64")? as i64)
            .to_be_bytes()
            .to_vec(),
        DataType::F32 => (as_f()? as f32).to_be_bytes().to_vec(),
        DataType::F64 => as_f()?.to_be_bytes().to_vec(),
        DataType::Str(n) => {
            let text = match v {
                Value::Text(t) => t.clone(),
                other => other.to_string(),
            };
            if !text.is_ascii() {
                return Err(CodecError::Value("string registers are ASCII only".into()));
            }
            let cap = n as usize * 2;
            if text.len() > cap {
                return Err(CodecError::Value(format!("\"{text}\" exceeds {cap} characters")));
            }
            let mut b = text.into_bytes();
            b.resize(cap, 0);
            // Byte-stream: only the per-word byte swap applies.
            return Ok(b
                .chunks(2)
                .map(|c| {
                    let w = u16::from_be_bytes([c[0], c[1]]);
                    if order.swaps_bytes() { w.swap_bytes() } else { w }
                })
                .collect());
        }
    };
    Ok(be_bytes_to_words(&bytes, order))
}

/// Value → coil state.
pub fn encode_bit(v: &Value) -> Result<bool, CodecError> {
    match v {
        Value::Bool(b) => Ok(*b),
        Value::Int(n) => Ok(*n != 0),
        Value::UInt(n) => Ok(*n != 0),
        Value::Float(x) => Ok(*x != 0.0),
        Value::Text(t) => match t.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "on" => Ok(true),
            "false" | "0" | "off" => Ok(false),
            _ => Err(CodecError::Value(format!("\"{t}\" is not a boolean"))),
        },
    }
}

/// Set / clear `bit` of `current` (read-modify-write for [`DataType::Bit`] writes).
pub fn merge_bit(current: u16, bit: u8, on: bool) -> u16 {
    if on {
        current | (1 << bit)
    } else {
        current & !(1 << bit)
    }
}

/// Parse a host point string (as carried by `write_point`) into a [`Value`] suited to `dt`.
pub fn parse_point_value(dt: DataType, raw: &str) -> Result<Value, CodecError> {
    let t = raw.trim();
    match dt {
        DataType::Bool | DataType::Bit(_) => encode_bit(&Value::Text(t.to_string())).map(Value::Bool),
        DataType::Str(_) => Ok(Value::Text(raw.to_string())),
        DataType::F32 | DataType::F64 => t
            .parse::<f64>()
            .map(Value::Float)
            .map_err(|_| CodecError::Value(format!("\"{t}\" is not a number"))),
        _ => {
            if let Ok(n) = t.parse::<i64>() {
                Ok(Value::Int(n))
            } else if let Ok(n) = t.parse::<u64>() {
                Ok(Value::UInt(n))
            } else if let Ok(x) = t.parse::<f64>() {
                if x.fract() == 0.0 && x.is_finite() {
                    Ok(Value::Float(x))
                } else {
                    Err(CodecError::Value(format!("\"{t}\" is not an integer")))
                }
            } else {
                Err(CodecError::Value(format!("\"{t}\" is not a number")))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Read planning
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Area {
    Coil,
    Discrete,
    Input,
    Holding,
}

impl Area {
    /// Parse the wire name; `None` when unknown.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "coil" => Some(Self::Coil),
            "discrete" => Some(Self::Discrete),
            "input" => Some(Self::Input),
            "holding" => Some(Self::Holding),
            _ => None,
        }
    }

    /// True for the two single-bit areas (coils, discrete inputs).
    pub fn is_bit(self) -> bool {
        matches!(self, Self::Coil | Self::Discrete)
    }

    /// Only coils and holding registers can be written; input registers and discrete inputs are read-only.
    pub fn writable(self) -> bool {
        matches!(self, Self::Coil | Self::Holding)
    }

    fn max_block(self) -> u16 {
        if self.is_bit() { MAX_READ_BITS } else { MAX_READ_REGS }
    }
}

/// One register / coil range a bind needs on every poll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadItem {
    /// Caller's index for this bind.
    pub key: usize,
    pub unit: u8,
    pub area: Area,
    pub address: u16,
    pub width: u16,
}

/// One Modbus request covering several items.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadBlock {
    pub unit: u8,
    pub area: Area,
    pub start: u16,
    pub count: u16,
    /// `(key, offset within block)`.
    pub items: Vec<(usize, u16)>,
}

/// Group items into the fewest requests. Neighbours within `max_gap` unused registers share a
/// block (reading a few extra words is cheaper than another round trip); blocks never exceed the
/// protocol limit.
pub fn plan_reads(items: &[ReadItem], max_gap: u16) -> Vec<ReadBlock> {
    let mut sorted: Vec<ReadItem> = items.to_vec();
    sorted.sort_by_key(|i| (i.unit, i.area, i.address));
    let mut blocks: Vec<ReadBlock> = Vec::new();
    for it in sorted {
        let end = it.address as u32 + it.width as u32;
        if let Some(b) = blocks.last_mut() {
            let b_end = b.start as u32 + b.count as u32;
            let same = b.unit == it.unit && b.area == it.area;
            let gap = (it.address as u32).saturating_sub(b_end);
            let new_count = end.max(b_end) - b.start as u32;
            if same && gap <= max_gap as u32 && new_count <= it.area.max_block() as u32 {
                b.items.push((it.key, it.address - b.start));
                b.count = new_count as u16;
                continue;
            }
        }
        blocks.push(ReadBlock {
            unit: it.unit,
            area: it.area,
            start: it.address,
            count: it.width,
            items: vec![(it.key, 0)],
        });
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_TYPES: [DataType; 11] = [
        DataType::U16,
        DataType::I16,
        DataType::U32,
        DataType::I32,
        DataType::U64,
        DataType::I64,
        DataType::F32,
        DataType::F64,
        DataType::Bit(5),
        DataType::Str(4),
        DataType::Bool,
    ];

    #[test]
    fn u32_known_sequences_for_each_order() {
        // 0x0A0B0C0D = 168496141
        let cases = [
            (ByteOrder::Abcd, [0x0A0B, 0x0C0D]),
            (ByteOrder::Cdab, [0x0C0D, 0x0A0B]),
            (ByteOrder::Badc, [0x0B0A, 0x0D0C]),
            (ByteOrder::Dcba, [0x0D0C, 0x0B0A]),
        ];
        for (order, words) in cases {
            assert_eq!(decode(DataType::U32, order, &words).unwrap(), Value::UInt(0x0A0B0C0D), "{order:?}");
            assert_eq!(encode(DataType::U32, order, &Value::UInt(0x0A0B0C0D)).unwrap(), words.to_vec());
        }
    }

    #[test]
    fn f32_known_values() {
        // 123.456f32 = 0x42F6E979
        assert_eq!(
            decode(DataType::F32, ByteOrder::Abcd, &[0x42F6, 0xE979]).unwrap(),
            Value::Float(123.456f32 as f64)
        );
        // Same float from a CDAB device.
        assert_eq!(
            decode(DataType::F32, ByteOrder::Cdab, &[0xE979, 0x42F6]).unwrap(),
            Value::Float(123.456f32 as f64)
        );
        assert_eq!(
            decode(DataType::F32, ByteOrder::Dcba, &[0x79E9, 0xF642]).unwrap(),
            Value::Float(123.456f32 as f64)
        );
    }

    #[test]
    fn sixty_four_bit_orders() {
        let v = 0x0102030405060708u64;
        let abcd = [0x0102, 0x0304, 0x0506, 0x0708];
        assert_eq!(decode(DataType::U64, ByteOrder::Abcd, &abcd).unwrap(), Value::UInt(v));
        assert_eq!(
            decode(DataType::U64, ByteOrder::Cdab, &[0x0708, 0x0506, 0x0304, 0x0102]).unwrap(),
            Value::UInt(v)
        );
        assert_eq!(
            decode(DataType::U64, ByteOrder::Dcba, &[0x0807, 0x0605, 0x0403, 0x0201]).unwrap(),
            Value::UInt(v)
        );
        assert_eq!(
            decode(DataType::F64, ByteOrder::Abcd, &[0x4009, 0x21FB, 0x5444, 0x2D18]).unwrap(),
            Value::Float(std::f64::consts::PI)
        );
    }

    #[test]
    fn sixteen_bit_signed_and_byte_swap() {
        assert_eq!(decode(DataType::I16, ByteOrder::Abcd, &[0xFFFE]).unwrap(), Value::Int(-2));
        // Little-endian device: 0x0100 on the wire is 1.
        assert_eq!(decode(DataType::U16, ByteOrder::Dcba, &[0x0100]).unwrap(), Value::UInt(1));
        assert_eq!(decode(DataType::I16, ByteOrder::Badc, &[0xFEFF]).unwrap(), Value::Int(-2));
        // 16-bit values ignore word order.
        assert_eq!(decode(DataType::U16, ByteOrder::Cdab, &[7]).unwrap(), Value::UInt(7));
    }

    #[test]
    fn round_trip_every_type_and_order() {
        let samples: Vec<(DataType, Value)> = vec![
            (DataType::U16, Value::UInt(54321)),
            (DataType::I16, Value::Int(-12345)),
            (DataType::U32, Value::UInt(4_000_000_000)),
            (DataType::I32, Value::Int(-2_000_000_000)),
            (DataType::U64, Value::UInt(u64::MAX - 7)),
            (DataType::I64, Value::Int(i64::MIN + 3)),
            (DataType::F32, Value::Float(-0.15625)),
            (DataType::F64, Value::Float(1.0e-300)),
            (DataType::Str(4), Value::Text("Pump-7A".into())),
        ];
        for order in ByteOrder::ALL {
            for (dt, v) in &samples {
                let words = encode(*dt, order, v).unwrap();
                assert_eq!(words.len(), dt.width() as usize);
                assert_eq!(&decode(*dt, order, &words).unwrap(), v, "{dt:?} {order:?}");
            }
        }
    }

    #[test]
    fn bit_extract_and_merge() {
        assert_eq!(decode(DataType::Bit(3), ByteOrder::Abcd, &[0b1000]).unwrap(), Value::Bool(true));
        assert_eq!(decode(DataType::Bit(2), ByteOrder::Abcd, &[0b1000]).unwrap(), Value::Bool(false));
        assert_eq!(merge_bit(0b0001, 3, true), 0b1001);
        assert_eq!(merge_bit(0b1001, 0, false), 0b1000);
        assert_eq!(encode(DataType::Bit(4), ByteOrder::Abcd, &Value::Bool(true)).unwrap(), vec![0x10]);
    }

    #[test]
    fn string_padding_and_limits() {
        // "AB" "C\0" -> "ABC"
        assert_eq!(
            decode(DataType::Str(2), ByteOrder::Abcd, &[0x4142, 0x4300]).unwrap(),
            Value::Text("ABC".into())
        );
        assert_eq!(
            encode(DataType::Str(2), ByteOrder::Abcd, &Value::Text("ABC".into())).unwrap(),
            vec![0x4142, 0x4300]
        );
        assert!(encode(DataType::Str(1), ByteOrder::Abcd, &Value::Text("ABC".into())).is_err());
        assert!(encode(DataType::Str(2), ByteOrder::Abcd, &Value::Text("é".into())).is_err());
        // Byte-swapped device stores "BA" "\0C".
        assert_eq!(
            decode(DataType::Str(2), ByteOrder::Badc, &[0x4241, 0x0043]).unwrap(),
            Value::Text("ABC".into())
        );
    }

    #[test]
    fn range_and_width_errors() {
        assert!(encode(DataType::U16, ByteOrder::Abcd, &Value::Int(70000)).is_err());
        assert!(encode(DataType::U16, ByteOrder::Abcd, &Value::Int(-1)).is_err());
        assert!(encode(DataType::I32, ByteOrder::Abcd, &Value::Float(1.5)).is_err());
        assert_eq!(
            decode(DataType::U32, ByteOrder::Abcd, &[1]).unwrap_err(),
            CodecError::Width { want: 2, got: 1 }
        );
        assert!(decode(DataType::Bool, ByteOrder::Abcd, &[1]).is_err());
        // Whole-number floats are accepted for integer registers (point bus carries f64).
        assert_eq!(
            encode(DataType::U16, ByteOrder::Abcd, &Value::Float(42.0)).unwrap(),
            vec![42]
        );
    }

    #[test]
    fn type_names_round_trip() {
        for dt in ALL_TYPES {
            assert_eq!(DataType::parse(&dt.as_string()), Some(dt));
        }
        assert_eq!(DataType::parse("bit16"), None);
        assert_eq!(DataType::parse("str0"), None);
        assert_eq!(DataType::parse("float"), None);
        assert_eq!(ByteOrder::parse("CDAB"), Some(ByteOrder::Cdab));
        assert_eq!(ByteOrder::parse("zz"), None);
    }

    #[test]
    fn point_strings_parse_per_type() {
        assert_eq!(parse_point_value(DataType::F32, " 1.5 ").unwrap(), Value::Float(1.5));
        assert_eq!(parse_point_value(DataType::U16, "12").unwrap(), Value::Int(12));
        assert_eq!(parse_point_value(DataType::U16, "12.0").unwrap(), Value::Float(12.0));
        assert!(parse_point_value(DataType::U16, "12.5").is_err());
        assert_eq!(parse_point_value(DataType::Bool, "on").unwrap(), Value::Bool(true));
        assert_eq!(parse_point_value(DataType::Bit(1), "false").unwrap(), Value::Bool(false));
        assert!(parse_point_value(DataType::Bool, "maybe").is_err());
    }

    fn item(key: usize, unit: u8, area: Area, address: u16, width: u16) -> ReadItem {
        ReadItem { key, unit, area, address, width }
    }

    #[test]
    fn plan_merges_neighbours_and_splits_units_areas() {
        let items = [
            item(0, 1, Area::Holding, 10, 2),
            item(1, 1, Area::Holding, 12, 1),
            item(2, 1, Area::Holding, 15, 1), // gap of 2 -> merged at max_gap 4
            item(3, 1, Area::Holding, 100, 1), // far -> own block
            item(4, 2, Area::Holding, 10, 1), // other unit
            item(5, 1, Area::Input, 10, 1),    // other area
        ];
        let blocks = plan_reads(&items, 4);
        assert_eq!(blocks.len(), 4);
        let b = blocks.iter().find(|b| b.unit == 1 && b.area == Area::Holding && b.start == 10).unwrap();
        assert_eq!(b.count, 6);
        assert_eq!(b.items, vec![(0, 0), (1, 2), (2, 5)]);
        assert!(blocks.iter().any(|b| b.start == 100 && b.count == 1));
    }

    #[test]
    fn plan_respects_protocol_limit_and_zero_gap() {
        let mut items = Vec::new();
        for i in 0..130u16 {
            items.push(item(i as usize, 1, Area::Holding, i, 1));
        }
        let blocks = plan_reads(&items, 0);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].count, MAX_READ_REGS);
        assert_eq!(blocks[1].count, 5);
        // No gap allowed -> distinct registers 0 and 2 stay apart.
        let b = plan_reads(&[item(0, 1, Area::Coil, 0, 1), item(1, 1, Area::Coil, 2, 1)], 0);
        assert_eq!(b.len(), 2);
        // Overlapping items share a block without growing it.
        let b = plan_reads(&[item(0, 1, Area::Holding, 0, 4), item(1, 1, Area::Holding, 1, 1)], 0);
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].count, 4);
    }
}
