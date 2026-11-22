//! Minimal BER + SNMP v1 / v2c message codec. Pure functions, unit-tested against known bytes.
//! Covers GET requests / responses (and the agent side, for tests and a future agent).

use std::fmt;

pub const T_INTEGER: u8 = 0x02;
pub const T_OCTETS: u8 = 0x04;
pub const T_NULL: u8 = 0x05;
pub const T_OID: u8 = 0x06;
pub const T_SEQ: u8 = 0x30;
pub const T_IPADDR: u8 = 0x40;
pub const T_COUNTER32: u8 = 0x41;
pub const T_GAUGE32: u8 = 0x42;
pub const T_TICKS: u8 = 0x43;
pub const T_OPAQUE: u8 = 0x44;
pub const T_COUNTER64: u8 = 0x46;
pub const T_NO_SUCH_OBJECT: u8 = 0x80;
pub const T_NO_SUCH_INSTANCE: u8 = 0x81;
pub const T_END_OF_MIB: u8 = 0x82;
pub const T_GET: u8 = 0xA0;
pub const T_RESPONSE: u8 = 0xA2;

/// SNMP error-status codes we act on.
pub const ERR_TOO_BIG: i64 = 1;
pub const ERR_NO_SUCH_NAME: i64 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnmpError {
    Truncated,
    Bad(String),
}

impl fmt::Display for SnmpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => f.write_str("truncated SNMP message"),
            Self::Bad(m) => write!(f, "bad SNMP message: {m}"),
        }
    }
}

impl std::error::Error for SnmpError {}

/// A decoded varbind value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i64),
    /// Counter32 / Gauge32 / Counter64 / Unsigned.
    UInt(u64),
    /// TimeTicks (hundredths of a second, raw).
    Ticks(u64),
    Text(String),
    Oid(String),
    Ip(String),
    Null,
    /// noSuchObject / noSuchInstance / endOfMibView — the agent has no value here.
    Missing(&'static str),
}

// ---------------------------------------------------------------------------
// TLV primitives
// ---------------------------------------------------------------------------

fn put_len(out: &mut Vec<u8>, len: usize) {
    if len < 0x80 {
        out.push(len as u8);
    } else {
        let bytes = (len as u32).to_be_bytes();
        let skip = bytes.iter().take_while(|b| **b == 0).count();
        out.push(0x80 | (4 - skip) as u8);
        out.extend_from_slice(&bytes[skip..]);
    }
}

/// Encode one BER tag-length-value (definite length, short or long form).
pub fn tlv(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    put_len(&mut out, body.len());
    out.extend_from_slice(body);
    out
}

/// Read one TLV from `buf`: `(tag, body, rest)`.
pub fn read_tlv(buf: &[u8]) -> Result<(u8, &[u8], &[u8]), SnmpError> {
    let tag = *buf.first().ok_or(SnmpError::Truncated)?;
    let l0 = *buf.get(1).ok_or(SnmpError::Truncated)?;
    let (len, hdr) = if l0 < 0x80 {
        (l0 as usize, 2)
    } else {
        let n = (l0 & 0x7F) as usize;
        if n == 0 || n > 4 {
            return Err(SnmpError::Bad("unsupported length form".into()));
        }
        let bytes = buf.get(2..2 + n).ok_or(SnmpError::Truncated)?;
        (bytes.iter().fold(0usize, |a, b| (a << 8) | *b as usize), 2 + n)
    };
    let body = buf.get(hdr..hdr + len).ok_or(SnmpError::Truncated)?;
    Ok((tag, body, &buf[hdr + len..]))
}

/// BER INTEGER body: minimal two's-complement bytes.
pub fn encode_int(v: i64) -> Vec<u8> {
    let mut bytes = v.to_be_bytes().to_vec();
    // Strip redundant leading sign bytes.
    while bytes.len() > 1
        && ((bytes[0] == 0x00 && bytes[1] & 0x80 == 0) || (bytes[0] == 0xFF && bytes[1] & 0x80 != 0))
    {
        bytes.remove(0);
    }
    tlv(T_INTEGER, &bytes)
}

fn decode_int(body: &[u8]) -> Result<i64, SnmpError> {
    if body.is_empty() || body.len() > 8 {
        return Err(SnmpError::Bad("integer length".into()));
    }
    let mut v: i64 = if body[0] & 0x80 != 0 { -1 } else { 0 };
    for b in body {
        v = (v << 8) | *b as i64;
    }
    Ok(v)
}

fn decode_uint(body: &[u8]) -> Result<u64, SnmpError> {
    let body = if body.first() == Some(&0) && body.len() > 1 { &body[1..] } else { body };
    if body.is_empty() || body.len() > 8 {
        return Err(SnmpError::Bad("unsigned length".into()));
    }
    Ok(body.iter().fold(0u64, |a, b| (a << 8) | *b as u64))
}

/// `"1.3.6.1.2.1.1.1.0"` (leading dot allowed) → OID TLV.
pub fn encode_oid(oid: &str) -> Result<Vec<u8>, SnmpError> {
    let parts: Result<Vec<u64>, _> = oid.trim().trim_start_matches('.').split('.').map(|p| p.parse::<u64>()).collect();
    let parts = parts.map_err(|_| SnmpError::Bad(format!("invalid OID \"{oid}\"")))?;
    if parts.len() < 2 || parts[0] > 2 || (parts[0] < 2 && parts[1] >= 40) {
        return Err(SnmpError::Bad(format!("invalid OID \"{oid}\"")));
    }
    let mut body = Vec::new();
    for (i, part) in std::iter::once(parts[0] * 40 + parts[1]).chain(parts[2..].iter().copied()).enumerate() {
        let _ = i;
        let mut groups = vec![(part & 0x7F) as u8];
        let mut n = part >> 7;
        while n > 0 {
            groups.push((n & 0x7F) as u8 | 0x80);
            n >>= 7;
        }
        groups.reverse();
        body.extend(groups);
    }
    Ok(tlv(T_OID, &body))
}

/// Decode an OBJECT IDENTIFIER body to dotted form (`1.3.6.1…`).
pub fn decode_oid(body: &[u8]) -> Result<String, SnmpError> {
    if body.is_empty() {
        return Err(SnmpError::Bad("empty OID".into()));
    }
    let mut arcs: Vec<u64> = Vec::new();
    let mut cur: u64 = 0;
    for (i, b) in body.iter().enumerate() {
        cur = cur.checked_mul(128).ok_or_else(|| SnmpError::Bad("OID arc overflow".into()))? | (*b & 0x7F) as u64;
        if b & 0x80 == 0 {
            if arcs.is_empty() {
                let (a, b2) = if cur < 80 { (cur / 40, cur % 40) } else { (2, cur - 80) };
                arcs.push(a);
                arcs.push(b2);
            } else {
                arcs.push(cur);
            }
            cur = 0;
        } else if i == body.len() - 1 {
            return Err(SnmpError::Truncated);
        }
    }
    Ok(arcs.iter().map(|a| a.to_string()).collect::<Vec<_>>().join("."))
}

/// Canonical OID text (no leading dot) — how responses report names.
pub fn normalize_oid(oid: &str) -> String {
    oid.trim().trim_start_matches('.').to_string()
}

/// Decode a value by its BER tag into [`Value`]; unknown tags are an error, never a panic.
pub fn decode_value(tag: u8, body: &[u8]) -> Result<Value, SnmpError> {
    Ok(match tag {
        T_INTEGER => Value::Int(decode_int(body)?),
        T_OCTETS | T_OPAQUE => match std::str::from_utf8(body) {
            Ok(t) if !t.chars().any(|c| c.is_control() && c != '\n' && c != '\r' && c != '\t') => Value::Text(t.to_string()),
            _ => Value::Text(body.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":")),
        },
        T_NULL => Value::Null,
        T_OID => Value::Oid(decode_oid(body)?),
        T_IPADDR if body.len() == 4 => Value::Ip(format!("{}.{}.{}.{}", body[0], body[1], body[2], body[3])),
        T_COUNTER32 | T_GAUGE32 | T_COUNTER64 => Value::UInt(decode_uint(body)?),
        T_TICKS => Value::Ticks(decode_uint(body)?),
        T_NO_SUCH_OBJECT => Value::Missing("noSuchObject"),
        T_NO_SUCH_INSTANCE => Value::Missing("noSuchInstance"),
        T_END_OF_MIB => Value::Missing("endOfMibView"),
        other => return Err(SnmpError::Bad(format!("unsupported value tag {other:#04x}"))),
    })
}

fn encode_value(v: &Value) -> Vec<u8> {
    match v {
        Value::Int(i) => encode_int(*i),
        Value::UInt(u) | Value::Ticks(u) => {
            let tag = if matches!(v, Value::Ticks(_)) { T_TICKS } else { T_GAUGE32 };
            let mut b = u.to_be_bytes().to_vec();
            while b.len() > 1 && b[0] == 0 {
                b.remove(0);
            }
            if b[0] & 0x80 != 0 {
                b.insert(0, 0);
            }
            tlv(tag, &b)
        }
        Value::Text(t) => tlv(T_OCTETS, t.as_bytes()),
        Value::Oid(o) => encode_oid(o).unwrap_or_else(|_| tlv(T_NULL, &[])),
        Value::Ip(ip) => {
            let o: Vec<u8> = ip.split('.').filter_map(|p| p.parse().ok()).collect();
            tlv(T_IPADDR, &o)
        }
        Value::Null => tlv(T_NULL, &[]),
        Value::Missing("noSuchObject") => tlv(T_NO_SUCH_OBJECT, &[]),
        Value::Missing("endOfMibView") => tlv(T_END_OF_MIB, &[]),
        Value::Missing(_) => tlv(T_NO_SUCH_INSTANCE, &[]),
    }
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

/// `0` = SNMPv1, `1` = SNMPv2c.
pub fn encode_get(version: i64, community: &str, request_id: i32, oids: &[String]) -> Result<Vec<u8>, SnmpError> {
    let mut vbs = Vec::new();
    for o in oids {
        let mut vb = encode_oid(o)?;
        vb.extend(tlv(T_NULL, &[]));
        vbs.extend(tlv(T_SEQ, &vb));
    }
    let mut pdu = encode_int(request_id as i64);
    pdu.extend(encode_int(0));
    pdu.extend(encode_int(0));
    pdu.extend(tlv(T_SEQ, &vbs));
    let mut msg = encode_int(version);
    msg.extend(tlv(T_OCTETS, community.as_bytes()));
    msg.extend(tlv(T_GET, &pdu));
    Ok(tlv(T_SEQ, &msg))
}

#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    pub request_id: i64,
    pub error_status: i64,
    pub error_index: i64,
    pub varbinds: Vec<(String, Value)>,
}

fn decode_pdu(tag_expected: u8, buf: &[u8]) -> Result<(i64, String, i64, i64, Vec<(String, Value)>), SnmpError> {
    let (t, msg, _) = read_tlv(buf)?;
    if t != T_SEQ {
        return Err(SnmpError::Bad("not a SEQUENCE".into()));
    }
    let (t, ver, rest) = read_tlv(msg)?;
    if t != T_INTEGER {
        return Err(SnmpError::Bad("missing version".into()));
    }
    let version = decode_int(ver)?;
    let (t, community, rest) = read_tlv(rest)?;
    if t != T_OCTETS {
        return Err(SnmpError::Bad("missing community".into()));
    }
    let (ptag, pdu, _) = read_tlv(rest)?;
    if ptag != tag_expected {
        return Err(SnmpError::Bad(format!("unexpected PDU {ptag:#04x}")));
    }
    let (_, rid, rest) = read_tlv(pdu)?;
    let (_, es, rest) = read_tlv(rest)?;
    let (_, ei, rest) = read_tlv(rest)?;
    let (t, vbl, _) = read_tlv(rest)?;
    if t != T_SEQ {
        return Err(SnmpError::Bad("missing varbind list".into()));
    }
    let mut varbinds = Vec::new();
    let mut cur = vbl;
    while !cur.is_empty() {
        let (_, vb, next) = read_tlv(cur)?;
        let (t, name, rest) = read_tlv(vb)?;
        if t != T_OID {
            return Err(SnmpError::Bad("varbind name is not an OID".into()));
        }
        let (vt, vbody, _) = read_tlv(rest)?;
        varbinds.push((decode_oid(name)?, decode_value(vt, vbody)?));
        cur = next;
    }
    let _ = version;
    Ok((decode_int(rid)?, String::from_utf8_lossy(community).into_owned(), decode_int(es)?, decode_int(ei)?, varbinds))
}

/// Parse a whole SNMP v1/v2c response message.
pub fn decode_response(buf: &[u8]) -> Result<Response, SnmpError> {
    let (request_id, _, error_status, error_index, varbinds) = decode_pdu(T_RESPONSE, buf)?;
    Ok(Response { request_id, error_status, error_index, varbinds })
}

/// Agent side: decode a GET request into `(version, community, request_id, oids)`.
pub fn decode_get_request(buf: &[u8]) -> Result<(i64, String, i64, Vec<String>), SnmpError> {
    let (rid, community, _, _, vbs) = decode_pdu(T_GET, buf)?;
    let (_, msg, _) = read_tlv(buf)?;
    let (_, ver, _) = read_tlv(msg)?;
    Ok((decode_int(ver)?, community, rid, vbs.into_iter().map(|(o, _)| o).collect()))
}

/// Agent side: encode a response.
pub fn encode_response(version: i64, community: &str, request_id: i64, error_status: i64, error_index: i64, vbs: &[(String, Value)]) -> Vec<u8> {
    let mut list = Vec::new();
    for (o, v) in vbs {
        let mut vb = encode_oid(o).unwrap_or_default();
        vb.extend(encode_value(v));
        list.extend(tlv(T_SEQ, &vb));
    }
    let mut pdu = encode_int(request_id);
    pdu.extend(encode_int(error_status));
    pdu.extend(encode_int(error_index));
    pdu.extend(tlv(T_SEQ, &list));
    let mut msg = encode_int(version);
    msg.extend(tlv(T_OCTETS, community.as_bytes()));
    msg.extend(tlv(T_RESPONSE, &pdu));
    tlv(T_SEQ, &msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oid_known_encoding_and_round_trip() {
        let e = encode_oid("1.3.6.1.2.1.1.1.0").unwrap();
        assert_eq!(e, vec![0x06, 0x08, 0x2B, 0x06, 0x01, 0x02, 0x01, 0x01, 0x01, 0x00]);
        assert_eq!(decode_oid(&e[2..]).unwrap(), "1.3.6.1.2.1.1.1.0");
        // Multi-byte arcs (enterprises 8072 -> 0xBF 0x08? actually 8072 = 0x3F 0x08 groups) and leading dot.
        let big = encode_oid(".1.3.6.1.4.1.8072.3.2.10").unwrap();
        assert_eq!(decode_oid(&big[2..]).unwrap(), "1.3.6.1.4.1.8072.3.2.10");
        assert_eq!(&big[2..9], &[0x2B, 0x06, 0x01, 0x04, 0x01, 0xBF, 0x08]);
        assert!(encode_oid("1").is_err());
        assert!(encode_oid("3.1.2").is_err());
        assert!(encode_oid("1.40.2").is_err());
        assert!(encode_oid("1.3.x").is_err());
    }

    #[test]
    fn integers_negative_and_boundaries() {
        for v in [0i64, 1, 127, 128, 255, 256, -1, -128, -129, 32767, -32768, i32::MAX as i64, i32::MIN as i64, i64::MAX, i64::MIN] {
            let e = encode_int(v);
            assert_eq!(decode_int(&e[2..]).unwrap(), v, "{v}");
        }
        assert_eq!(encode_int(128), vec![0x02, 0x02, 0x00, 0x80]);
        assert_eq!(encode_int(-129), vec![0x02, 0x02, 0xFF, 0x7F]);
        assert_eq!(encode_int(0), vec![0x02, 0x01, 0x00]);
    }

    #[test]
    fn long_form_lengths() {
        let body = vec![0xAB; 300];
        let t = tlv(T_OCTETS, &body);
        assert_eq!(&t[..4], &[0x04, 0x82, 0x01, 0x2C]);
        let (tag, b, rest) = read_tlv(&t).unwrap();
        assert_eq!((tag, b.len(), rest.len()), (T_OCTETS, 300, 0));
        assert_eq!(read_tlv(&t[..10]), Err(SnmpError::Truncated));
        assert!(read_tlv(&[0x04, 0x85, 0, 0, 0, 0, 1]).is_err());
    }

    #[test]
    fn get_request_matches_known_v1_bytes() {
        let req = encode_get(0, "public", 1, &["1.3.6.1.2.1.1.1.0".to_string()]).unwrap();
        // v1 GET sysDescr.0, request id 1 — lengths: varbind 12, list 14, PDU 25, message 38.
        let known: Vec<u8> = vec![
            0x30, 0x26, 0x02, 0x01, 0x00, 0x04, 0x06, b'p', b'u', b'b', b'l', b'i', b'c', 0xA0, 0x19, 0x02, 0x01, 0x01, 0x02, 0x01,
            0x00, 0x02, 0x01, 0x00, 0x30, 0x0E, 0x30, 0x0C, 0x06, 0x08, 0x2B, 0x06, 0x01, 0x02, 0x01, 0x01, 0x01, 0x00, 0x05, 0x00,
        ];
        assert_eq!(req, known);
        let (ver, comm, rid, oids) = decode_get_request(&req).unwrap();
        assert_eq!((ver, comm.as_str(), rid, oids), (0, "public", 1, vec!["1.3.6.1.2.1.1.1.0".to_string()]));
    }

    #[test]
    fn response_round_trip_every_value_type() {
        let vbs = vec![
            ("1.3.6.1.2.1.1.1.0".to_string(), Value::Text("Linux box".into())),
            ("1.3.6.1.2.1.1.3.0".to_string(), Value::Ticks(123_456)),
            ("1.3.6.1.2.1.2.2.1.10.1".to_string(), Value::UInt(4_000_000_000)),
            ("1.3.6.1.2.1.2.2.1.5.1".to_string(), Value::UInt(u64::MAX)),
            ("1.3.6.1.4.1.9.1".to_string(), Value::Int(-42)),
            ("1.3.6.1.2.1.4.20.1.1".to_string(), Value::Ip("10.0.0.7".into())),
            ("1.3.6.1.2.1.1.2.0".to_string(), Value::Oid("1.3.6.1.4.1.8072.3.2.10".into())),
            ("1.3.6.1.2.1.9.9".to_string(), Value::Missing("noSuchInstance")),
            ("1.3.6.1.2.1.9.8".to_string(), Value::Missing("noSuchObject")),
        ];
        let bytes = encode_response(1, "public", 77, 0, 0, &vbs);
        let r = decode_response(&bytes).unwrap();
        assert_eq!(r.request_id, 77);
        assert_eq!(r.varbinds, vbs);
    }

    #[test]
    fn counter64_and_non_utf8_octets_decode() {
        assert_eq!(decode_value(T_COUNTER64, &[0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]).unwrap(), Value::UInt(u64::MAX));
        assert_eq!(decode_value(T_OCTETS, &[0x00, 0x1A, 0x2B, 0x3C, 0x4D, 0x5E]).unwrap(), Value::Text("00:1a:2b:3c:4d:5e".into()));
        assert_eq!(decode_value(T_OCTETS, b"plain text").unwrap(), Value::Text("plain text".into()));
        assert!(decode_value(0x77, &[]).is_err());
    }

    #[test]
    fn garbage_and_wrong_pdu_are_errors_not_panics() {
        assert!(decode_response(&[]).is_err());
        assert!(decode_response(&[0x30, 0x03, 0x02, 0x01]).is_err());
        let req = encode_get(1, "x", 1, &["1.3.6.1".to_string()]).unwrap();
        assert!(decode_response(&req).is_err(), "a request is not a response");
        for cut in 0..req.len() {
            let _ = decode_get_request(&req[..cut]); // never panics
        }
        assert_eq!(normalize_oid(" .1.3.6 "), "1.3.6");
    }
}
