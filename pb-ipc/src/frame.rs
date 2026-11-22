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
