//! Sparkplug B payload codec (hand-written protobuf for the fields Protocol Bench uses) and
//! topic helpers. Spec: Eclipse Sparkplug 3.0, `sparkplug_b.proto`.
//!
//! Supported metric values: integers (Int8..UInt64), Float, Double, Boolean, String / Text,
//! DateTime. DataSets, Templates, properties and metadata are skipped when decoding.

/// Sparkplug data types (subset).
pub mod dt {
    pub const INT8: u32 = 1;
    pub const INT16: u32 = 2;
    pub const INT32: u32 = 3;
    pub const INT64: u32 = 4;
    pub const UINT8: u32 = 5;
    pub const UINT16: u32 = 6;
    pub const UINT32: u32 = 7;
    pub const UINT64: u32 = 8;
    pub const FLOAT: u32 = 9;
    pub const DOUBLE: u32 = 10;
    pub const BOOLEAN: u32 = 11;
    pub const STRING: u32 = 12;
    pub const DATETIME: u32 = 13;
    pub const TEXT: u32 = 14;
}

/// A metric value.
#[derive(Debug, Clone, PartialEq)]
pub enum MValue {
    Int(i64),
    UInt(u64),
    Float(f32),
    Double(f64),
    Bool(bool),
    Text(String),
    Null,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Metric {
    pub name: Option<String>,
    pub alias: Option<u64>,
    pub timestamp: Option<u64>,
    pub datatype: u32,
    pub value: MValue,
}

impl Metric {
    pub fn new(name: &str, datatype: u32, value: MValue) -> Self {
        Self { name: Some(name.to_string()), alias: None, timestamp: None, datatype, value }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Payload {
    pub timestamp: Option<u64>,
    pub metrics: Vec<Metric>,
    pub seq: Option<u64>,
}

// ---- protobuf primitives --------------------------------------------------------------------

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

fn put_key(out: &mut Vec<u8>, field: u32, wire: u8) {
    put_varint(out, ((field as u64) << 3) | wire as u64);
}

fn put_bytes(out: &mut Vec<u8>, field: u32, b: &[u8]) {
    put_key(out, field, 2);
    put_varint(out, b.len() as u64);
    out.extend_from_slice(b);
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Reader<'a> {
    fn done(&self) -> bool {
        self.i >= self.b.len()
    }
    fn varint(&mut self) -> Result<u64, String> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = *self.b.get(self.i).ok_or("truncated varint")?;
            self.i += 1;
            v |= ((byte & 0x7f) as u64) << shift;
            if byte & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err("varint too long".into())
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.i.checked_add(n).filter(|e| *e <= self.b.len()).ok_or("truncated field")?;
        let s = &self.b[self.i..end];
        self.i = end;
        Ok(s)
    }
    /// (field, wire type)
    fn key(&mut self) -> Result<(u32, u8), String> {
        let k = self.varint()?;
        Ok(((k >> 3) as u32, (k & 7) as u8))
    }
    fn skip(&mut self, wire: u8) -> Result<(), String> {
        match wire {
            0 => self.varint().map(|_| ()),
            1 => self.take(8).map(|_| ()),
            2 => {
                let n = self.varint()? as usize;
                self.take(n).map(|_| ())
            }
            5 => self.take(4).map(|_| ()),
            w => Err(format!("unsupported wire type {w}")),
        }
    }
}

// ---- encode -----------------------------------------------------------------------------------

fn encode_metric(m: &Metric) -> Vec<u8> {
    let mut o = Vec::new();
    if let Some(n) = &m.name {
        put_bytes(&mut o, 1, n.as_bytes());
    }
    if let Some(a) = m.alias {
        put_key(&mut o, 2, 0);
        put_varint(&mut o, a);
    }
    if let Some(t) = m.timestamp {
        put_key(&mut o, 3, 0);
        put_varint(&mut o, t);
    }
    put_key(&mut o, 4, 0);
    put_varint(&mut o, m.datatype as u64);
    match &m.value {
        MValue::Null => {
            put_key(&mut o, 7, 0);
            put_varint(&mut o, 1);
        }
        MValue::Int(i) => {
            if matches!(m.datatype, dt::INT64 | dt::UINT64 | dt::DATETIME) {
                put_key(&mut o, 11, 0);
                put_varint(&mut o, *i as u64);
            } else {
                put_key(&mut o, 10, 0);
                put_varint(&mut o, (*i as i32 as u32) as u64);
            }
        }
        MValue::UInt(u) => {
            if matches!(m.datatype, dt::INT64 | dt::UINT64 | dt::DATETIME) {
                put_key(&mut o, 11, 0);
                put_varint(&mut o, *u);
            } else {
                put_key(&mut o, 10, 0);
                put_varint(&mut o, *u as u32 as u64);
            }
        }
        MValue::Float(f) => {
            put_key(&mut o, 12, 5);
            o.extend_from_slice(&f.to_le_bytes());
        }
        MValue::Double(d) => {
            put_key(&mut o, 13, 1);
            o.extend_from_slice(&d.to_le_bytes());
        }
        MValue::Bool(b) => {
            put_key(&mut o, 14, 0);
            put_varint(&mut o, *b as u64);
        }
        MValue::Text(s) => put_bytes(&mut o, 15, s.as_bytes()),
    }
    o
}

pub fn encode(p: &Payload) -> Vec<u8> {
    let mut o = Vec::new();
    if let Some(t) = p.timestamp {
        put_key(&mut o, 1, 0);
        put_varint(&mut o, t);
    }
    for m in &p.metrics {
        put_bytes(&mut o, 2, &encode_metric(m));
    }
    if let Some(s) = p.seq {
        put_key(&mut o, 3, 0);
        put_varint(&mut o, s);
    }
    o
}

// ---- decode -----------------------------------------------------------------------------------

fn decode_metric(b: &[u8]) -> Result<Metric, String> {
    let mut r = Reader { b, i: 0 };
    let mut m = Metric { name: None, alias: None, timestamp: None, datatype: 0, value: MValue::Null };
    let mut raw_int: Option<u64> = None;
    let mut raw_long: Option<u64> = None;
    while !r.done() {
        let (f, w) = r.key()?;
        match (f, w) {
            (1, 2) => {
                let n = r.varint()? as usize;
                m.name = Some(String::from_utf8_lossy(r.take(n)?).into_owned());
            }
            (2, 0) => m.alias = Some(r.varint()?),
            (3, 0) => m.timestamp = Some(r.varint()?),
            (4, 0) => m.datatype = r.varint()? as u32,
            (7, 0) => {
                if r.varint()? != 0 {
                    m.value = MValue::Null;
                }
            }
            (10, 0) => raw_int = Some(r.varint()?),
            (11, 0) => raw_long = Some(r.varint()?),
            (12, 5) => m.value = MValue::Float(f32::from_le_bytes(r.take(4)?.try_into().unwrap())),
            (13, 1) => m.value = MValue::Double(f64::from_le_bytes(r.take(8)?.try_into().unwrap())),
            (14, 0) => m.value = MValue::Bool(r.varint()? != 0),
            (15, 2) => {
                let n = r.varint()? as usize;
                m.value = MValue::Text(String::from_utf8_lossy(r.take(n)?).into_owned());
            }
            (_, w) => r.skip(w)?,
        }
    }
    if let Some(v) = raw_int {
        let v32 = v as u32;
        m.value = match m.datatype {
            dt::INT8 => MValue::Int(v32 as u8 as i8 as i64),
            dt::INT16 => MValue::Int(v32 as u16 as i16 as i64),
            dt::INT32 => MValue::Int(v32 as i32 as i64),
            _ => MValue::UInt(v32 as u64),
        };
    }
    if let Some(v) = raw_long {
        m.value = match m.datatype {
            dt::INT64 => MValue::Int(v as i64),
            _ => MValue::UInt(v),
        };
    }
    Ok(m)
}

pub fn decode(b: &[u8]) -> Result<Payload, String> {
    let mut r = Reader { b, i: 0 };
    let mut p = Payload::default();
    while !r.done() {
        let (f, w) = r.key()?;
        match (f, w) {
            (1, 0) => p.timestamp = Some(r.varint()?),
            (2, 2) => {
                let n = r.varint()? as usize;
                p.metrics.push(decode_metric(r.take(n)?)?);
            }
            (3, 0) => p.seq = Some(r.varint()?),
            (_, w) => r.skip(w)?,
        }
    }
    Ok(p)
}

// ---- values from host text ----------------------------------------------------------------------

/// Infer a datatype + value from host point text (`true` → Boolean, `12` → Int64,
/// `1.5` → Double, else String).
pub fn from_text(t: &str) -> (u32, MValue) {
    let s = t.trim();
    match s.to_ascii_lowercase().as_str() {
        "true" => return (dt::BOOLEAN, MValue::Bool(true)),
        "false" => return (dt::BOOLEAN, MValue::Bool(false)),
        _ => {}
    }
    if let Ok(i) = s.parse::<i64>() {
        return (dt::INT64, MValue::Int(i));
    }
    if let Ok(f) = s.parse::<f64>() {
        if f.is_finite() {
            return (dt::DOUBLE, MValue::Double(f));
        }
    }
    (dt::STRING, MValue::Text(t.to_string()))
}

/// Host point text → value of a fixed datatype (for DATA after BIRTH fixed the type).
pub fn text_as(datatype: u32, t: &str) -> MValue {
    let s = t.trim();
    match datatype {
        dt::BOOLEAN => MValue::Bool(matches!(s.to_ascii_lowercase().as_str(), "true" | "1" | "on")),
        dt::FLOAT => s.parse::<f32>().map(MValue::Float).unwrap_or(MValue::Null),
        dt::DOUBLE => s.parse::<f64>().map(MValue::Double).unwrap_or(MValue::Null),
        dt::INT8 | dt::INT16 | dt::INT32 | dt::INT64 => {
            s.parse::<f64>().map(|f| MValue::Int(f.round() as i64)).unwrap_or(MValue::Null)
        }
        dt::UINT8 | dt::UINT16 | dt::UINT32 | dt::UINT64 | dt::DATETIME => {
            s.parse::<f64>().map(|f| MValue::UInt(f.max(0.0).round() as u64)).unwrap_or(MValue::Null)
        }
        _ => MValue::Text(t.to_string()),
    }
}

// ---- topics -------------------------------------------------------------------------------------

/// A parsed Sparkplug topic `spBv1.0/<group>/<type>/<edge>[/<device>]`.
#[derive(Debug, Clone, PartialEq)]
pub struct Topic {
    pub group: String,
    pub kind: String,
    pub edge: String,
    pub device: String,
}

pub fn parse_topic(t: &str) -> Option<Topic> {
    let parts: Vec<&str> = t.split('/').collect();
    if parts.first() != Some(&"spBv1.0") || parts.len() < 4 || parts[1] == "STATE" {
        return None;
    }
    Some(Topic {
        group: parts[1].into(),
        kind: parts[2].into(),
        edge: parts[3].into(),
        device: parts.get(4).map(|s| s.to_string()).unwrap_or_default(),
    })
}

pub fn topic(group: &str, kind: &str, edge: &str, device: &str) -> String {
    if device.is_empty() {
        format!("spBv1.0/{group}/{kind}/{edge}")
    } else {
        format!("spBv1.0/{group}/{kind}/{edge}/{device}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_round_trips_every_type() {
        let p = Payload {
            timestamp: Some(1_700_000_000_000),
            seq: Some(7),
            metrics: vec![
                Metric::new("bdSeq", dt::UINT64, MValue::UInt(3)),
                Metric::new("Pressure", dt::DOUBLE, MValue::Double(3412.5)),
                Metric::new("Temp", dt::FLOAT, MValue::Float(21.25)),
                Metric::new("Neg16", dt::INT16, MValue::Int(-5)),
                Metric::new("Neg32", dt::INT32, MValue::Int(-70000)),
                Metric::new("Big", dt::INT64, MValue::Int(-1)),
                Metric::new("Run", dt::BOOLEAN, MValue::Bool(true)),
                Metric::new("Mode", dt::STRING, MValue::Text("AUTO".into())),
                Metric { name: None, alias: Some(4), timestamp: Some(5), datatype: dt::UINT16, value: MValue::UInt(65535) },
                Metric::new("Gone", dt::DOUBLE, MValue::Null),
            ],
        };
        let back = decode(&encode(&p)).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn known_bytes_and_garbage() {
        // timestamp=1, seq=2: 08 01 18 02
        let p = Payload { timestamp: Some(1), seq: Some(2), metrics: vec![] };
        assert_eq!(encode(&p), vec![0x08, 0x01, 0x18, 0x02]);
        // Unknown field 5 (uuid string) is skipped.
        let mut b = encode(&p);
        b.extend_from_slice(&[0x22, 0x02, b'h', b'i']);
        assert_eq!(decode(&b).unwrap(), p);
        assert!(decode(&[0x12, 0x10, 0x01]).is_err());
        assert!(decode(&[0xff, 0xff]).is_err());
    }

    #[test]
    fn text_values_and_topics() {
        assert_eq!(from_text("true"), (dt::BOOLEAN, MValue::Bool(true)));
        assert_eq!(from_text("12"), (dt::INT64, MValue::Int(12)));
        assert_eq!(from_text("1.5"), (dt::DOUBLE, MValue::Double(1.5)));
        assert_eq!(from_text("x"), (dt::STRING, MValue::Text("x".into())));
        assert_eq!(text_as(dt::FLOAT, "2.5"), MValue::Float(2.5));
        assert_eq!(text_as(dt::INT32, "2.6"), MValue::Int(3));
        assert_eq!(text_as(dt::DOUBLE, "x"), MValue::Null);
        let t = parse_topic("spBv1.0/Plant/DDATA/Line2/Robot1").unwrap();
        assert_eq!((t.group.as_str(), t.kind.as_str(), t.edge.as_str(), t.device.as_str()), ("Plant", "DDATA", "Line2", "Robot1"));
        assert_eq!(parse_topic("spBv1.0/STATE/host"), None);
        assert_eq!(parse_topic("other/x/y/z"), None);
        assert_eq!(topic("G", "NBIRTH", "E", ""), "spBv1.0/G/NBIRTH/E");
    }
}
