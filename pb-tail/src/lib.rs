//! Byte-offset file tailing (shared by the File follow and LAS packs).
//!
//! Only **complete lines** (ending in `\n`) are returned and the offset advances only past them,
//! so a half-written row is picked up whole on a later poll and a row is never read twice. The
//! offset (plus the file identity) can be saved and restored across restarts; truncation or
//! replacement (log rotation) restarts from byte 0 and tells the caller to re-read the header.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Bytes read per poll at most (a bigger backlog drains over several polls).
const CHUNK: usize = 1 << 20;

/// What survives a restart.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TailState {
    /// Path the offset belongs to.
    #[serde(default)]
    pub path: String,
    /// Bytes consumed (always on a line boundary).
    #[serde(default)]
    pub offset: u64,
    /// File identity (inode on Unix) so a replaced file is not mistaken for the old one.
    #[serde(default)]
    pub ino: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TailError {
    NotFound,
    Io(String),
}

impl std::fmt::Display for TailError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => f.write_str("file not found"),
            Self::Io(m) => f.write_str(m),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Batch {
    /// New complete lines, `\r` stripped, empty lines dropped.
    pub lines: Vec<String>,
    /// The file was truncated or replaced: `lines` start at byte 0, so a CSV header (if any) is
    /// the first of them.
    pub reset: bool,
    /// A single line exceeded the read chunk and was skipped.
    pub skipped_oversize: bool,
}

pub struct Tail {
    path: PathBuf,
    follow_rotation: bool,
    pub state: TailState,
    /// True until the first successful poll after (re)start with `offset == 0`.
    seen_file: bool,
}

#[cfg(unix)]
fn identity(meta: &std::fs::Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    Some(meta.ino())
}

#[cfg(not(unix))]
fn identity(_meta: &std::fs::Metadata) -> Option<u64> {
    None
}

impl Tail {
    /// Begin following `path`. `saved` resumes a previous run when it still matches the file;
    /// otherwise `from_end` picks the start (skip existing rows vs. read them).
    pub fn start(path: &Path, saved: Option<TailState>, from_end: bool, follow_rotation: bool) -> Self {
        let key = path.to_string_lossy().to_string();
        let mut state = TailState { path: key.clone(), offset: 0, ino: None };
        let mut seen_file = false;
        if let Ok(meta) = std::fs::metadata(path) {
            seen_file = true;
            let ino = identity(&meta);
            state.ino = ino;
            match saved {
                Some(s) if s.path == key && s.offset <= meta.len() && (s.ino.is_none() || s.ino == ino) => {
                    state.offset = s.offset;
                }
                _ => {
                    if from_end {
                        state.offset = meta.len();
                    }
                }
            }
            // Never resume mid-line: if the byte before the offset is not a newline, rewind to it.
            if state.offset > 0 {
                state.offset = line_boundary_at_or_before(path, state.offset);
            }
        }
        Self { path: path.to_path_buf(), follow_rotation, state, seen_file }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Read whatever complete lines were appended since the last call.
    pub fn poll(&mut self) -> Result<Batch, TailError> {
        let meta = match std::fs::metadata(&self.path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Gone (rotated away): the next file that appears is read from its start.
                self.state.offset = 0;
                self.state.ino = None;
                self.seen_file = false;
                return Err(TailError::NotFound);
            }
            Err(e) => return Err(TailError::Io(e.to_string())),
        };
        let ino = identity(&meta);
        let mut reset = false;
        if !self.seen_file {
            // First sight of a file that was missing at start: it is new, read it all.
            self.seen_file = true;
            self.state.offset = 0;
            self.state.ino = ino;
            reset = true;
        } else if self.follow_rotation
            && (meta.len() < self.state.offset
                || (self.state.ino.is_some() && ino.is_some() && self.state.ino != ino))
        {
            self.state.offset = 0;
            self.state.ino = ino;
            reset = true;
        } else if meta.len() < self.state.offset {
            return Err(TailError::Io("file shrank and rotation following is off".into()));
        }
        self.state.ino = ino;
        let mut batch = Batch { reset, ..Default::default() };
        if meta.len() == self.state.offset {
            return Ok(batch);
        }
        let mut f = File::open(&self.path).map_err(|e| TailError::Io(e.to_string()))?;
        f.seek(SeekFrom::Start(self.state.offset)).map_err(|e| TailError::Io(e.to_string()))?;
        let want = ((meta.len() - self.state.offset) as usize).min(CHUNK);
        let mut buf = vec![0u8; want];
        let mut got = 0;
        while got < want {
            match f.read(&mut buf[got..]) {
                Ok(0) => break,
                Ok(n) => got += n,
                Err(e) => return Err(TailError::Io(e.to_string())),
            }
        }
        buf.truncate(got);
        match buf.iter().rposition(|b| *b == b'\n') {
            Some(last) => {
                for raw in buf[..last].split(|b| *b == b'\n') {
                    let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
                    if !raw.is_empty() {
                        batch.lines.push(String::from_utf8_lossy(raw).into_owned());
                    }
                }
                self.state.offset += last as u64 + 1;
            }
            None if got == CHUNK => {
                // A "line" longer than the chunk: skip it rather than stall forever.
                batch.skipped_oversize = true;
                self.state.offset += got as u64;
            }
            None => {} // partial line — wait for its newline
        }
        Ok(batch)
    }
}

/// Offset of the start of the line containing byte `offset - 1`'s successor: i.e. the largest
/// line start `<= offset`. Cheap check of the previous byte; scans back only when needed.
fn line_boundary_at_or_before(path: &Path, offset: u64) -> u64 {
    let Ok(mut f) = File::open(path) else { return 0 };
    let back = offset.min(64 * 1024);
    let from = offset - back;
    if f.seek(SeekFrom::Start(from)).is_err() {
        return 0;
    }
    let mut buf = vec![0u8; back as usize];
    if f.read_exact(&mut buf).is_err() {
        return 0;
    }
    match buf.iter().rposition(|b| *b == b'\n') {
        Some(i) => from + i as u64 + 1,
        None if from == 0 => 0,
        None => offset, // no newline in the window: leave it
    }
}

/// First line of the file (CSV header), or `None` when the file is empty / has no full line yet.
pub fn first_line(path: &Path) -> Option<String> {
    let mut f = File::open(path).ok()?;
    let mut buf = vec![0u8; 64 * 1024];
    let n = f.read(&mut buf).ok()?;
    let end = buf[..n].iter().position(|b| *b == b'\n')?;
    let raw = &buf[..end];
    let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
    (!raw.is_empty()).then(|| String::from_utf8_lossy(raw).into_owned())
}

/// Last complete line of the file (looks at the final 64 KiB), or `None` when there is none.
pub fn last_line(path: &Path) -> Option<String> {
    let mut f = File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let from = len.saturating_sub(64 * 1024);
    f.seek(SeekFrom::Start(from)).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    // Only complete lines count: cut at the last newline, then take the line before it.
    let end = buf.iter().rposition(|b| *b == b'\n')?;
    let body = &buf[..end];
    let start = body.iter().rposition(|b| *b == b'\n').map(|i| i + 1).unwrap_or(0);
    if start == 0 && from > 0 {
        return None; // the only line in the window started before it
    }
    let raw = &body[start..];
    let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
    (!raw.is_empty()).then(|| String::from_utf8_lossy(raw).into_owned())
}

pub fn load_state(path: &Path) -> Option<TailState> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

pub fn save_state(path: &Path, state: &TailState) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Write-then-rename so a crash never leaves a half-written state file.
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_vec(state).unwrap_or_default())?;
    std::fs::rename(tmp, path)
}

/// Scratch directories for tests (also used by dependent crates' tests).
pub mod testutil {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    static N: AtomicU32 = AtomicU32::new(0);

    /// A fresh empty directory under the system temp dir.
    pub fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pb-files-{}-{}-{tag}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Append `s` to `path`, creating it when missing (simulates a producer writing the file).
    pub fn append(path: &std::path::Path, s: &str) {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
        f.write_all(s.as_bytes()).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::{append, dir};
    use super::*;

    #[test]
    fn reads_only_new_complete_lines_never_twice() {
        let d = dir("basic");
        let p = d.join("a.log");
        append(&p, "h1,h2\n1,2\n");
        let mut t = Tail::start(&p, None, false, true);
        assert_eq!(t.poll().unwrap().lines, vec!["h1,h2", "1,2"]);
        assert!(t.poll().unwrap().lines.is_empty());
        append(&p, "3,4\r\n5,6\n");
        assert_eq!(t.poll().unwrap().lines, vec!["3,4", "5,6"]);
        assert!(t.poll().unwrap().lines.is_empty());
    }

    #[test]
    fn partial_line_waits_for_its_newline() {
        let d = dir("partial");
        let p = d.join("a.log");
        append(&p, "one\ntw");
        let mut t = Tail::start(&p, None, false, true);
        assert_eq!(t.poll().unwrap().lines, vec!["one"]);
        assert!(t.poll().unwrap().lines.is_empty());
        append(&p, "o\nthree");
        assert_eq!(t.poll().unwrap().lines, vec!["two"]);
        append(&p, "\n");
        assert_eq!(t.poll().unwrap().lines, vec!["three"]);
    }

    #[test]
    fn from_end_skips_existing_rows() {
        let d = dir("end");
        let p = d.join("a.log");
        append(&p, "old1\nold2\n");
        let mut t = Tail::start(&p, None, true, true);
        assert!(t.poll().unwrap().lines.is_empty());
        append(&p, "new\n");
        assert_eq!(t.poll().unwrap().lines, vec!["new"]);
    }

    #[test]
    fn truncation_restarts_and_flags_reset() {
        let d = dir("trunc");
        let p = d.join("a.log");
        append(&p, "h\n1\n2\n3\n");
        let mut t = Tail::start(&p, None, false, true);
        t.poll().unwrap();
        std::fs::write(&p, "h\nA\n").unwrap();
        let b = t.poll().unwrap();
        assert!(b.reset);
        assert_eq!(b.lines, vec!["h", "A"]);
        assert!(!t.poll().unwrap().reset);
    }

    #[cfg(unix)]
    #[test]
    fn replaced_file_with_more_data_is_detected_by_identity() {
        let d = dir("rotate");
        let p = d.join("a.log");
        append(&p, "h\n1\n");
        let mut t = Tail::start(&p, None, false, true);
        t.poll().unwrap();
        // logrotate style: move away, create a new (longer) file at the same path.
        std::fs::rename(&p, d.join("a.log.1")).unwrap();
        append(&p, "h\nX\nY\nZ\nW\n");
        let b = t.poll().unwrap();
        assert!(b.reset, "new inode must reset");
        assert_eq!(b.lines, vec!["h", "X", "Y", "Z", "W"]);
    }

    #[test]
    fn missing_then_appearing_file_is_read_from_the_start() {
        let d = dir("missing");
        let p = d.join("later.log");
        let mut t = Tail::start(&p, None, true, true);
        assert_eq!(t.poll(), Err(TailError::NotFound));
        append(&p, "h\n1\n");
        let b = t.poll().unwrap();
        assert!(b.reset);
        assert_eq!(b.lines, vec!["h", "1"]);
        // And it disappearing again is reported, then recovers.
        std::fs::remove_file(&p).unwrap();
        assert_eq!(t.poll(), Err(TailError::NotFound));
        append(&p, "h\n9\n");
        assert_eq!(t.poll().unwrap().lines, vec!["h", "9"]);
    }

    #[test]
    fn saved_state_resumes_without_rereading_and_catches_up_missed_rows() {
        let d = dir("resume");
        let p = d.join("a.log");
        let st = d.join("state.json");
        append(&p, "h\n1\n2\n");
        let mut t = Tail::start(&p, None, false, true);
        assert_eq!(t.poll().unwrap().lines.len(), 3);
        save_state(&st, &t.state).unwrap();
        drop(t);
        // Rows written while the service was down.
        append(&p, "3\n4\n");
        let mut t2 = Tail::start(&p, load_state(&st), true, true);
        assert_eq!(t2.poll().unwrap().lines, vec!["3", "4"], "resume must catch up, not skip, not repeat");
        assert!(t2.poll().unwrap().lines.is_empty());
    }

    #[test]
    fn stale_state_for_another_file_or_shrunk_file_is_ignored() {
        let d = dir("stale");
        let p = d.join("a.log");
        append(&p, "a\nb\nc\n");
        let other = TailState { path: "/somewhere/else.log".into(), offset: 4, ino: None };
        let mut t = Tail::start(&p, Some(other), false, true);
        assert_eq!(t.poll().unwrap().lines, vec!["a", "b", "c"]);
        let past_end = TailState { path: p.to_string_lossy().into(), offset: 9999, ino: None };
        let mut t = Tail::start(&p, Some(past_end), false, true);
        assert_eq!(t.poll().unwrap().lines.len(), 3);
    }

    #[test]
    fn resume_mid_line_rewinds_to_the_line_start() {
        let d = dir("midline");
        let p = d.join("a.log");
        append(&p, "alpha\nbravo\ncharlie\n");
        let st = TailState { path: p.to_string_lossy().into(), offset: 9, ino: None }; // inside "bravo"
        let mut t = Tail::start(&p, Some(st), false, true);
        assert_eq!(t.poll().unwrap().lines, vec!["bravo", "charlie"]);
    }

    #[test]
    fn rotation_off_reports_shrink_instead_of_restarting() {
        let d = dir("norot");
        let p = d.join("a.log");
        append(&p, "1\n2\n3\n");
        let mut t = Tail::start(&p, None, false, false);
        t.poll().unwrap();
        std::fs::write(&p, "x\n").unwrap();
        assert!(matches!(t.poll(), Err(TailError::Io(_))));
    }

    #[test]
    fn last_line_is_the_last_complete_line() {
        let d = dir("lastline");
        let p = d.join("a.log");
        assert_eq!(last_line(&p), None);
        append(&p, "h\n1\n2\r\n3");
        assert_eq!(last_line(&p).as_deref(), Some("2"), "a partial last line is ignored");
        append(&p, "\n");
        assert_eq!(last_line(&p).as_deref(), Some("3"));
        std::fs::write(&p, "only\n").unwrap();
        assert_eq!(last_line(&p).as_deref(), Some("only"));
    }

    #[test]
    fn first_line_and_state_file_roundtrip() {
        let d = dir("first");
        let p = d.join("a.csv");
        assert_eq!(first_line(&p), None);
        append(&p, "time,temp\r\n1,2\n");
        assert_eq!(first_line(&p).as_deref(), Some("time,temp"));
        let st = d.join("sub/dir/state.json");
        let s = TailState { path: "x".into(), offset: 42, ino: Some(7) };
        save_state(&st, &s).unwrap();
        assert_eq!(load_state(&st), Some(s));
        assert_eq!(load_state(&d.join("nope.json")), None);
    }
}
