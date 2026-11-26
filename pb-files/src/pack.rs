//! C-ABI pack: one worker thread per followed file.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use pb_abi::worker::{sanitize_id, sleep_stop, update_status};
use pb_abi::{HostRef, Pack, PB_LOG_DEBUG, PB_LOG_ERROR, PB_LOG_INFO, PB_QUALITY_BAD, PB_QUALITY_GOOD};
use serde::{Deserialize, Serialize};

use crate::header::{auto_settled, head_lines, layout, lines_before, HeaderLayout, HeaderMode, SCAN_LINES};
use crate::parse::{column_index, csv_columns, json_cell, json_leaves, json_path, split_csv_line, type_text, Cell};
use crate::tail::{last_line, load_state, save_state, Tail, TailError};

const PACK_ID: &[u8] = b"files\0";
const PACK_TITLE: &[u8] = b"File follow\0";

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PackConfig {
    /// Where per-source offset files live (empty = offsets are not persisted).
    #[serde(default)]
    state_dir: String,
    #[serde(default)]
    sources: Vec<SourceJson>,
    #[serde(default)]
    inputs: Vec<InputJson>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct SourceJson {
    id: String,
    #[serde(default)]
    path: String,
    /// `csv` | `jsonl`.
    #[serde(default = "default_parser")]
    parser: String,
    /// CSV header: `"auto"` (smart detection), `false` (none) or a 1-based names line.
    #[serde(default)]
    header_row: HeaderMode,
    #[serde(default = "default_delim")]
    delimiter: String,
    #[serde(default = "default_poll")]
    poll_ms: u32,
    /// `beginning` | `end`.
    #[serde(default = "default_start")]
    start_at: String,
    #[serde(default = "yes")]
    follow_rotation: bool,
    #[serde(default = "yes")]
    enabled: bool,
}

fn default_parser() -> String {
    "csv".into()
}
fn yes() -> bool {
    true
}
fn default_delim() -> String {
    ",".into()
}
fn default_poll() -> u32 {
    500
}
fn default_start() -> String {
    "end".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct BindJson {
    /// CSV header name / 0-based index, or JSON-lines dotted path.
    column: String,
    #[serde(default)]
    point: String,
    #[serde(default)]
    source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct InputJson {
    source: String,
    binds: Vec<BindJson>,
}

// ---------------------------------------------------------------------------
// Host
// ---------------------------------------------------------------------------

/// Convert a parsed cell into a host point.
trait PutCell {
    fn put(&self, name: &str, source: &str, cell: &Cell, quality: i32);
}

impl PutCell for HostRef {
    fn put(&self, name: &str, source: &str, cell: &Cell, quality: i32) {
        match cell {
            Cell::Bool(b) => self.upsert_bool(name, source, *b, quality),
            Cell::Int(i) => self.upsert_i64(name, source, *i, quality),
            Cell::Float(x) => self.upsert_f64(name, source, *x, quality),
            Cell::Text(t) => self.upsert_text(name, source, t, quality),
        }
    }
}

// ---------------------------------------------------------------------------
// Worker
// ---------------------------------------------------------------------------

#[derive(Default, Clone)]
struct Status {
    state: String,
    error: String,
    connected: bool,
    rows: u64,
    bad_rows: u64,
    offset: u64,
    /// `(name, sample)` for the UI column picker: header names / JSON paths + the latest row.
    columns: Vec<(String, String)>,
    /// CSV header layout summary for the UI (`3 preamble lines · names on line 4 · …`).
    header: String,
    /// Preamble lines above the header (first few, for the UI).
    preamble: Vec<String>,
}

const PREAMBLE_SHOWN: usize = 24;

fn header_status(s: &mut Status, l: &HeaderLayout, mode: HeaderMode, settled: bool) {
    let tag = match mode {
        HeaderMode::Auto if settled => "auto",
        HeaderMode::Auto => "auto, waiting for data",
        HeaderMode::None => "none",
        HeaderMode::Line(_) => "fixed",
    };
    s.header = format!("{} ({tag})", l.describe());
    s.preamble = l.preamble.iter().take(PREAMBLE_SHOWN).map(|p| p.chars().take(200).collect()).collect();
}

/// Header resolution for a CSV source while it is followed.
struct HeaderTrack {
    mode: HeaderMode,
    delim: char,
    /// Settled layout; `None` while Auto is still collecting evidence.
    layout: Option<HeaderLayout>,
    /// Index (non-blank lines from the top) of the next streamed line, when known.
    line_no: Option<usize>,
    /// Lines held back while the header is unsettled: `(line_no, text)`.
    pending: Vec<(Option<usize>, String)>,
}

impl HeaderTrack {
    fn names(&self) -> Option<&[String]> {
        self.layout.as_ref().and_then(|l| l.names.as_deref())
    }

    /// Re-read the top of the file and try to settle. Returns the layout that was looked at.
    fn resolve(&mut self, path: &std::path::Path, idle: bool) -> HeaderLayout {
        let head = head_lines(path, SCAN_LINES);
        let l = layout(&head, self.delim, self.mode);
        let settled = match self.mode {
            HeaderMode::Auto => auto_settled(&l, head.len(), idle),
            HeaderMode::None => true,
            HeaderMode::Line(n) => head.len() >= n as usize,
        };
        if settled {
            self.layout = Some(l.clone());
        }
        l
    }

    /// Is this streamed line part of the preamble / header (never published)?
    fn is_header_line(&self, line_no: Option<usize>) -> bool {
        match (line_no, &self.layout) {
            (Some(i), Some(l)) => i < l.data_start,
            _ => false,
        }
    }
}

struct BindRt {
    column: String,
    name: String,
    source: String,
    last: Option<Cell>,
    warned_missing: bool,
}

struct Worker {
    id: String,
    key: String,
    path: String,
    stop: Arc<AtomicBool>,
    status: Arc<Mutex<Status>>,
    join: Option<JoinHandle<()>>,
}

impl Worker {
    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

struct RunCtx {
    host: HostRef,
    src: SourceJson,
    binds: Vec<BindRt>,
    state_path: Option<PathBuf>,
    stop: Arc<AtomicBool>,
    status: Arc<Mutex<Status>>,
}

fn run_source(mut c: RunCtx) {
    let path = PathBuf::from(c.src.path.trim());
    let csv = !c.src.parser.eq_ignore_ascii_case("jsonl");
    let delim = c.src.delimiter.chars().next().unwrap_or(',');
    let saved = c.state_path.as_deref().and_then(load_state);
    let mut tail = Tail::start(&path, saved, c.src.start_at.eq_ignore_ascii_case("end"), c.src.follow_rotation);
    let mut hdr = HeaderTrack {
        mode: if csv { c.src.header_row } else { HeaderMode::None },
        delim,
        layout: None,
        line_no: if tail.state.offset == 0 { Some(0) } else { lines_before(&path, tail.state.offset) },
        pending: Vec::new(),
    };
    if csv {
        // Resumed / started mid-file: the header is read from the top of the file.
        let l = hdr.resolve(&path, false);
        let settled = hdr.layout.is_some();
        update_status(&c.status, |s| header_status(s, &l, hdr.mode, settled));
    }
    let mut missing = false;
    let mut saved_offset = tail.state.offset;
    update_status(&c.status, |s| s.offset = tail.state.offset);
    let poll = Duration::from_millis(c.src.poll_ms.clamp(20, 600_000) as u64);

    while !c.stop.load(Ordering::SeqCst) {
        match tail.poll() {
            Ok(batch) => {
                if missing {
                    missing = false;
                    c.host.log(PB_LOG_INFO, &format!("files {}: {} is back", c.src.id, path.display()));
                }
                if batch.reset {
                    hdr.layout = None;
                    hdr.line_no = Some(0);
                    hdr.pending.clear();
                }
                let mut rows = 0u64;
                let mut last_cols: Option<Vec<(String, String)>> = None;
                let mut lines: Vec<(Option<usize>, String)> = Vec::with_capacity(batch.lines.len());
                for line in batch.lines {
                    lines.push((hdr.line_no, line));
                    hdr.line_no = hdr.line_no.map(|n| n + 1);
                }
                if csv && hdr.layout.is_none() {
                    let idle = lines.is_empty();
                    hdr.pending.append(&mut lines);
                    let l = hdr.resolve(&path, idle);
                    let settled = hdr.layout.is_some();
                    update_status(&c.status, |s| {
                        header_status(s, &l, hdr.mode, settled);
                        if !settled && s.columns.is_empty() {
                            if let Some(n) = l.names.as_deref() {
                                s.columns = csv_columns(Some(n), &[]);
                            }
                        }
                    });
                    if settled {
                        lines = std::mem::take(&mut hdr.pending);
                        if let Some(n) = hdr.names() {
                            last_cols = Some(csv_columns(Some(n), &[]));
                        }
                    }
                }
                for (line_no, line) in &lines {
                    if csv {
                        if hdr.is_header_line(*line_no) {
                            continue;
                        }
                        let cols = split_csv_line(line, delim);
                        let header = hdr.names();
                        rows += 1;
                        last_cols = Some(csv_columns(header, &cols));
                        for b in &mut c.binds {
                            match column_index(header, &b.column).and_then(|i| cols.get(i)) {
                                Some(cell) => publish(&c.host, b, type_text(cell)),
                                None if !b.warned_missing => {
                                    b.warned_missing = true;
                                    let msg = format!("column \"{}\" not found in row / header", b.column);
                                    c.host.log(PB_LOG_ERROR, &format!("files {}: {msg}", c.src.id));
                                    update_status(&c.status, |s| s.error = msg);
                                }
                                None => {}
                            }
                        }
                    } else {
                        match serde_json::from_str::<serde_json::Value>(line) {
                            Ok(v) => {
                                rows += 1;
                                last_cols = Some(json_leaves(&v, 60));
                                for b in &mut c.binds {
                                    match json_path(&v, &b.column) {
                                        Some(j) => publish(&c.host, b, json_cell(j)),
                                        None if !b.warned_missing => {
                                            b.warned_missing = true;
                                            let msg = format!("path \"{}\" not found in a row", b.column);
                                            c.host.log(PB_LOG_ERROR, &format!("files {}: {msg}", c.src.id));
                                            update_status(&c.status, |s| s.error = msg);
                                        }
                                        None => {}
                                    }
                                }
                            }
                            Err(e) => {
                                update_status(&c.status, |s| {
                                    s.bad_rows += 1;
                                    s.error = format!("invalid JSON line skipped: {e}");
                                });
                            }
                        }
                    }
                }
                if batch.skipped_oversize {
                    update_status(&c.status, |s| s.error = "a line longer than 1 MiB was skipped".into());
                }
                if rows > 0 {
                    c.host.log(PB_LOG_DEBUG, &format!("files {}: +{rows} row(s)", c.src.id));
                }
                if tail.state.offset != saved_offset {
                    if let Some(sp) = &c.state_path {
                        if let Err(e) = save_state(sp, &tail.state) {
                            c.host.log(PB_LOG_ERROR, &format!("files {}: cannot save offset: {e}", c.src.id));
                        }
                    }
                    saved_offset = tail.state.offset;
                }
                // A CSV header seen with no data row yet still gives the picker its names.
                if last_cols.is_none() && csv {
                    if let Some(h) = hdr.names() {
                        if c.status.lock().map(|s| s.columns.is_empty()).unwrap_or(false) {
                            last_cols = Some(csv_columns(Some(h), &[]));
                        }
                    }
                }
                update_status(&c.status, |s| {
                    if let Some(cols) = last_cols.take() {
                        s.columns = cols;
                    }
                    s.rows += rows;
                    s.offset = tail.state.offset;
                    s.connected = true;
                    s.state = "following".into();
                    if s.error.starts_with("file not found") || s.error.starts_with("shrank") {
                        s.error.clear();
                    }
                });
            }
            Err(TailError::NotFound) => {
                if !missing {
                    missing = true;
                    for b in &c.binds {
                        if let Some(last) = &b.last {
                            c.host.put(&b.name, &b.source, last, PB_QUALITY_BAD);
                        } else {
                            c.host.put(&b.name, &b.source, &Cell::Text(String::new()), PB_QUALITY_BAD);
                        }
                    }
                    c.host.log(PB_LOG_ERROR, &format!("files {}: {} not found", c.src.id, path.display()));
                }
                update_status(&c.status, |s| {
                    s.connected = false;
                    s.state = "waiting for file".into();
                    s.error = format!("file not found: {}", path.display());
                });
            }
            Err(TailError::Io(e)) => {
                update_status(&c.status, |s| {
                    s.connected = false;
                    s.state = "error".into();
                    s.error = e.clone();
                });
            }
        }
        sleep_stop(&c.stop, poll);
    }
    update_status(&c.status, |s| {
        s.connected = false;
        s.state = "stopped".into();
    });
}

/// Binds-free source: report the file's columns (header + latest row) without consuming rows,
/// saving offsets or publishing points.
fn run_preview(c: RunCtx) {
    let path = PathBuf::from(c.src.path.trim());
    let csv = !c.src.parser.eq_ignore_ascii_case("jsonl");
    let delim = c.src.delimiter.chars().next().unwrap_or(',');
    while !c.stop.load(Ordering::SeqCst) {
        let head = if csv { head_lines(&path, SCAN_LINES) } else { Vec::new() };
        let lay = layout(&head, delim, c.src.header_row);
        let header = lay.names.as_deref();
        let last = last_line(&path);
        let columns = match (&last, csv) {
            // No data rows yet: the "last line" is part of the header / preamble.
            (Some(_), true) if lay.data_rows == 0 => header.map(|h| csv_columns(Some(h), &[])).unwrap_or_default(),
            (Some(l), true) => csv_columns(header, &split_csv_line(l, delim)),
            (Some(l), false) => serde_json::from_str::<serde_json::Value>(l).map(|v| json_leaves(&v, 60)).unwrap_or_default(),
            (None, true) => header.map(|h| csv_columns(Some(h), &[])).unwrap_or_default(),
            (None, false) => Vec::new(),
        };
        let exists = path.exists();
        let settled = match c.src.header_row {
            HeaderMode::Auto => auto_settled(&lay, head.len(), true),
            _ => true,
        };
        update_status(&c.status, |s| {
            if csv {
                header_status(s, &lay, c.src.header_row, settled);
            }
            s.columns = columns;
            s.connected = exists;
            s.state = if exists { "preview (no binds)".into() } else { "waiting for file".into() };
            s.error = if exists { String::new() } else { format!("file not found: {}", path.display()) };
        });
        sleep_stop(&c.stop, Duration::from_millis(1500));
    }
    update_status(&c.status, |s| {
        s.connected = false;
        s.state = "stopped".into();
    });
}

fn publish(host: &HostRef, b: &mut BindRt, cell: Cell) {
    host.put(&b.name, &b.source, &cell, PB_QUALITY_GOOD);
    b.last = Some(cell);
}

// ---------------------------------------------------------------------------
// Pack state
// ---------------------------------------------------------------------------

struct PackState {
    host: HostRef,
    config: PackConfig,
    workers: Vec<Worker>,
    enabled: bool,
    config_error: String,
}

impl PackState {

    fn stop_all(&mut self) {
        for w in &mut self.workers {
            w.stop.store(true, Ordering::SeqCst);
        }
        for w in &mut self.workers {
            w.shutdown();
        }
        self.workers.clear();
    }

    fn configure_json(&mut self, text: &str) -> Result<(), String> {
        self.config = serde_json::from_str(text).map_err(|e| format!("json: {e}"))?;
        self.sync_workers();
        Ok(())
    }

    fn sync_workers(&mut self) {
        self.config_error.clear();
        let mut old = std::mem::take(&mut self.workers);
        let mut keep = Vec::new();
        let mut spawn: Vec<(SourceJson, Vec<BindJson>, String)> = Vec::new();
        if self.enabled {
            for src in self.config.sources.iter().filter(|s| s.enabled) {
                let binds: Vec<BindJson> = self
                    .config
                    .inputs
                    .iter()
                    .filter(|i| i.source == src.id)
                    .flat_map(|i| i.binds.clone())
                    .filter(|b| !b.column.trim().is_empty())
                    .collect();
                // No binds yet: a preview worker still reports the columns for the picker.
                let key = serde_json::to_string(&(src, &binds, &self.config.state_dir)).unwrap_or_default();
                if let Some(pos) = old.iter().position(|w| w.id == src.id && w.key == key) {
                    keep.push(old.remove(pos));
                } else {
                    spawn.push((src.clone(), binds, key));
                }
            }
        }
        for w in &mut old {
            w.stop.store(true, Ordering::SeqCst);
        }
        for w in &mut old {
            w.shutdown();
        }
        for (src, binds, key) in spawn {
            match self.spawn_worker(src, binds, key) {
                Ok(w) => keep.push(w),
                Err(e) => {
                    self.host.log(PB_LOG_ERROR, &format!("files: {e}"));
                    self.config_error = e;
                }
            }
        }
        self.workers = keep;
        self.host.log(PB_LOG_INFO, &format!("files configured: {} source(s)", self.workers.len()));
    }

    fn spawn_worker(&self, src: SourceJson, binds: Vec<BindJson>, key: String) -> Result<Worker, String> {
        if src.path.trim().is_empty() {
            return Err(format!("{}: no file path set", src.id));
        }
        if src.delimiter.chars().count() != 1 && src.parser.eq_ignore_ascii_case("csv") {
            return Err(format!("{}: CSV delimiter must be one character", src.id));
        }
        let preview = binds.is_empty();
        let rts: Vec<BindRt> = binds
            .iter()
            .map(|b| {
                let source = if b.source.is_empty() { format!("file/{}::{}", src.id, b.column) } else { b.source.clone() };
                let name = if b.point.trim().is_empty() { source.clone() } else { b.point.trim().to_string() };
                BindRt { column: b.column.clone(), name, source, last: None, warned_missing: false }
            })
            .collect();
        let state_path = (!self.config.state_dir.trim().is_empty())
            .then(|| PathBuf::from(&self.config.state_dir).join(format!("{}.json", sanitize_id(&src.id))));
        let stop = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new(Status { state: "starting".into(), ..Default::default() }));
        let (id, path) = (src.id.clone(), src.path.clone());
        let ctx = RunCtx { host: self.host, src, binds: rts, state_path, stop: stop.clone(), status: status.clone() };
        let join = std::thread::Builder::new()
            .name(format!("pb-files-{id}"))
            .spawn(move || if preview { run_preview(ctx) } else { run_source(ctx) })
            .map_err(|e| format!("spawn: {e}"))?;
        Ok(Worker { id, key, path, stop, status, join: Some(join) })
    }

    fn statuses(&self) -> Vec<(String, String, Status)> {
        self.workers
            .iter()
            .map(|w| (w.id.clone(), w.path.clone(), w.status.lock().map(|s| s.clone()).unwrap_or_default()))
            .collect()
    }



}

impl Drop for PackState {
    fn drop(&mut self) {
        self.stop_all();
    }
}

// ---------------------------------------------------------------------------
// C ABI
// ---------------------------------------------------------------------------

impl Pack for PackState {
    const ID: &'static [u8] = PACK_ID;
    const TITLE: &'static [u8] = PACK_TITLE;

    fn new(host: HostRef) -> Self {
        Self { host, config: PackConfig::default(), workers: Vec::new(), enabled: true, config_error: String::new() }
    }

    fn configure(&mut self, json: &str) -> Result<(), String> {
        self.configure_json(json)
    }

    fn config_failed(&mut self, err: String) {
        self.host.log(PB_LOG_ERROR, &err);
        self.config_error = err;
    }

    fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
        self.sync_workers();
    }

    fn shutdown(&mut self) {
        self.stop_all();
    }

    fn state_label(&self) -> String {
        if !self.config_error.is_empty() {
            return "config error".into();
        }
        let st = self.statuses();
        if st.is_empty() {
            return if self.enabled { "idle".into() } else { "stopped".into() };
        }
        format!("{}/{} following", st.iter().filter(|(_, _, s)| s.connected).count(), st.len())
    }

    fn last_error(&self) -> String {
        if !self.config_error.is_empty() {
            return self.config_error.clone();
        }
        self.statuses()
            .into_iter()
            .find(|(_, _, s)| !s.error.is_empty())
            .map(|(id, _, s)| format!("{id}: {}", s.error))
            .unwrap_or_default()
    }

    fn is_connected(&self) -> bool {
        self.statuses().iter().any(|(_, _, s)| s.connected)
    }

    fn detail_json(&self) -> String {
        let sources: Vec<serde_json::Value> = self
            .statuses()
            .into_iter()
            .map(|(id, path, s)| {
                serde_json::json!({
                    "id": id, "path": path, "state": s.state, "connected": s.connected,
                    "error": s.error, "rows": s.rows, "bad_rows": s.bad_rows, "offset": s.offset,
                    "columns": s.columns.iter().map(|(n, v)| serde_json::json!({"name": n, "sample": v})).collect::<Vec<_>>(),
                    "header": s.header, "preamble": s.preamble,
                })
            })
            .collect();
        serde_json::json!({ "sources": sources }).to_string()
    }
}

pb_abi::export_pack!(PackState);

#[cfg(test)]
mod tests {
    use super::*;
    use pb_abi::{PbHostApi, PbPackApi, PB_ABI_VERSION, PB_VALUE_BOOL, PB_VALUE_F64, PB_VALUE_I64};
    use std::ffi::CStr;
    use std::os::raw::{c_char, c_void};
    use crate::tail::testutil::{append, dir};
    use std::time::Instant;
    use std::path::Path;
    use std::sync::Mutex as StdMutex;

    #[derive(Debug, Clone)]
    struct Rec {
        name: String,
        cell: Cell,
        quality: i32,
    }
    type Log = StdMutex<Vec<Rec>>;

    unsafe extern "C" fn cap(ctx: *mut c_void, name: *const c_char, kind: i32, value: *const c_void, len: usize, quality: i32, _s: *const c_char) {
        let log = &*(ctx as *const Log);
        let name = CStr::from_ptr(name).to_string_lossy().into_owned();
        let cell = match kind {
            PB_VALUE_BOOL => Cell::Bool(*(value as *const u8) != 0),
            PB_VALUE_I64 => Cell::Int(*(value as *const i64)),
            PB_VALUE_F64 => Cell::Float(*(value as *const f64)),
            _ => Cell::Text(String::from_utf8_lossy(std::slice::from_raw_parts(value as *const u8, len)).into_owned()),
        };
        log.lock().unwrap().push(Rec { name, cell, quality });
    }

    fn leak() -> &'static Log {
        Box::leak(Box::new(StdMutex::new(Vec::new())))
    }

    fn host(log: &'static Log) -> HostRef {
        HostRef::new(PbHostApi { abi_version: PB_ABI_VERSION, ctx: log as *const Log as *mut c_void, upsert_point: Some(cap), log_message: None })
    }

    fn wait(log: &'static Log, what: &str, f: impl Fn(&[Rec]) -> bool) {
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end {
            if f(&log.lock().unwrap()) {
                return;
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        panic!("timed out waiting for {what}; log = {:?}", log.lock().unwrap());
    }

    fn cfg(path: &Path, state: &Path, parser: &str, start: &str, binds: serde_json::Value) -> String {
        serde_json::json!({
            "state_dir": state.to_string_lossy(),
            "sources": [{"id":"m","path":path.to_string_lossy(),"parser":parser,"header_row":true,
                         "poll_ms":20,"start_at":start}],
            "inputs": [{"source":"m","binds":binds}]
        })
        .to_string()
    }

    fn last(log: &'static Log, n: &str) -> Option<Cell> {
        log.lock().unwrap().iter().rev().find(|r| r.name == n).map(|r| r.cell.clone())
    }

    #[test]
    fn csv_rows_become_typed_points_and_old_rows_are_skipped_from_end() {
        let d = dir("csv");
        let (p, st) = (d.join("m.csv"), d.join("state"));
        append(&p, "time,temp,run,tag\n10:00,20.5,true,A\n");
        let log = leak();
        let mut pk = PackState::new(host(log));
        pk.configure_json(&cfg(&p, &st, "csv", "end", serde_json::json!([
            {"column":"temp","point":"file_1"},{"column":"run","point":"file_2"},
            {"column":"3","point":"file_3"},{"column":"time","point":"file_4"}]))).unwrap();
        std::thread::sleep(Duration::from_millis(150));
        assert!(log.lock().unwrap().is_empty(), "start_at=end must skip the existing row");
        append(&p, "10:01,21.25,false,B\n");
        wait(log, "row", |l| l.iter().any(|r| r.name == "file_1"));
        assert_eq!(last(log, "file_1"), Some(Cell::Float(21.25)));
        assert_eq!(last(log, "file_2"), Some(Cell::Bool(false)));
        assert_eq!(last(log, "file_3"), Some(Cell::Text("B".into())));
        assert_eq!(last(log, "file_4"), Some(Cell::Text("10:01".into())));
        assert_eq!(pk.state_label(), "1/1 following");
        pk.stop_all();
    }

    #[test]
    fn restart_resumes_from_saved_offset_without_rereading() {
        let d = dir("restart");
        let (p, st) = (d.join("m.csv"), d.join("state"));
        append(&p, "t,v\n1,10\n2,20\n");
        let binds = serde_json::json!([{"column":"v","point":"file_1"}]);
        let log1 = leak();
        let mut pk = PackState::new(host(log1));
        pk.configure_json(&cfg(&p, &st, "csv", "beginning", binds.clone())).unwrap();
        wait(log1, "both rows", |l| l.iter().any(|r| r.cell == Cell::Int(20)));
        assert_eq!(log1.lock().unwrap().iter().filter(|r| r.name == "file_1").count(), 2);
        pk.stop_all();
        drop(pk);
        // Rows arrive while the service is down, then it comes back — start_at=beginning must
        // still honour the saved offset.
        append(&p, "3,30\n4,40\n");
        let log2 = leak();
        let mut pk2 = PackState::new(host(log2));
        pk2.configure_json(&cfg(&p, &st, "csv", "beginning", binds)).unwrap();
        wait(log2, "catch-up", |l| l.iter().any(|r| r.cell == Cell::Int(40)));
        let vals: Vec<Cell> = log2.lock().unwrap().iter().map(|r| r.cell.clone()).collect();
        assert_eq!(vals, vec![Cell::Int(30), Cell::Int(40)], "no re-read of 10 / 20, header still understood");
        pk2.stop_all();
    }

    #[test]
    fn rotation_rereads_the_new_header_and_columns_follow_it() {
        let d = dir("rotate");
        let (p, st) = (d.join("m.csv"), d.join("state"));
        append(&p, "a,b\n1,2\n");
        let log = leak();
        let mut pk = PackState::new(host(log));
        pk.configure_json(&cfg(&p, &st, "csv", "beginning", serde_json::json!([{"column":"b","point":"file_1"}]))).unwrap();
        wait(log, "first", |l| l.iter().any(|r| r.cell == Cell::Int(2)));
        // New file, columns swapped: "b" is now the first column.
        std::fs::remove_file(&p).unwrap();
        append(&p, "b,a\n7,8\n");
        wait(log, "after rotate", |l| l.iter().any(|r| r.cell == Cell::Int(7)));
        assert_eq!(last(log, "file_1"), Some(Cell::Int(7)));
        pk.stop_all();
    }

    #[test]
    fn missing_file_goes_bad_then_recovers() {
        let d = dir("missing");
        let (p, st) = (d.join("later.csv"), d.join("state"));
        let log = leak();
        let mut pk = PackState::new(host(log));
        pk.configure_json(&cfg(&p, &st, "csv", "end", serde_json::json!([{"column":"v","point":"file_1"}]))).unwrap();
        wait(log, "bad", |l| l.iter().any(|r| r.name == "file_1" && r.quality == PB_QUALITY_BAD));
        assert!(pk.last_error().contains("file not found"));
        assert_eq!(pk.state_label(), "0/1 following");
        append(&p, "v\n5\n");
        wait(log, "good", |l| l.iter().any(|r| r.quality == PB_QUALITY_GOOD && r.cell == Cell::Int(5)));
        assert!(pk.last_error().is_empty(), "{}", pk.last_error());
        pk.stop_all();
    }

    #[test]
    fn jsonl_paths_bad_lines_and_missing_paths() {
        let d = dir("jsonl");
        let (p, st) = (d.join("m.jsonl"), d.join("state"));
        let log = leak();
        let mut pk = PackState::new(host(log));
        let mut c: serde_json::Value = serde_json::from_str(&cfg(&p, &st, "jsonl", "beginning", serde_json::json!([
            {"column":"t.v","point":"file_1"},{"column":"ok","point":"file_2"},{"column":"nope","point":"file_3"}]))).unwrap();
        c["sources"][0]["header_row"] = false.into();
        pk.configure_json(&c.to_string()).unwrap();
        append(&p, "{\"t\":{\"v\":1.5},\"ok\":true}\nnot json\n{\"t\":{\"v\":2.5},\"ok\":false}\n");
        wait(log, "rows", |l| l.iter().any(|r| r.cell == Cell::Float(2.5)));
        assert_eq!(last(log, "file_2"), Some(Cell::Bool(false)));
        assert!(last(log, "file_3").is_none());
        let d = pk.detail_json();
        assert!(d.contains("\"bad_rows\":1"), "{d}");
        assert!(d.contains("\"rows\":2"), "{d}");
        assert!(pk.last_error().contains("nope") || pk.last_error().contains("invalid JSON"), "{}", pk.last_error());
        pk.stop_all();
    }

    #[test]
    fn column_picker_data_with_and_without_binds() {
        let d = dir("picker");
        let (p, st) = (d.join("m.csv"), d.join("state"));
        append(&p, "time,temp,run\n10:00,20.5,true\n10:01,21.5,false\n");
        // No binds: preview mode lists header names with the latest row's values, publishes nothing.
        let log = leak();
        let mut pk = PackState::new(host(log));
        pk.configure_json(&cfg(&p, &st, "csv", "end", serde_json::json!([]))).unwrap();
        let end = Instant::now() + Duration::from_secs(5);
        let mut d1 = String::new();
        while Instant::now() < end {
            d1 = pk.detail_json();
            if d1.contains("\"sample\":\"21.5\"") {
                break;
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        assert!(d1.contains("\"name\":\"temp\"") && d1.contains("\"sample\":\"21.5\""), "{d1}");
        assert!(d1.contains("preview"), "{d1}");
        assert!(log.lock().unwrap().is_empty(), "preview must not publish points");
        assert!(!st.exists() || std::fs::read_dir(&st).map(|r| r.count() == 0).unwrap_or(true), "preview must not save offsets");
        pk.stop_all();
        // With binds the running worker keeps the picker data fresh.
        let log2 = leak();
        let mut pk2 = PackState::new(host(log2));
        pk2.configure_json(&cfg(&p, &st, "csv", "end", serde_json::json!([{"column":"temp","point":"file_1"}]))).unwrap();
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end && !pk2.detail_json().contains("\"state\":\"following\"") {
            std::thread::sleep(Duration::from_millis(10));
        }
        append(&p, "10:02,22.5,true\n"); // appended only once the worker is positioned at the end
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end && !pk2.detail_json().contains("22.5") {
            std::thread::sleep(Duration::from_millis(30));
        }
        assert!(pk2.detail_json().contains("\"sample\":\"22.5\""), "{}", pk2.detail_json());
        pk2.stop_all();
        // JSON lines: dotted leaf paths with samples.
        let jp = d.join("m.jsonl");
        append(&jp, "{\"t\":{\"v\":1.5},\"ok\":true}\n");
        let mut c: serde_json::Value = serde_json::from_str(&cfg(&jp, &st, "jsonl", "end", serde_json::json!([]))).unwrap();
        c["sources"][0]["header_row"] = false.into();
        let mut pk3 = PackState::new(host(leak()));
        pk3.configure_json(&c.to_string()).unwrap();
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end && !pk3.detail_json().contains("t.v") {
            std::thread::sleep(Duration::from_millis(30));
        }
        assert!(pk3.detail_json().contains("\"name\":\"t.v\"") && pk3.detail_json().contains("\"sample\":\"1.5\""), "{}", pk3.detail_json());
        pk3.stop_all();
    }

    #[test]
    fn config_validation_and_unchanged_workers_survive_reconfigure() {
        let d = dir("cfg");
        let (p, st) = (d.join("m.csv"), d.join("state"));
        let log = leak();
        let mut pk = PackState::new(host(log));
        let c = cfg(&p, &st, "csv", "end", serde_json::json!([{"column":"v","point":"file_1"}]));
        pk.configure_json(&c).unwrap();
        let id1 = pk.workers[0].join.as_ref().unwrap().thread().id();
        pk.configure_json(&c).unwrap();
        assert_eq!(pk.workers[0].join.as_ref().unwrap().thread().id(), id1);
        // No binds -> a (preview) worker that publishes nothing; empty path -> config error.
        let none = cfg(&p, &st, "csv", "end", serde_json::json!([]));
        pk.configure_json(&none).unwrap();
        assert_eq!(pk.workers.len(), 1);
        let n = log.lock().unwrap().len(); // the replaced worker may have flagged its bind bad
        std::thread::sleep(Duration::from_millis(120));
        assert_eq!(log.lock().unwrap().len(), n, "a preview worker publishes nothing");
        let nopath = c.replace(&p.to_string_lossy().to_string(), "");
        pk.configure_json(&nopath).unwrap();
        assert!(pk.last_error().contains("no file path"));
        let baddelim = c.replace("\"parser\":\"csv\"", "\"parser\":\"csv\",\"delimiter\":\";;\"");
        pk.configure_json(&baddelim).unwrap();
        assert!(pk.last_error().contains("delimiter"));
        pk.stop_all();
    }

    #[test]
    fn auto_header_skips_preamble_and_units_and_names_columns() {
        let d = dir("auto-hdr");
        let (p, st) = (d.join("m.csv"), d.join("state"));
        append(&p, "Pump test,rig 4\nOperator: Sam\nSerial,SN-77\n\nTime,Pressure,Temp\ns,kPa,degC\n0,101.3,20.1\n1,101.4,20.3\n");
        let log = leak();
        let mut pk = PackState::new(host(log));
        pk.configure_json(&cfg(&p, &st, "csv", "beginning", serde_json::json!([
            {"column":"Pressure","point":"file_1"},{"column":"temp","point":"file_2"}]))).unwrap();
        wait(log, "both data rows", |l| l.iter().any(|r| r.cell == Cell::Float(101.4)));
        let vals: Vec<Cell> = log.lock().unwrap().iter().filter(|r| r.name == "file_1").map(|r| r.cell.clone()).collect();
        assert_eq!(vals, vec![Cell::Float(101.3), Cell::Float(101.4)], "preamble / header / units never published");
        append(&p, "2,101.9,20.5\n");
        wait(log, "appended", |l| l.iter().any(|r| r.cell == Cell::Float(20.5)));
        let dj = pk.detail_json();
        assert!(dj.contains("3 preamble lines · names on line 4 · +1 header line · 3 columns · data from line 6 (auto)"), "{dj}");
        assert!(dj.contains("Operator: Sam"), "{dj}");
        pk.stop_all();
        // Fixed line mode: names on line 2, everything above is preamble.
        let p2 = d.join("f.csv");
        append(&p2, "junk line\nA,B\n1,2\n3,4\n");
        let log2 = leak();
        let mut c: serde_json::Value = serde_json::from_str(&cfg(&p2, &st, "csv", "beginning", serde_json::json!([{"column":"B","point":"file_1"}]))).unwrap();
        c["sources"][0]["header_row"] = 2.into();
        let mut pk2 = PackState::new(host(log2));
        pk2.configure_json(&c.to_string()).unwrap();
        wait(log2, "fixed", |l| l.iter().any(|r| r.cell == Cell::Int(4)));
        let vals: Vec<Cell> = log2.lock().unwrap().iter().map(|r| r.cell.clone()).collect();
        assert_eq!(vals, vec![Cell::Int(2), Cell::Int(4)]);
        pk2.stop_all();
    }

    #[test]
    fn abi_registers() {
        let mut api = std::mem::MaybeUninit::<PbPackApi>::uninit();
        assert_eq!(unsafe { pb_pack_register(api.as_mut_ptr()) }, 0);
        assert_eq!(unsafe { pb_pack_abi_version() }, PB_ABI_VERSION);
    }
}
