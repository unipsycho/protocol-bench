//! Text framing: delimited EOL / idle gap / transparent chunks.

#[derive(Debug, Clone)]
pub struct FrameConfig {
    pub mode: String,
    pub eol: String,
    pub eol_custom: String,
    pub idle_ms: u64,
}

impl Default for FrameConfig {
    fn default() -> Self {
        Self {
            mode: "delimited".into(),
            eol: "lf".into(),
            eol_custom: String::new(),
            idle_ms: 50,
        }
    }
}

impl FrameConfig {
    pub fn eol_bytes(&self) -> Vec<u8> {
        match self.eol.as_str() {
            "crlf" => b"\r\n".to_vec(),
            "cr" => b"\r".to_vec(),
            "custom" => self.eol_custom.as_bytes().to_vec(),
            "idle" | "none" => Vec::new(),
            "eol_idle" => {
                // Prefer LF when both; idle still applies in the reader.
                b"\n".to_vec()
            }
            _ => b"\n".to_vec(), // lf
        }
    }

    pub fn use_idle(&self) -> bool {
        matches!(self.eol.as_str(), "idle" | "eol_idle")
            || (self.mode != "transparent" && self.idle_ms > 0 && self.eol == "idle")
    }

    pub fn idle_timeout(&self) -> Option<std::time::Duration> {
        if self.mode == "transparent" {
            return None;
        }
        if matches!(self.eol.as_str(), "idle" | "eol_idle") || self.use_idle() {
            Some(std::time::Duration::from_millis(self.idle_ms.max(1)))
        } else {
            // Short read timeout so workers stay responsive.
            Some(std::time::Duration::from_millis(50))
        }
    }
}

/// Accumulate bytes and emit complete frames.
pub struct Framer {
    cfg: FrameConfig,
    buf: Vec<u8>,
    last_byte_at: Option<std::time::Instant>,
}

impl Framer {
    pub fn new(cfg: FrameConfig) -> Self {
        Self {
            cfg,
            buf: Vec::new(),
            last_byte_at: None,
        }
    }

    pub fn push(&mut self, data: &[u8]) -> Vec<String> {
        let mut out = Vec::new();
        if data.is_empty() {
            return out;
        }
        self.last_byte_at = Some(std::time::Instant::now());
        if self.cfg.mode == "transparent" {
            // Emit as UTF-8 chunks immediately.
            if let Ok(s) = std::str::from_utf8(data) {
                if !s.is_empty() {
                    out.push(s.to_string());
                }
            } else {
                out.push(String::from_utf8_lossy(data).into_owned());
            }
            return out;
        }
        self.buf.extend_from_slice(data);
        let eol = self.cfg.eol_bytes();
        if eol.is_empty() {
            return out;
        }
        while let Some(pos) = find_subslice(&self.buf, &eol) {
            let frame = self.buf.drain(..pos).collect::<Vec<u8>>();
            let _ = self.buf.drain(..eol.len()); // drop delimiter
            out.push(String::from_utf8_lossy(&frame).into_owned());
        }
        out
    }

    /// Flush a frame when idle gap elapsed (idle / eol_idle modes).
    pub fn tick_idle(&mut self) -> Option<String> {
        if self.cfg.mode == "transparent" || self.buf.is_empty() {
            return None;
        }
        if !matches!(self.cfg.eol.as_str(), "idle" | "eol_idle") {
            return None;
        }
        let idle = std::time::Duration::from_millis(self.cfg.idle_ms.max(1));
        let Some(at) = self.last_byte_at else {
            return None;
        };
        if at.elapsed() < idle {
            return None;
        }
        let frame = std::mem::take(&mut self.buf);
        self.last_byte_at = None;
        Some(String::from_utf8_lossy(&frame).into_owned())
    }
}

fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_lf() {
        let mut f = Framer::new(FrameConfig::default());
        let a = f.push(b"12.5,OK\n99");
        assert_eq!(a, vec!["12.5,OK".to_string()]);
        let b = f.push(b",X\n");
        assert_eq!(b, vec!["99,X".to_string()]);
    }
}
