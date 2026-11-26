//! Length-prefixed JSON frames (u32 LE length + UTF-8 body).

use std::io::{Read, Write};

pub fn write_json<W: Write, T: serde::Serialize>(w: &mut W, value: &T) -> Result<(), String> {
    let body = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    if body.len() > u32::MAX as usize {
        return Err("IPC frame too large".into());
    }
    // One write per frame: a separate 4-byte header write lets Nagle hold the body back
    // until the peer's delayed ACK, which costs ~40 ms per request on localhost.
    let mut frame = Vec::with_capacity(4 + body.len());
    frame.extend_from_slice(&(body.len() as u32).to_le_bytes());
    frame.extend_from_slice(&body);
    w.write_all(&frame).map_err(|e| e.to_string())?;
    w.flush().map_err(|e| e.to_string())
}

pub fn read_json<R: Read, T: serde::de::DeserializeOwned>(r: &mut R) -> Result<T, String> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf).map_err(|e| e.to_string())?;
    let len = u32::from_le_bytes(len_buf) as usize;
    // Hard cap ~32 MiB — snapshots must stay under this.
    if len > 32 * 1024 * 1024 {
        return Err(format!("IPC frame too large: {len} bytes"));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).map_err(|e| e.to_string())?;
    serde_json::from_slice(&body).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn frames_roundtrip_back_to_back() {
        let mut buf = Vec::new();
        write_json(&mut buf, &serde_json::json!({"a": 1})).unwrap();
        write_json(&mut buf, &"second").unwrap();
        write_json(&mut buf, &vec![1u8, 2, 3]).unwrap();
        let mut r = Cursor::new(buf);
        let a: serde_json::Value = read_json(&mut r).unwrap();
        let b: String = read_json(&mut r).unwrap();
        let c: Vec<u8> = read_json(&mut r).unwrap();
        assert_eq!(a["a"], 1);
        assert_eq!(b, "second");
        assert_eq!(c, vec![1, 2, 3]);
        // Clean end of stream is an error the server treats as "peer closed".
        assert!(read_json::<_, serde_json::Value>(&mut r).is_err());
    }

    #[test]
    fn header_is_little_endian_body_length() {
        let mut buf = Vec::new();
        write_json(&mut buf, &"hi").unwrap();
        assert_eq!(&buf[..4], &4u32.to_le_bytes(), "\"hi\" is 4 bytes of JSON");
        assert_eq!(&buf[4..], b"\"hi\"");
    }

    #[test]
    fn oversized_length_is_refused_before_allocating() {
        let mut buf = (64u32 * 1024 * 1024).to_le_bytes().to_vec();
        buf.extend_from_slice(b"{}");
        let err = read_json::<_, serde_json::Value>(&mut Cursor::new(buf)).unwrap_err();
        assert!(err.contains("too large"), "{err}");
    }

    #[test]
    fn truncated_and_garbage_frames_fail() {
        let mut short = 10u32.to_le_bytes().to_vec();
        short.extend_from_slice(b"{\"a\"");
        assert!(read_json::<_, serde_json::Value>(&mut Cursor::new(short)).is_err(), "body shorter than header");
        let mut junk = 3u32.to_le_bytes().to_vec();
        junk.extend_from_slice(b"{{{");
        assert!(read_json::<_, serde_json::Value>(&mut Cursor::new(junk)).is_err(), "not JSON");
        assert!(read_json::<_, serde_json::Value>(&mut Cursor::new(vec![1u8, 0])).is_err(), "half a header");
    }

    /// Counts `write` calls: header and body must leave in one write (Nagle / delayed-ACK stall).
    struct CountingWriter {
        writes: usize,
        bytes: Vec<u8>,
    }
    impl Write for CountingWriter {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            self.writes += 1;
            self.bytes.extend_from_slice(data);
            Ok(data.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn one_write_per_frame() {
        let mut w = CountingWriter { writes: 0, bytes: Vec::new() };
        write_json(&mut w, &serde_json::json!({"method": "status", "id": 7})).unwrap();
        assert_eq!(w.writes, 1);
        let back: serde_json::Value = read_json(&mut Cursor::new(w.bytes)).unwrap();
        assert_eq!(back["id"], 7);
    }
}
