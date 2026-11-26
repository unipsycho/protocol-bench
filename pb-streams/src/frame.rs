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
    /// Terminator bytes for the configured end-of-line mode.
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

    /// True when frames are also split on a silent gap.
    pub fn use_idle(&self) -> bool {
        matches!(self.eol.as_str(), "idle" | "eol_idle")
            || (self.mode != "transparent" && self.idle_ms > 0 && self.eol == "idle")
    }

    /// The idle gap, when idle splitting is on.
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

/// A delimiter that never comes (wrong EOL setting, binary device) must not grow the buffer
/// forever: past this many bytes the buffer is emitted as one frame.
pub const MAX_FRAME: usize = 64 * 1024;

/// Accumulate bytes and emit complete frames.
pub struct Framer {
    cfg: FrameConfig,
    buf: Vec<u8>,
    last_byte_at: Option<std::time::Instant>,
}

impl Framer {
    /// Framer for `cfg`.
    pub fn new(cfg: FrameConfig) -> Self {
        Self {
            cfg,
            buf: Vec::new(),
            last_byte_at: None,
        }
    }

    /// Feed received bytes; returns the complete frames now available (partial data is kept).
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
            let mut frame = self.buf.drain(..pos).collect::<Vec<u8>>();
            let _ = self.buf.drain(..eol.len()); // drop delimiter
            // LF mode fed by a CRLF device: the CR is part of the terminator, not the value.
            if eol == b"\n" && frame.last() == Some(&b'\r') {
                frame.pop();
            }
            out.push(String::from_utf8_lossy(&frame).into_owned());
        }
        if self.buf.len() > MAX_FRAME {
            out.push(String::from_utf8_lossy(&std::mem::take(&mut self.buf)).into_owned());
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

    fn cfg(mode: &str, eol: &str, idle_ms: u64) -> FrameConfig {
        FrameConfig { mode: mode.into(), eol: eol.into(), eol_custom: String::new(), idle_ms }
    }

    #[test]
    fn crlf_device_on_lf_endpoint_drops_the_cr() {
        let mut f = Framer::new(FrameConfig::default());
        assert_eq!(f.push(b"12.5\r\n13.0\r\n"), vec!["12.5", "13.0"]);
    }

    #[test]
    fn crlf_and_custom_terminators_split_across_reads() {
        let mut f = Framer::new(cfg("delimited", "crlf", 0));
        assert!(f.push(b"A\r").is_empty(), "half a terminator waits");
        assert_eq!(f.push(b"\nB\r\n"), vec!["A", "B"]);
        let mut f = Framer::new(FrameConfig { eol_custom: "<END>".into(), ..cfg("delimited", "custom", 0) });
        assert_eq!(f.push(b"x<EN"), Vec::<String>::new());
        assert_eq!(f.push(b"D>y<END>"), vec!["x", "y"]);
    }

    #[test]
    fn transparent_passes_chunks_through() {
        let mut f = Framer::new(cfg("transparent", "lf", 0));
        assert_eq!(f.push(b"no\nsplit"), vec!["no\nsplit"]);
        assert!(f.push(b"").is_empty());
        assert_eq!(f.push(&[0xff, b'a']), vec!["\u{fffd}a"], "invalid UTF-8 is replaced, not dropped");
    }

    #[test]
    fn idle_gap_flushes_a_partial_frame() {
        let mut f = Framer::new(cfg("delimited", "idle", 20));
        assert!(f.push(b"no terminator").is_empty());
        assert_eq!(f.tick_idle(), None, "gap not elapsed yet");
        std::thread::sleep(std::time::Duration::from_millis(40));
        assert_eq!(f.tick_idle().as_deref(), Some("no terminator"));
        assert_eq!(f.tick_idle(), None, "buffer emptied");
        // LF mode never flushes on idle.
        let mut f = Framer::new(cfg("delimited", "lf", 1));
        f.push(b"partial");
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert_eq!(f.tick_idle(), None);
    }

    #[test]
    fn missing_delimiter_cannot_grow_the_buffer_forever() {
        let mut f = Framer::new(FrameConfig::default());
        let chunk = vec![b'x'; 4096];
        let mut frames = Vec::new();
        for _ in 0..40 {
            frames.extend(f.push(&chunk));
        }
        assert!(!frames.is_empty(), "oversized garbage is flushed");
        assert!(f.buf.len() <= MAX_FRAME, "buffer bounded: {}", f.buf.len());
    }

    #[test]
    fn eol_bytes_and_idle_timeouts() {
        assert_eq!(cfg("delimited", "cr", 0).eol_bytes(), b"\r");
        assert!(cfg("delimited", "idle", 0).eol_bytes().is_empty());
        assert_eq!(cfg("delimited", "eol_idle", 0).eol_bytes(), b"\n");
        assert_eq!(cfg("transparent", "idle", 30).idle_timeout(), None);
        assert_eq!(cfg("delimited", "idle", 30).idle_timeout(), Some(std::time::Duration::from_millis(30)));
        assert_eq!(cfg("delimited", "lf", 30).idle_timeout(), Some(std::time::Duration::from_millis(50)), "short poll in LF mode");
    }

    #[test]
    fn splits_on_lf() {
        let mut f = Framer::new(FrameConfig::default());
        let a = f.push(b"12.5,OK\n99");
        assert_eq!(a, vec!["12.5,OK".to_string()]);
        let b = f.push(b",X\n");
        assert_eq!(b, vec!["99,X".to_string()]);
    }
}
