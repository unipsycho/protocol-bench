//! C-ABI pack. Incoming endpoints are followed (growing `~A` section → points); outgoing
//! endpoints append rows built from host points.

use std::collections::BTreeMap;
use std::io::Write;
use std::os::raw::c_int;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use pb_abi::worker::{sanitize_id, sleep_stop, update_status};
use pb_abi::{value_text, HostRef, Pack, PB_LOG_ERROR, PB_LOG_INFO, PB_QUALITY_BAD, PB_QUALITY_GOOD};
use pb_tail::{load_state, save_state, Tail, TailError};
use serde::{Deserialize, Serialize};

use crate::las::{self, Curve, Header, HeaderParser};

const PACK_ID: &[u8] = b"las\0";
const PACK_TITLE: &[u8] = b"LAS\0";
/// Time-indexed outputs write at most one row per this interval.
const MIN_ROW_INTERVAL: Duration = Duration::from_millis(1000);

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PackConfig {
    #[serde(default)]
    state_dir: String,
    #[serde(default)]
    endpoints: Vec<EndpointJson>,
    #[serde(default)]
    inputs: Vec<InputJson>,
    #[serde(default)]
    outputs: Vec<OutputJson>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct EndpointJson {
    id: String,
    #[serde(default)]
    path: String,
    /// `incoming` | `outgoing`.
    #[serde(default = "default_role")]
    role: String,
    /// Incoming: keep following (true) or load once (false).
    #[serde(default = "yes")]
    follow: bool,
}

fn default_role() -> String {
    "incoming".into()
}
fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct BindJson {
    curve: String,
    #[serde(default)]
    point: String,
    #[serde(default)]
    source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct InputJson {
    endpoint: String,
    binds: Vec<BindJson>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct OutputJson {
    endpoint: String,
    /// `depth` | `time`.
    #[serde(default = "default_index")]
    index: String,
    binds: Vec<BindJson>,
}

fn default_index() -> String {
    "depth".into()
}

// ---------------------------------------------------------------------------
// Host
// ---------------------------------------------------------------------------


#[derive(Default, Clone)]
struct Status {
    state: String,
    error: String,
    connected: bool,
    rows: u64,
    bad_rows: u64,
    offset: u64,
    /// Incoming: curve mnemonics of the file (for the UI curve picker).
    curves: Vec<String>,
}

// ---------------------------------------------------------------------------
// Incoming (follow)
// ---------------------------------------------------------------------------

struct InBind {
    curve: String,
    name: String,
    source: String,
    column: Option<usize>,
    last: Option<f64>,
    warned: bool,
}

struct FollowCtx {
    host: HostRef,
    ep: EndpointJson,
    binds: Vec<InBind>,
    state_path: Option<PathBuf>,
    stop: Arc<AtomicBool>,
    status: Arc<Mutex<Status>>,
}

fn resolve(binds: &mut [InBind], header: &Header, host: &HostRef, id: &str, status: &Arc<Mutex<Status>>) {
    for b in binds.iter_mut() {
        b.column = header.column(&b.curve);
        if b.column.is_none() && !b.warned {
            b.warned = true;
            let msg = format!("curve \"{}\" is not in the file's ~Curve section", b.curve);
            host.log(PB_LOG_ERROR, &format!("las {id}: {msg}"));
            update_status(status, |s| s.error = msg);
        }
    }
}

/// Binds-free endpoint: report the file's curves (for the UI picker); consume and publish nothing.
fn run_preview(c: FollowCtx) {
    let path = PathBuf::from(c.ep.path.trim());
    while !c.stop.load(Ordering::SeqCst) {
        let exists = path.exists();
        match las::read_header(&path) {
            Ok(h) => update_status(&c.status, |s| {
                s.curves = h.curves.iter().map(|x| x.mnemonic.clone()).collect();
                s.connected = true;
                s.state = "preview (no binds)".into();
                s.error.clear();
            }),
            Err(e) => update_status(&c.status, |s| {
                s.curves.clear();
                s.connected = false;
                s.state = if exists { "error".into() } else { "waiting for file".into() };
                s.error = if exists { e } else { format!("file not found: {}", path.display()) };
            }),
        }
        sleep_stop(&c.stop, Duration::from_millis(1500));
    }
    update_status(&c.status, |s| {
        s.connected = false;
        s.state = "stopped".into();
    });
}

fn run_follow(mut c: FollowCtx) {
    let path = PathBuf::from(c.ep.path.trim());
    let saved = c.state_path.as_deref().and_then(load_state);
    let mut tail = Tail::start(&path, saved, false, true);
    let mut parser = HeaderParser::new();
    let mut header: Option<Header> = None;
    if tail.state.offset > 0 {
        // Resumed past the header: read it from the top.
        match las::read_header(&path) {
            Ok(h) => {
                parser = HeaderParser::new();
                resolve(&mut c.binds, &h, &c.host, &c.ep.id, &c.status);
                let names: Vec<String> = h.curves.iter().map(|x| x.mnemonic.clone()).collect();
                update_status(&c.status, |s| s.curves = names);
                header = Some(h);
            }
            Err(_) => tail = Tail::start(&path, None, false, true), // unusable state: start over
        }
    }
    let mut missing = false;
    let mut saved_offset = tail.state.offset;
    let mut loaded_once = false;
    let mut fatal: Option<String> = None;

    while !c.stop.load(Ordering::SeqCst) {
        if loaded_once && !c.ep.follow {
            sleep_stop(&c.stop, Duration::from_millis(200));
            continue;
        }
        match tail.poll() {
            Ok(batch) => {
                missing = false;
                if batch.reset {
                    parser = HeaderParser::new();
                    header = None;
                    fatal = None;
                }
                let mut rows = 0u64;
                for line in &batch.lines {
                    if fatal.is_some() {
                        break;
                    }
                    if header.is_none() {
                        if let Err(e) = parser.feed(line) {
                            fatal = Some(e);
                            break;
                        }
                        if parser.in_data() {
                            let h = parser.header().clone();
                            resolve(&mut c.binds, &h, &c.host, &c.ep.id, &c.status);
                            let names: Vec<String> = h.curves.iter().map(|x| x.mnemonic.clone()).collect();
                            update_status(&c.status, |s| s.curves = names);
                            header = Some(h);
                        }
                        continue;
                    }
                    let h = header.as_ref().unwrap();
                    if line.trim_start().starts_with('#') {
                        continue;
                    }
                    match las::parse_row(line, h) {
                        Ok(vals) => {
                            rows += 1;
                            for b in &mut c.binds {
                                let Some(col) = b.column else { continue };
                                match vals[col] {
                                    Some(v) => {
                                        c.host.upsert_f64(&b.name, &b.source, v, PB_QUALITY_GOOD);
                                        b.last = Some(v);
                                    }
                                    None => c.host.upsert_f64(&b.name, &b.source, b.last.unwrap_or(f64::NAN), PB_QUALITY_BAD),
                                }
                            }
                        }
                        Err(e) => update_status(&c.status, |s| {
                            s.bad_rows += 1;
                            s.error = format!("row skipped: {e}");
                        }),
                    }
                }
                if tail.state.offset != saved_offset {
                    if let Some(sp) = &c.state_path {
                        if let Err(e) = save_state(sp, &tail.state) {
                            c.host.log(PB_LOG_ERROR, &format!("las {}: cannot save offset: {e}", c.ep.id));
                        }
                    }
                    saved_offset = tail.state.offset;
                }
                if batch.lines.is_empty() && !batch.reset {
                    loaded_once = true;
                }
                let fatal_msg = fatal.clone();
                update_status(&c.status, |s| {
                    s.rows += rows;
                    s.offset = tail.state.offset;
                    s.connected = fatal_msg.is_none();
                    s.state = match (&fatal_msg, c.ep.follow, loaded_once) {
                        (Some(_), _, _) => "error".into(),
                        (None, true, _) => "following".into(),
                        (None, false, true) => "loaded".into(),
                        (None, false, false) => "loading".into(),
                    };
                    if let Some(m) = fatal_msg {
                        s.error = m;
                    } else if s.error.starts_with("file not found") {
                        s.error.clear();
                    }
                });
            }
            Err(TailError::NotFound) => {
                if !missing {
                    missing = true;
                    for b in &c.binds {
                        c.host.upsert_f64(&b.name, &b.source, b.last.unwrap_or(f64::NAN), PB_QUALITY_BAD);
                    }
                    c.host.log(PB_LOG_ERROR, &format!("las {}: {} not found", c.ep.id, path.display()));
                }
                update_status(&c.status, |s| {
                    s.connected = false;
                    s.state = "waiting for file".into();
                    s.error = format!("file not found: {}", path.display());
                });
            }
            Err(TailError::Io(e)) => update_status(&c.status, |s| {
                s.connected = false;
                s.state = "error".into();
                s.error = e;
            }),
        }
        sleep_stop(&c.stop, Duration::from_millis(250));
    }
    update_status(&c.status, |s| {
        s.connected = false;
        s.state = "stopped".into();
    });
}

// ---------------------------------------------------------------------------
// Outgoing (write)
// ---------------------------------------------------------------------------

struct Msg {
    point: String,
    raw: String,
}

struct OutCtx {
    host: HostRef,
    ep: EndpointJson,
    index: String,
    /// `(curve, point)` in file column order, excluding the depth index curve.
    cols: Vec<(String, String)>,
    /// Point driving the depth index (depth mode only).
    depth_point: Option<String>,
    rx: Receiver<Msg>,
    stop: Arc<AtomicBool>,
    status: Arc<Mutex<Status>>,
}

fn unit_for(curve: &str) -> &'static str {
    match curve.to_ascii_uppercase().as_str() {
        "DEPT" | "DEPTH" => "M",
        "TIME" => "S",
        "GR" => "GAPI",
        _ => "",
    }
}

fn run_write(c: OutCtx) {
    let _ = &c.host;
    let path = PathBuf::from(c.ep.path.trim());
    let depth_mode = c.index == "depth";
    let index_curve = Curve {
        mnemonic: if depth_mode { "DEPT".into() } else { "TIME".into() },
        unit: unit_for(if depth_mode { "DEPT" } else { "TIME" }).into(),
        description: String::new(),
    };
    let curves: Vec<Curve> = c.cols.iter().map(|(n, _)| Curve { mnemonic: n.clone(), unit: unit_for(n).into(), description: String::new() }).collect();
    let mut values: BTreeMap<String, f64> = BTreeMap::new(); // point -> latest numeric value
    let mut dirty = false;
    let mut last_depth: Option<f64> = None;
    let mut last_row = Instant::now() - MIN_ROW_INTERVAL;
    let mut ready = false; // header checked / written
    let mut rows_written = 0u64;
    update_status(&c.status, |s| {
        s.connected = true;
        s.state = "waiting for values".into();
    });

    while !c.stop.load(Ordering::SeqCst) {
        match c.rx.recv_timeout(Duration::from_millis(200)) {
            Ok(m) => {
                if let Ok(v) = m.raw.trim().parse::<f64>() {
                    if v.is_finite() {
                        let changed = values.insert(m.point.clone(), v) != Some(v);
                        if changed {
                            dirty = true;
                        }
                        // Depth mode: a new depth is a new row, even if a rate limit would apply.
                        if depth_mode && c.depth_point.as_deref() == Some(m.point.as_str()) && last_depth != Some(v) {
                            dirty = true;
                        }
                    }
                } else {
                    // Non-numeric / blank value: that column becomes NULL until a number arrives.
                    if values.remove(&m.point).is_some() {
                        dirty = true;
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if !dirty {
            continue;
        }
        let index_value = if depth_mode {
            match c.depth_point.as_ref().and_then(|p| values.get(p)) {
                Some(d) => *d,
                None => continue, // no depth yet
            }
        } else {
            if last_row.elapsed() < MIN_ROW_INTERVAL {
                continue;
            }
            SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
        };
        if depth_mode && last_depth == Some(index_value) {
            dirty = false;
            continue; // same depth: nothing new to record
        }
        let row_vals: Vec<Option<f64>> = c.cols.iter().map(|(_, p)| values.get(p).copied()).collect();

        if !ready {
            match prepare_file(&path, &c.ep.id, &index_curve, &curves, index_value) {
                Ok(()) => ready = true,
                Err(e) => {
                    update_status(&c.status, |s| {
                        s.connected = false;
                        s.state = "error".into();
                        s.error = e.clone();
                    });
                    c.host.log(PB_LOG_ERROR, &format!("las {}: {e}", c.ep.id));
                    sleep_stop(&c.stop, Duration::from_secs(2));
                    continue;
                }
            }
        }
        let line = las::render_row(index_value, &row_vals);
        match std::fs::OpenOptions::new().append(true).open(&path).and_then(|mut f| f.write_all(line.as_bytes())) {
            Ok(()) => {
                rows_written += 1;
                last_row = Instant::now();
                last_depth = Some(index_value);
                dirty = false;
                update_status(&c.status, |s| {
                    s.connected = true;
                    s.state = "writing".into();
                    s.rows = rows_written;
                    s.error.clear();
                });
            }
            Err(e) => {
                ready = false; // re-check the file next time (it may have been moved)
                update_status(&c.status, |s| {
                    s.connected = false;
                    s.state = "error".into();
                    s.error = format!("{}: {e}", path.display());
                });
            }
        }
    }
    update_status(&c.status, |s| {
        s.connected = false;
        s.state = "stopped".into();
    });
}

/// Make sure `path` holds a LAS header with exactly our curves (create it when missing / empty).
fn prepare_file(path: &std::path::Path, well: &str, index: &Curve, curves: &[Curve], start: f64) -> Result<(), String> {
    let existing = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if existing == 0 {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        return std::fs::write(path, las::render_header(well, index, curves, start)).map_err(|e| format!("{}: {e}", path.display()));
    }
    let h = las::read_header(path).map_err(|e| format!("{}: existing file is not a usable LAS file ({e})", path.display()))?;
    let want: Vec<&str> = std::iter::once(index.mnemonic.as_str()).chain(curves.iter().map(|c| c.mnemonic.as_str())).collect();
    let have: Vec<&str> = h.curves.iter().map(|c| c.mnemonic.as_str()).collect();
    let same = want.len() == have.len() && want.iter().zip(&have).all(|(a, b)| a.eq_ignore_ascii_case(b));
    if !same {
        return Err(format!("{}: existing curves [{}] differ from the bound curves [{}] — not appending", path.display(), have.join(", "), want.join(", ")));
    }
    // Make sure the last row ended with a newline before we append.
    if let Ok(bytes) = std::fs::read(path) {
        if bytes.last() != Some(&b'\n') {
            if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(path) {
                let _ = f.write_all(b"\n");
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Pack state
// ---------------------------------------------------------------------------

struct Worker {
    id: String,
    key: String,
    path: String,
    outgoing: bool,
    stop: Arc<AtomicBool>,
    status: Arc<Mutex<Status>>,
    tx: Option<Sender<Msg>>,
    /// Points this (outgoing) worker listens to.
    points: Vec<String>,
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
        let mut spawn: Vec<(EndpointJson, Vec<BindJson>, String, String)> = Vec::new();
        if self.enabled {
            for ep in &self.config.endpoints {
                let outgoing = ep.role.eq_ignore_ascii_case("outgoing");
                let (binds, index): (Vec<BindJson>, String) = if outgoing {
                    let o: Vec<&OutputJson> = self.config.outputs.iter().filter(|o| o.endpoint == ep.id).collect();
                    (o.iter().flat_map(|o| o.binds.clone()).collect(), o.first().map(|o| o.index.clone()).unwrap_or_else(default_index))
                } else {
                    (self.config.inputs.iter().filter(|i| i.endpoint == ep.id).flat_map(|i| i.binds.clone()).collect(), String::new())
                };
                let binds: Vec<BindJson> = binds.into_iter().filter(|b| !b.curve.trim().is_empty()).collect();
                // An incoming endpoint with no binds yet still runs a preview worker so the UI can
                // list the file's curves; an outgoing one has nothing to write.
                if binds.is_empty() && outgoing {
                    continue;
                }
                let key = serde_json::to_string(&(ep, &binds, &index, &self.config.state_dir)).unwrap_or_default();
                if let Some(pos) = old.iter().position(|w| w.id == ep.id && w.key == key) {
                    keep.push(old.remove(pos));
                } else {
                    spawn.push((ep.clone(), binds, index, key));
                }
            }
        }
        for w in &mut old {
            w.stop.store(true, Ordering::SeqCst);
        }
        for w in &mut old {
            w.shutdown();
        }
        for (ep, binds, index, key) in spawn {
            match self.spawn_worker(ep, binds, index, key) {
                Ok(w) => keep.push(w),
                Err(e) => {
                    self.host.log(PB_LOG_ERROR, &format!("las: {e}"));
                    self.config_error = e;
                }
            }
        }
        self.workers = keep;
        self.host.log(PB_LOG_INFO, &format!("las configured: {} endpoint(s)", self.workers.len()));
    }

    fn spawn_worker(&self, ep: EndpointJson, binds: Vec<BindJson>, index: String, key: String) -> Result<Worker, String> {
        if ep.path.trim().is_empty() {
            return Err(format!("{}: no file path set", ep.id));
        }
        let stop = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new(Status { state: "starting".into(), ..Default::default() }));
        let outgoing = ep.role.eq_ignore_ascii_case("outgoing");
        let (id, path) = (ep.id.clone(), ep.path.clone());
        if outgoing {
            let depth_mode = !index.eq_ignore_ascii_case("time");
            let mut cols: Vec<(String, String)> = Vec::new();
            let mut depth_point = None;
            for b in &binds {
                if b.point.trim().is_empty() {
                    continue;
                }
                let is_depth = matches!(b.curve.trim().to_ascii_uppercase().as_str(), "DEPT" | "DEPTH");
                if depth_mode && is_depth {
                    depth_point = Some(b.point.trim().to_string());
                } else {
                    cols.push((b.curve.trim().to_string(), b.point.trim().to_string()));
                }
            }
            if depth_mode && depth_point.is_none() {
                return Err(format!("{id}: depth-indexed output needs a DEPT curve bind (the depth point)"));
            }
            if !depth_mode && cols.is_empty() {
                return Err(format!("{id}: no curve binds with a point"));
            }
            let points: Vec<String> = cols.iter().map(|(_, p)| p.clone()).chain(depth_point.clone()).collect();
            let (tx, rx) = channel();
            let ctx = OutCtx {
                host: self.host,
                ep,
                index: if depth_mode { "depth".into() } else { "time".into() },
                cols,
                depth_point,
                rx,
                stop: stop.clone(),
                status: status.clone(),
            };
            let join = std::thread::Builder::new().name(format!("pb-las-out-{id}")).spawn(move || run_write(ctx)).map_err(|e| format!("spawn: {e}"))?;
            return Ok(Worker { id, key, path, outgoing: true, stop, status, tx: Some(tx), points, join: Some(join) });
        }
        let rts: Vec<InBind> = binds
            .iter()
            .map(|b| {
                let source = if b.source.is_empty() { format!("las/{}::{}", ep.id, b.curve.trim()) } else { b.source.clone() };
                let name = if b.point.trim().is_empty() { source.clone() } else { b.point.trim().to_string() };
                InBind { curve: b.curve.trim().to_string(), name, source, column: None, last: None, warned: false }
            })
            .collect();
        let state_path = (!self.config.state_dir.trim().is_empty()).then(|| PathBuf::from(&self.config.state_dir).join(format!("{}.json", sanitize_id(&ep.id))));
        let preview = rts.is_empty();
        let ctx = FollowCtx { host: self.host, ep, binds: rts, state_path, stop: stop.clone(), status: status.clone() };
        let join = std::thread::Builder::new()
            .name(format!("pb-las-in-{id}"))
            .spawn(move || if preview { run_preview(ctx) } else { run_follow(ctx) })
            .map_err(|e| format!("spawn: {e}"))?;
        Ok(Worker { id, key, path, outgoing: false, stop, status, tx: None, points: Vec::new(), join: Some(join) })
    }

    fn write_point(&self, name: &str, raw: &str) {
        for w in self.workers.iter().filter(|w| w.outgoing) {
            if w.points.iter().any(|p| p == name) {
                if let Some(tx) = &w.tx {
                    let _ = tx.send(Msg { point: name.to_string(), raw: raw.to_string() });
                }
            }
        }
    }

    fn statuses(&self) -> Vec<(String, String, bool, Status)> {
        self.workers.iter().map(|w| (w.id.clone(), w.path.clone(), w.outgoing, w.status.lock().map(|s| s.clone()).unwrap_or_default())).collect()
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

    fn write_point(&mut self, name: &str, _kind: i32, bytes: &[u8]) -> c_int {
        PackState::write_point(self, name, &value_text(bytes));
        0
    }

    fn state_label(&self) -> String {
        if !self.config_error.is_empty() {
            return "config error".into();
        }
        let st = self.statuses();
        if st.is_empty() {
            return if self.enabled { "idle".into() } else { "stopped".into() };
        }
        format!("{}/{} files ok", st.iter().filter(|(_, _, _, s)| s.connected).count(), st.len())
    }

    fn last_error(&self) -> String {
        if !self.config_error.is_empty() {
            return self.config_error.clone();
        }
        self.statuses().into_iter().find(|(_, _, _, s)| !s.error.is_empty()).map(|(id, _, _, s)| format!("{id}: {}", s.error)).unwrap_or_default()
    }

    fn is_connected(&self) -> bool {
        self.statuses().iter().any(|(_, _, _, s)| s.connected)
    }

    fn detail_json(&self) -> String {
        let eps: Vec<serde_json::Value> = self
            .statuses()
            .into_iter()
            .map(|(id, path, outgoing, s)| {
                serde_json::json!({
                    "id": id, "path": path, "role": if outgoing { "outgoing" } else { "incoming" },
                    "state": s.state, "connected": s.connected, "error": s.error,
                    "rows": s.rows, "bad_rows": s.bad_rows, "offset": s.offset, "curves": s.curves,
                })
            })
            .collect();
        serde_json::json!({ "endpoints": eps }).to_string()
    }
}

pb_abi::export_pack!(PackState);

#[cfg(test)]
mod tests {
    use super::*;
    use pb_abi::{PbHostApi, PbPackApi, PB_ABI_VERSION};
    use std::ffi::CStr;
    use std::os::raw::{c_char, c_void};
    use crate::las::{render_header, render_row, SAMPLE};
    use pb_tail::testutil::{append, dir};
    use std::path::Path;
    use std::sync::Mutex as StdMutex;

    #[derive(Debug, Clone)]
    struct Rec {
        name: String,
        v: f64,
        quality: i32,
    }
    type Log = StdMutex<Vec<Rec>>;

    unsafe extern "C" fn cap(ctx: *mut c_void, name: *const c_char, _k: i32, value: *const c_void, _l: usize, quality: i32, _s: *const c_char) {
        let log = &*(ctx as *const Log);
        let name = CStr::from_ptr(name).to_string_lossy().into_owned();
        log.lock().unwrap().push(Rec { name, v: *(value as *const f64), quality });
    }

    fn leak() -> &'static Log {
        Box::leak(Box::new(StdMutex::new(Vec::new())))
    }

    fn host(log: &'static Log) -> HostRef {
        HostRef::new(PbHostApi { abi_version: PB_ABI_VERSION, ctx: log as *const Log as *mut c_void, upsert_point: Some(cap), log_message: None })
    }

    fn wait(what: &str, f: impl Fn() -> bool) {
        let end = Instant::now() + Duration::from_secs(8);
        while Instant::now() < end {
            if f() {
                return;
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        panic!("timed out waiting for {what}");
    }

    fn last(log: &'static Log, n: &str) -> Option<Rec> {
        log.lock().unwrap().iter().rev().find(|r| r.name == n).cloned()
    }

    fn in_cfg(path: &Path, state: &Path, follow: bool) -> String {
        serde_json::json!({
            "state_dir": state.to_string_lossy(),
            "endpoints": [{"id":"w","path":path.to_string_lossy(),"role":"incoming","follow":follow}],
            "inputs": [{"endpoint":"w","binds":[{"curve":"GR","point":"las_1"},{"curve":"dt","point":"las_2"},
                                                {"curve":"DEPT","point":"las_3"},{"curve":"NOPE","point":"las_4"}]}]
        })
        .to_string()
    }

    #[test]
    fn follows_a_growing_log_with_nulls_resume_and_unknown_curves() {
        let d = dir("lasin");
        let (p, st) = (d.join("w.las"), d.join("state"));
        std::fs::write(&p, SAMPLE).unwrap();
        let log = leak();
        let mut pk = PackState::new(host(log));
        pk.configure_json(&in_cfg(&p, &st, true)).unwrap();
        wait("rows", || last(log, "las_3").is_some_and(|r| r.v == 1669.875));
        assert_eq!(last(log, "las_2").unwrap().v, 123.45);
        let gr = last(log, "las_1").unwrap();
        // The last row had NULL for GR: the last good value is held and the quality goes bad.
        assert_eq!((gr.v, gr.quality), (110.2, PB_QUALITY_BAD));
        assert!(log.lock().unwrap().iter().any(|r| r.name == "las_1" && r.quality == PB_QUALITY_GOOD && r.v == 110.2));
        assert!(pk.last_error().contains("NOPE"), "{}", pk.last_error());
        assert!(pk.detail_json().contains("\"curves\":[\"DEPT\",\"DT\",\"GR\"]"), "{}", pk.detail_json());
        // Logging continues.
        append(&p, "1669.750   124.0   111.0\n");
        wait("append", || last(log, "las_1").is_some_and(|r| r.v == 111.0 && r.quality == PB_QUALITY_GOOD));
        pk.stop_all();
        drop(pk);
        // Restart: rows written while down are caught up, nothing is re-read.
        append(&p, "1669.625   125.0   112.0\n");
        let log2 = leak();
        let mut pk2 = PackState::new(host(log2));
        pk2.configure_json(&in_cfg(&p, &st, true)).unwrap();
        wait("catch-up", || last(log2, "las_1").is_some_and(|r| r.v == 112.0));
        let depths: Vec<f64> = log2.lock().unwrap().iter().filter(|r| r.name == "las_3").map(|r| r.v).collect();
        assert_eq!(depths, vec![1669.625], "only the new row, header still understood");
        pk2.stop_all();
    }

    #[test]
    fn endpoint_without_binds_lists_its_curves_and_publishes_nothing() {
        let d = dir("lasprev");
        let (p, st) = (d.join("w.las"), d.join("state"));
        std::fs::write(&p, SAMPLE).unwrap();
        let log = leak();
        let mut pk = PackState::new(host(log));
        let mut c: serde_json::Value = serde_json::from_str(&in_cfg(&p, &st, true)).unwrap();
        c["inputs"] = serde_json::json!([]);
        pk.configure_json(&c.to_string()).unwrap();
        wait("curves", || pk.detail_json().contains("\"curves\":[\"DEPT\",\"DT\",\"GR\"]"));
        assert!(pk.detail_json().contains("preview"), "{}", pk.detail_json());
        assert!(log.lock().unwrap().is_empty());
        assert!(!st.exists(), "preview must not save offsets");
        // A missing file is reported, not fatal.
        std::fs::remove_file(&p).unwrap();
        wait("missing", || pk.last_error().contains("file not found"));
        pk.stop_all();
    }

    #[test]
    fn once_mode_loads_the_file_and_stops_following() {
        let d = dir("lasonce");
        let (p, st) = (d.join("w.las"), d.join("state"));
        std::fs::write(&p, SAMPLE).unwrap();
        let log = leak();
        let mut pk = PackState::new(host(log));
        pk.configure_json(&in_cfg(&p, &st, false)).unwrap();
        wait("loaded", || pk.detail_json().contains("\"state\":\"loaded\""));
        let n = log.lock().unwrap().len();
        append(&p, "1669.5   1   2\n");
        std::thread::sleep(Duration::from_millis(700));
        assert_eq!(log.lock().unwrap().len(), n, "no more reads in once mode");
        pk.stop_all();
    }

    #[test]
    fn bad_rows_and_unsupported_files_are_reported() {
        let d = dir("lasbad");
        let (p, st) = (d.join("w.las"), d.join("state"));
        std::fs::write(&p, format!("{SAMPLE}garbage line\n1669.5   9   8\n")).unwrap();
        let log = leak();
        let mut pk = PackState::new(host(log));
        pk.configure_json(&in_cfg(&p, &st, true)).unwrap();
        wait("good row after garbage", || last(log, "las_1").is_some_and(|r| r.v == 8.0));
        assert!(pk.detail_json().contains("\"bad_rows\":1"));
        pk.stop_all();
        let wrapped = SAMPLE.replace("WRAP.   NO ", "WRAP.   YES");
        std::fs::write(&p, wrapped).unwrap();
        let log2 = leak();
        let mut pk2 = PackState::new(host(log2));
        pk2.configure_json(&in_cfg(&p, &d.join("state2"), true)).unwrap();
        wait("wrap error", || pk2.last_error().contains("WRAP YES"));
        pk2.stop_all();
    }

    fn out_cfg(path: &Path, index: &str, binds: serde_json::Value) -> String {
        serde_json::json!({
            "endpoints": [{"id":"out","path":path.to_string_lossy(),"role":"outgoing"}],
            "outputs": [{"endpoint":"out","index":index,"binds":binds}]
        })
        .to_string()
    }

    #[test]
    fn depth_indexed_output_creates_header_and_appends_one_row_per_depth() {
        let d = dir("lasout");
        let p = d.join("sub/out.las");
        let log = leak();
        let mut pk = PackState::new(host(log));
        pk.configure_json(&out_cfg(&p, "depth", serde_json::json!([
            {"curve":"DEPT","point":"depth"},{"curve":"GR","point":"gr"},{"curve":"ROP","point":"rop"}]))).unwrap();
        pk.write_point("gr", "55.5");
        pk.write_point("depth", "100.0");
        wait("first row", || std::fs::read_to_string(&p).map(|t| t.contains("100 55.5")).unwrap_or(false));
        pk.write_point("rop", "12.25");
        pk.write_point("gr", "56"); // same depth: no new row yet
        std::thread::sleep(Duration::from_millis(300));
        pk.write_point("depth", "100.5");
        wait("second row", || std::fs::read_to_string(&p).map(|t| t.contains("100.5 56 12.25")).unwrap_or(false));
        let text = std::fs::read_to_string(&p).unwrap();
        assert_eq!(text.matches("\n100 ").count(), 1, "one row per depth:\n{text}");
        // First row has ROP as NULL (no value had arrived yet).
        assert!(text.contains("100 55.5 -999.25"), "{text}");
        let h = las::read_header(&p).unwrap();
        assert_eq!(h.curves.iter().map(|c| c.mnemonic.as_str()).collect::<Vec<_>>(), vec!["DEPT", "GR", "ROP"]);
        assert!(pk.detail_json().contains("\"rows\":2"), "{}", pk.detail_json());
        pk.stop_all();
        // A new run appends to the same file (same curves) rather than rewriting it.
        let mut pk2 = PackState::new(host(leak()));
        pk2.configure_json(&out_cfg(&p, "depth", serde_json::json!([
            {"curve":"DEPT","point":"depth"},{"curve":"GR","point":"gr"},{"curve":"ROP","point":"rop"}]))).unwrap();
        pk2.write_point("gr", "57");
        pk2.write_point("depth", "101"); // a row is recorded when the depth changes
        wait("appended", || std::fs::read_to_string(&p).map(|t| t.contains("101 57")).unwrap_or(false));
        assert_eq!(std::fs::read_to_string(&p).unwrap().matches("~Curve").count(), 1);
        pk2.stop_all();
    }

    #[test]
    fn existing_file_with_other_curves_is_not_touched() {
        let d = dir("lasclash");
        let p = d.join("other.las");
        let idx = Curve { mnemonic: "DEPT".into(), unit: "M".into(), description: String::new() };
        let other = vec![Curve { mnemonic: "SP".into(), unit: "MV".into(), description: String::new() }];
        let original = format!("{}{}", render_header("x", &idx, &other, 0.0), render_row(1.0, &[Some(2.0)]));
        std::fs::write(&p, &original).unwrap();
        let mut pk = PackState::new(host(leak()));
        pk.configure_json(&out_cfg(&p, "depth", serde_json::json!([{"curve":"DEPT","point":"d"},{"curve":"GR","point":"g"}]))).unwrap();
        pk.write_point("d", "5");
        pk.write_point("g", "1");
        wait("refusal", || pk.last_error().contains("not appending"));
        assert_eq!(std::fs::read_to_string(&p).unwrap(), original);
        pk.stop_all();
    }

    #[test]
    fn time_indexed_output_and_config_validation() {
        let d = dir("lastime");
        let p = d.join("t.las");
        let mut pk = PackState::new(host(leak()));
        pk.configure_json(&out_cfg(&p, "time", serde_json::json!([{"curve":"TEMP","point":"t1"}]))).unwrap();
        pk.write_point("t1", "21.5");
        wait("time row", || std::fs::read_to_string(&p).map(|t| t.contains(" 21.5")).unwrap_or(false));
        let text = std::fs::read_to_string(&p).unwrap();
        let h = las::read_header(&p).unwrap();
        assert_eq!(h.curves[0].mnemonic, "TIME");
        let row = text.lines().last().unwrap();
        let t: f64 = row.split_whitespace().next().unwrap().parse().unwrap();
        assert!(t > 1.7e9, "epoch-seconds index, got {t}");
        pk.stop_all();
        // Depth output without a DEPT bind, and a missing path, are config errors.
        let mut pk = PackState::new(host(leak()));
        pk.configure_json(&out_cfg(&p, "depth", serde_json::json!([{"curve":"GR","point":"g"}]))).unwrap();
        assert!(pk.last_error().contains("needs a DEPT"), "{}", pk.last_error());
        pk.configure_json(&out_cfg(Path::new(""), "time", serde_json::json!([{"curve":"GR","point":"g"}]))).unwrap();
        assert!(pk.last_error().contains("no file path"), "{}", pk.last_error());
    }

    #[test]
    fn abi_registers() {
        let mut api = std::mem::MaybeUninit::<PbPackApi>::uninit();
        assert_eq!(unsafe { pb_pack_register(api.as_mut_ptr()) }, 0);
        assert_eq!(unsafe { pb_pack_abi_version() }, PB_ABI_VERSION);
    }
}
