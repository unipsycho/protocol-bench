//! Shared runtime for the "ext" protocol packs (WITS, WebSocket, Sparkplug B, DNP3, CAN).
//!
//! Every ext pack receives the same configure document from the service
//! (`build_ext_configure` in `pb-service`):
//!
//! ```json
//! { "connections": [ { "id": "van", "enabled": true, ... } ],
//!   "inputs":  [ { "connection": "van", "interval_secs": 1.0, "binds": [ { ..., "point": "wits_1", "source": "wits/van::0108" } ] } ],
//!   "outputs": [ { "connection": "van", "interval_secs": 1.0, "binds": [ { ..., "point": "modbus_3" } ] } ] }
//! ```
//!
//! [`ExtPack`] does the parts that are the same for all of them — one worker thread per enabled
//! connection, restarting only workers whose config changed, routing host point writes to the
//! workers that have an Output bind for that point, status / error / "seen" reporting in
//! `detail_json` — and calls the pack's [`RunFn`] for the protocol itself. A pack's own code is
//! then just "open the link, turn wire data into points, turn writes into wire data".
//!
//! `detail_json` shape (the UI's generic panel reads it):
//!
//! ```json
//! { "connections": [ { "id", "endpoint", "state", "connected", "error", "rx", "tx", "errors",
//!                      "peers", "seen": [ { "leaf": {"item":"0108"}, "label": "0108 Bit depth", "value": "1234.5" } ] } ] }
//! ```

pub mod link;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub use pb_abi::worker::{sleep_stop, update_status};
use pb_abi::{HostRef, PB_LOG_INFO, PB_LOG_WARN, PB_QUALITY_BAD, PB_QUALITY_GOOD};
use serde::Deserialize;
use serde_json::{Map, Value};

// ---------------------------------------------------------------------------
// Values
// ---------------------------------------------------------------------------

/// A typed value on its way to (or from) a host point.
#[derive(Debug, Clone, PartialEq)]
pub enum Val {
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
}

impl Val {
    /// Type a text value: `true` / `false`, integer, float, else text.
    pub fn parse_text(t: &str) -> Val {
        let s = t.trim();
        match s.to_ascii_lowercase().as_str() {
            "true" => return Val::Bool(true),
            "false" => return Val::Bool(false),
            _ => {}
        }
        if let Ok(i) = s.parse::<i64>() {
            return Val::Int(i);
        }
        if let Ok(f) = s.parse::<f64>() {
            if f.is_finite() {
                return Val::Float(f);
            }
        }
        Val::Text(t.to_string())
    }

    /// Numeric view (bools are 0 / 1). `None` for non-numeric text.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Val::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            Val::Int(i) => Some(*i as f64),
            Val::Float(f) => Some(*f),
            Val::Text(t) => t.trim().parse().ok(),
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Val::Bool(b) => Some(*b),
            Val::Int(i) => Some(*i != 0),
            Val::Float(f) => Some(*f != 0.0),
            Val::Text(t) => match t.trim().to_ascii_lowercase().as_str() {
                "true" | "on" | "1" | "yes" => Some(true),
                "false" | "off" | "0" | "no" => Some(false),
                _ => None,
            },
        }
    }

    /// JSON form (numbers stay numbers).
    pub fn to_json(&self) -> Value {
        match self {
            Val::Bool(b) => Value::Bool(*b),
            Val::Int(i) => Value::from(*i),
            Val::Float(f) => serde_json::Number::from_f64(*f).map(Value::Number).unwrap_or(Value::Null),
            Val::Text(t) => Value::String(t.clone()),
        }
    }

    /// From a JSON leaf. Objects / arrays become their JSON text.
    pub fn from_json(v: &Value) -> Val {
        match v {
            Value::Bool(b) => Val::Bool(*b),
            Value::Number(n) => n.as_i64().map(Val::Int).unwrap_or_else(|| Val::Float(n.as_f64().unwrap_or(0.0))),
            Value::String(s) => Val::Text(s.clone()),
            Value::Null => Val::Text(String::new()),
            other => Val::Text(other.to_string()),
        }
    }

    /// Publish as a host point.
    pub fn put(&self, host: &HostRef, name: &str, source: &str, quality: i32) {
        match self {
            Val::Bool(b) => host.upsert_bool(name, source, *b, quality),
            Val::Int(i) => host.upsert_i64(name, source, *i, quality),
            Val::Float(f) => host.upsert_f64(name, source, *f, quality),
            Val::Text(t) => host.upsert_text(name, source, t, quality),
        }
    }
}

impl std::fmt::Display for Val {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Val::Bool(b) => write!(f, "{b}"),
            Val::Int(i) => write!(f, "{i}"),
            Val::Float(x) => write!(f, "{x}"),
            Val::Text(t) => f.write_str(t),
        }
    }
}

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Default)]
struct RawConfig {
    #[serde(default)]
    connections: Vec<Value>,
    #[serde(default)]
    inputs: Vec<RawGroup>,
    #[serde(default)]
    outputs: Vec<RawGroup>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawGroup {
    connection: String,
    #[serde(default = "one")]
    interval_secs: f64,
    #[serde(default)]
    binds: Vec<Value>,
}

fn one() -> f64 {
    1.0
}

/// One bind as a worker sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct Bind {
    /// All fields of the bind as saved by the UI (`item`, `path`, `kind`, …).
    pub fields: Map<String, Value>,
    /// Host point name: the assigned short id, else (Input with no id yet) the source address.
    pub point: String,
    /// Wire address (`wits/van::0108`); empty on Output binds.
    pub source: String,
    /// Interval of the node this bind came from (poll period for Inputs, send period for Outputs).
    pub interval: Duration,
}

impl Bind {
    /// A field as text (strings as-is, numbers / bools formatted).
    pub fn text(&self, key: &str) -> String {
        field_text(&self.fields, key)
    }
    /// A numeric field (strings are parsed too).
    pub fn num(&self, key: &str) -> Option<f64> {
        match self.fields.get(key) {
            Some(Value::Number(n)) => n.as_f64(),
            Some(Value::String(s)) => s.trim().parse().ok(),
            Some(Value::Bool(b)) => Some(if *b { 1.0 } else { 0.0 }),
            _ => None,
        }
    }
    pub fn flag(&self, key: &str) -> bool {
        match self.fields.get(key) {
            Some(Value::Bool(b)) => *b,
            Some(Value::String(s)) => matches!(s.trim().to_ascii_lowercase().as_str(), "true" | "yes" | "1" | "on"),
            Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0) != 0.0,
            _ => false,
        }
    }
    /// Scale / offset fields (identity when missing or zero scale).
    pub fn scaling(&self) -> (f64, f64) {
        let s = self.num("scale").filter(|s| *s != 0.0 && s.is_finite()).unwrap_or(1.0);
        let o = self.num("offset").filter(|o| o.is_finite()).unwrap_or(0.0);
        (s, o)
    }
}

/// A field of a JSON object as text.
pub fn field_text(m: &Map<String, Value>, key: &str) -> String {
    match m.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

/// Connection field helpers.
pub trait ConnExt {
    fn s(&self, key: &str, default: &str) -> String;
    fn n(&self, key: &str, default: f64) -> f64;
    fn b(&self, key: &str, default: bool) -> bool;
}

impl ConnExt for Map<String, Value> {
    fn s(&self, key: &str, default: &str) -> String {
        let t = field_text(self, key);
        if t.trim().is_empty() {
            default.to_string()
        } else {
            t.trim().to_string()
        }
    }
    fn n(&self, key: &str, default: f64) -> f64 {
        match self.get(key) {
            Some(Value::Number(n)) => n.as_f64().unwrap_or(default),
            Some(Value::String(s)) => s.trim().parse().unwrap_or(default),
            _ => default,
        }
    }
    fn b(&self, key: &str, default: bool) -> bool {
        match self.get(key) {
            Some(Value::Bool(b)) => *b,
            Some(Value::String(s)) => matches!(s.trim().to_ascii_lowercase().as_str(), "true" | "yes" | "1" | "on"),
            Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0) != 0.0,
            _ => default,
        }
    }
}

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

/// Something the link has seen on the wire, offered as a one-click bind in the UI.
#[derive(Debug, Clone, PartialEq)]
pub struct Seen {
    /// Bind fields that select it (`{"item": "0108"}`).
    pub leaf: Map<String, Value>,
    pub label: String,
    pub value: String,
    pub at_ms: u64,
}

/// What a worker reports (shared with the pack thread for `detail_json`).
#[derive(Debug, Clone, Default)]
pub struct LinkStatus {
    pub state: String,
    pub connected: bool,
    pub error: String,
    pub rx: u64,
    pub tx: u64,
    pub errors: u64,
    /// Connected peers (listeners / servers).
    pub peers: Option<u64>,
    seen: BTreeMap<String, Seen>,
}

/// Cap on the "seen" table so a chatty link cannot grow it forever.
pub const SEEN_MAX: usize = 400;

impl LinkStatus {
    /// Record something seen on the wire under `key` (replaces the older entry).
    pub fn see(&mut self, key: &str, leaf: Map<String, Value>, label: impl Into<String>, value: impl Into<String>) {
        if !self.seen.contains_key(key) && self.seen.len() >= SEEN_MAX {
            return;
        }
        self.seen.insert(key.to_string(), Seen { leaf, label: label.into(), value: value.into(), at_ms: now_ms() });
    }
    pub fn seen(&self) -> impl Iterator<Item = &Seen> {
        self.seen.values()
    }
}

/// Build a one-field leaf map.
pub fn leaf1(k: &str, v: impl Into<Value>) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert(k.to_string(), v.into());
    m
}

// ---------------------------------------------------------------------------
// Worker plumbing
// ---------------------------------------------------------------------------

/// A host point write routed to a worker: (point name, value text).
pub type Write = (String, String);

/// Everything a pack's protocol code gets for one connection.
pub struct LinkCtx {
    pub host: HostRef,
    pub id: String,
    pub conn: Map<String, Value>,
    pub inputs: Vec<Bind>,
    pub outputs: Vec<Bind>,
    pub stop: Arc<AtomicBool>,
    pub status: Arc<Mutex<LinkStatus>>,
    pub writes: Receiver<Write>,
    /// Latest value written per Output point (filled before the worker starts with any value
    /// the pack already knows, then kept up to date by [`LinkCtx::drain_writes`]).
    pub latest: BTreeMap<String, String>,
}

impl LinkCtx {
    pub fn stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    pub fn status(&self, f: impl FnOnce(&mut LinkStatus)) {
        update_status(&self.status, f);
    }

    pub fn log(&self, level: i32, msg: &str) {
        self.host.log(level, &format!("{}: {msg}", self.id));
    }

    /// Mark the link up / down with a state line.
    pub fn up(&self, state: impl Into<String>) {
        let state = state.into();
        self.status(|s| {
            s.connected = true;
            s.state = state;
            s.error.clear();
        });
    }

    /// Link failed: every Input point goes bad (last value held by the host), error shown.
    pub fn down(&self, err: impl Into<String>) {
        let err = err.into();
        for b in &self.inputs {
            self.host.upsert_text(&b.point, &b.source, "", PB_QUALITY_BAD);
        }
        self.host.log(PB_LOG_WARN, &format!("{}: {err}", self.id));
        self.status(|s| {
            s.connected = false;
            s.state = "error".into();
            s.error = err;
            s.errors += 1;
        });
    }

    /// Publish a good value for an Input bind.
    pub fn publish(&self, b: &Bind, v: &Val) {
        v.put(&self.host, &b.point, &b.source, PB_QUALITY_GOOD);
    }

    /// Publish a bad quality for one Input bind (value held).
    pub fn publish_bad(&self, b: &Bind) {
        self.host.upsert_text(&b.point, &b.source, "", PB_QUALITY_BAD);
    }

    /// Pull queued writes into `latest`; returns the ones that arrived (in order).
    pub fn drain_writes(&mut self) -> Vec<Write> {
        let mut got = Vec::new();
        while let Ok((p, v)) = self.writes.try_recv() {
            self.latest.insert(p.clone(), v.clone());
            got.push((p, v));
        }
        got
    }

    /// Wait up to `d` for the first write (then drain the rest). Returns as soon as stop is set.
    pub fn wait_writes(&mut self, d: Duration) -> Vec<Write> {
        let end = Instant::now() + d;
        loop {
            let got = self.drain_writes();
            if !got.is_empty() || self.stopped() || Instant::now() >= end {
                return got;
            }
            let slice = Duration::from_millis(20).min(end.saturating_duration_since(Instant::now()));
            match self.writes.recv_timeout(slice) {
                Ok((p, v)) => {
                    self.latest.insert(p.clone(), v.clone());
                    let mut got = vec![(p, v)];
                    got.extend(self.drain_writes());
                    return got;
                }
                Err(_) => continue,
            }
        }
    }

    /// Output binds whose point is `name`.
    pub fn outputs_for<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Bind> + 'a {
        self.outputs.iter().filter(move |b| b.point == name)
    }

    /// Shortest Input interval (poll period), or `default`.
    pub fn input_interval(&self, default: Duration) -> Duration {
        self.inputs.iter().map(|b| b.interval).min().unwrap_or(default)
    }

    /// Shortest Output interval (send period), or `default`.
    pub fn output_interval(&self, default: Duration) -> Duration {
        self.outputs.iter().map(|b| b.interval).min().unwrap_or(default)
    }
}

/// The protocol code of a pack: runs one connection until `ctx.stop` is set.
pub type RunFn = fn(LinkCtx);
/// Short endpoint text for a connection (`127.0.0.1:5100`, `can0`).
pub type EndpointFn = fn(&Map<String, Value>) -> String;
/// Validate a connection before a worker starts it.
pub type CheckFn = fn(&Map<String, Value>) -> Result<(), String>;

struct Worker {
    id: String,
    endpoint: String,
    key: String,
    stop: Arc<AtomicBool>,
    tx: Sender<Write>,
    status: Arc<Mutex<LinkStatus>>,
    out_points: Vec<String>,
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

/// Generic pack state: wrap it in the pack's own type and forward the `Pack` methods.
pub struct ExtPack {
    pub host: HostRef,
    name: &'static str,
    run: RunFn,
    endpoint: EndpointFn,
    check: CheckFn,
    enabled: bool,
    cfg: RawConfig,
    workers: Vec<Worker>,
    config_error: String,
    /// Last value seen per point, replayed into a worker that (re)starts.
    latest: BTreeMap<String, String>,
}

impl ExtPack {
    pub fn new(host: HostRef, name: &'static str, run: RunFn, endpoint: EndpointFn, check: CheckFn) -> Self {
        Self {
            host,
            name,
            run,
            endpoint,
            check,
            enabled: true,
            cfg: RawConfig::default(),
            workers: Vec::new(),
            config_error: String::new(),
            latest: BTreeMap::new(),
        }
    }

    pub fn configure(&mut self, json: &str) -> Result<(), String> {
        let cfg: RawConfig = serde_json::from_str(json).map_err(|e| format!("json: {e}"))?;
        for c in &cfg.connections {
            if !c.is_object() {
                return Err("connection is not an object".into());
            }
        }
        self.cfg = cfg;
        self.config_error.clear();
        self.sync();
        Ok(())
    }

    pub fn config_failed(&mut self, err: String) {
        self.host.log(PB_LOG_WARN, &format!("{}: config: {err}", self.name));
        self.config_error = err;
    }

    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
        self.sync();
    }

    pub fn shutdown(&mut self) {
        for w in &mut self.workers {
            w.shutdown();
        }
        self.workers.clear();
    }

    fn group_binds(groups: &[RawGroup], conn: &str, incoming: bool) -> Vec<Bind> {
        let mut out = Vec::new();
        for g in groups.iter().filter(|g| g.connection.trim() == conn) {
            for b in &g.binds {
                let Some(fields) = b.as_object() else { continue };
                let source = field_text(fields, "source");
                let point = field_text(fields, "point").trim().to_string();
                let point = if point.is_empty() {
                    if incoming {
                        source.clone()
                    } else {
                        continue;
                    }
                } else {
                    point
                };
                if incoming && source.is_empty() {
                    continue;
                }
                out.push(Bind {
                    fields: fields.clone(),
                    point,
                    source,
                    interval: Duration::from_secs_f64(g.interval_secs.clamp(0.02, 86_400.0)),
                });
            }
        }
        out
    }

    fn sync(&mut self) {
        let mut wanted: Vec<(String, Map<String, Value>, Vec<Bind>, Vec<Bind>, String)> = Vec::new();
        if self.enabled {
            for c in &self.cfg.connections {
                let Some(m) = c.as_object() else { continue };
                let id = field_text(m, "id").trim().to_string();
                if id.is_empty() || !m.b("enabled", true) {
                    continue;
                }
                let ins = Self::group_binds(&self.cfg.inputs, &id, true);
                let outs = Self::group_binds(&self.cfg.outputs, &id, false);
                let key = serde_json::to_string(&(c, &self.cfg.inputs.iter().filter(|g| g.connection == id).map(|g| (&g.binds, g.interval_secs)).collect::<Vec<_>>(), &self.cfg.outputs.iter().filter(|g| g.connection == id).map(|g| (&g.binds, g.interval_secs)).collect::<Vec<_>>())).unwrap_or_default();
                wanted.push((id, m.clone(), ins, outs, key));
            }
        }
        // Stop workers that are gone or changed.
        let mut keep = Vec::new();
        for mut w in std::mem::take(&mut self.workers) {
            if wanted.iter().any(|(id, _, _, _, key)| *id == w.id && *key == w.key) {
                keep.push(w);
            } else {
                w.shutdown();
            }
        }
        self.workers = keep;
        for (id, conn, inputs, outputs, key) in wanted {
            if self.workers.iter().any(|w| w.id == id) {
                continue;
            }
            let endpoint = (self.endpoint)(&conn);
            let status = Arc::new(Mutex::new(LinkStatus { state: "starting".into(), ..Default::default() }));
            if let Err(e) = (self.check)(&conn) {
                update_status(&status, |s| {
                    s.state = "error".into();
                    s.error = e.clone();
                });
                self.host.log(PB_LOG_WARN, &format!("{} {id}: {e}", self.name));
                // Keep a placeholder so the error shows; no thread.
                let (tx, _rx) = channel();
                self.workers.push(Worker { id, endpoint, key, stop: Arc::new(AtomicBool::new(true)), tx, status, out_points: Vec::new(), join: None });
                continue;
            }
            let stop = Arc::new(AtomicBool::new(false));
            let (tx, rx) = channel();
            let out_points: Vec<String> = outputs.iter().map(|b| b.point.clone()).collect();
            let latest: BTreeMap<String, String> =
                self.latest.iter().filter(|(k, _)| out_points.contains(k)).map(|(k, v)| (k.clone(), v.clone())).collect();
            let ctx = LinkCtx {
                host: self.host,
                id: id.clone(),
                conn,
                inputs,
                outputs,
                stop: stop.clone(),
                status: status.clone(),
                writes: rx,
                latest,
            };
            let run = self.run;
            self.host.log(PB_LOG_INFO, &format!("{} {id}: starting {endpoint}", self.name));
            let join = std::thread::Builder::new()
                .name(format!("pb-{}-{}", self.name, id))
                .spawn(move || {
                    let status = ctx.status.clone();
                    run(ctx);
                    update_status(&status, |s| {
                        s.connected = false;
                        if s.state != "error" {
                            s.state = "stopped".into();
                        }
                    });
                })
                .ok();
            self.workers.push(Worker { id, endpoint, key, stop, tx, status, out_points, join });
        }
    }

    /// Host egress: route to every worker with an Output bind for `name`.
    pub fn write_point(&mut self, name: &str, value: &str) -> i32 {
        self.latest.insert(name.to_string(), value.to_string());
        let mut n = 0;
        for w in &self.workers {
            if w.out_points.iter().any(|p| p == name) && w.tx.send((name.to_string(), value.to_string())).is_ok() {
                n += 1;
            }
        }
        if n > 0 {
            1
        } else {
            0
        }
    }

    fn statuses(&self) -> Vec<(String, String, LinkStatus)> {
        self.workers
            .iter()
            .map(|w| (w.id.clone(), w.endpoint.clone(), w.status.lock().map(|s| s.clone()).unwrap_or_default()))
            .collect()
    }

    pub fn state_label(&self) -> String {
        if !self.config_error.is_empty() {
            return "config error".into();
        }
        let st = self.statuses();
        if st.is_empty() {
            return if self.enabled { "idle".into() } else { "stopped".into() };
        }
        format!("{}/{} links up", st.iter().filter(|(_, _, s)| s.connected).count(), st.len())
    }

    pub fn last_error(&self) -> String {
        if !self.config_error.is_empty() {
            return self.config_error.clone();
        }
        self.statuses()
            .into_iter()
            .find(|(_, _, s)| !s.error.is_empty())
            .map(|(id, _, s)| format!("{id}: {}", s.error))
            .unwrap_or_default()
    }

    pub fn is_connected(&self) -> bool {
        self.statuses().iter().any(|(_, _, s)| s.connected)
    }

    pub fn detail_json(&self) -> String {
        let conns: Vec<Value> = self
            .statuses()
            .into_iter()
            .map(|(id, endpoint, s)| {
                let seen: Vec<Value> = s
                    .seen()
                    .map(|x| serde_json::json!({"leaf": x.leaf, "label": x.label, "value": x.value, "at_ms": x.at_ms}))
                    .collect();
                serde_json::json!({
                    "id": id, "endpoint": endpoint, "state": s.state, "connected": s.connected,
                    "error": s.error, "rx": s.rx, "tx": s.tx, "errors": s.errors, "peers": s.peers,
                    "seen": seen,
                })
            })
            .collect();
        serde_json::json!({ "connections": conns }).to_string()
    }
}

impl Drop for ExtPack {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Implement `pb_abi::Pack` for a newtype around [`ExtPack`]:
///
/// ```ignore
/// pub struct PackState(pb_extkit::ExtPack);
/// pb_extkit::ext_pack_impl!(PackState, b"wits\0", b"WITS\0", "wits", run, endpoint, check);
/// ```
#[macro_export]
macro_rules! ext_pack_impl {
    ($ty:ident, $id:expr, $title:expr, $name:expr, $run:expr, $endpoint:expr, $check:expr) => {
        impl pb_abi::Pack for $ty {
            const ID: &'static [u8] = $id;
            const TITLE: &'static [u8] = $title;
            fn new(host: pb_abi::HostRef) -> Self {
                $ty($crate::ExtPack::new(host, $name, $run, $endpoint, $check))
            }
            fn configure(&mut self, json: &str) -> Result<(), String> {
                self.0.configure(json)
            }
            fn config_failed(&mut self, err: String) {
                self.0.config_failed(err)
            }
            fn set_enabled(&mut self, on: bool) {
                self.0.set_enabled(on)
            }
            fn shutdown(&mut self) {
                self.0.shutdown()
            }
            fn write_point(&mut self, name: &str, _kind: i32, bytes: &[u8]) -> std::os::raw::c_int {
                self.0.write_point(name, &pb_abi::value_text(bytes))
            }
            fn state_label(&self) -> String {
                self.0.state_label()
            }
            fn last_error(&self) -> String {
                self.0.last_error()
            }
            fn is_connected(&self) -> bool {
                self.0.is_connected()
            }
            fn detail_json(&self) -> String {
                self.0.detail_json()
            }
        }
    };
}

// ---------------------------------------------------------------------------
// Test host
// ---------------------------------------------------------------------------

/// A capturing host for pack tests: every upsert lands in a leaked, shared log.
pub mod testhost {
    use super::Val;
    use pb_abi::{HostRef, PbHostApi, PB_ABI_VERSION, PB_VALUE_BOOL, PB_VALUE_F64, PB_VALUE_I64};
    use std::ffi::CStr;
    use std::os::raw::{c_char, c_void};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    #[derive(Debug, Clone)]
    pub struct Rec {
        pub name: String,
        pub source: String,
        pub value: Val,
        pub quality: i32,
    }

    pub type Log = Mutex<Vec<Rec>>;

    unsafe extern "C" fn cap(ctx: *mut c_void, name: *const c_char, kind: i32, value: *const c_void, len: usize, quality: i32, src: *const c_char) {
        let log = &*(ctx as *const Log);
        let name = CStr::from_ptr(name).to_string_lossy().into_owned();
        let source = if src.is_null() { String::new() } else { CStr::from_ptr(src).to_string_lossy().into_owned() };
        let v = match kind {
            PB_VALUE_BOOL => Val::Bool(*(value as *const u8) != 0),
            PB_VALUE_I64 => Val::Int(*(value as *const i64)),
            PB_VALUE_F64 => Val::Float(*(value as *const f64)),
            _ => Val::Text(String::from_utf8_lossy(std::slice::from_raw_parts(value as *const u8, len)).into_owned()),
        };
        log.lock().unwrap().push(Rec { name, source, value: v, quality });
    }

    /// A new log and a host that writes into it.
    pub fn host() -> (&'static Log, HostRef) {
        let log: &'static Log = Box::leak(Box::new(Mutex::new(Vec::new())));
        let h = HostRef::new(PbHostApi { abi_version: PB_ABI_VERSION, ctx: log as *const Log as *mut c_void, upsert_point: Some(cap), log_message: None });
        (log, h)
    }

    /// Wait (up to 8 s) until `f` holds for the log; panics with the log otherwise.
    pub fn wait(log: &'static Log, what: &str, f: impl Fn(&[Rec]) -> bool) {
        let end = Instant::now() + Duration::from_secs(8);
        while Instant::now() < end {
            if f(&log.lock().unwrap()) {
                return;
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        panic!("timed out waiting for {what}; log = {:?}", log.lock().unwrap());
    }

    /// Latest good value for a point.
    pub fn last_good(log: &'static Log, name: &str) -> Option<Val> {
        log.lock().unwrap().iter().rev().find(|r| r.name == name && r.quality == pb_abi::PB_QUALITY_GOOD).map(|r| r.value.clone())
    }

    /// A free localhost TCP port.
    pub fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    static RUNS: AtomicUsize = AtomicUsize::new(0);

    fn echo_run(mut ctx: LinkCtx) {
        RUNS.fetch_add(1, Ordering::SeqCst);
        ctx.up("echo");
        // Every Input publishes its bind's "value" field; every write is mirrored to inputs whose
        // "mirror" field names the written point.
        for b in &ctx.inputs {
            ctx.publish(b, &Val::parse_text(&b.text("value")));
        }
        while !ctx.stopped() {
            for (p, v) in ctx.wait_writes(Duration::from_millis(50)) {
                for b in ctx.inputs.iter().filter(|b| b.text("mirror") == p) {
                    ctx.publish(b, &Val::parse_text(&v));
                }
                ctx.status(|s| s.tx += 1);
            }
        }
    }

    fn ep(c: &Map<String, Value>) -> String {
        c.s("host", "?")
    }

    fn check(c: &Map<String, Value>) -> Result<(), String> {
        if c.s("host", "") == "bad" {
            Err("bad host".into())
        } else {
            Ok(())
        }
    }

    fn cfg(host: &str, value: &str) -> String {
        serde_json::json!({
            "connections": [{"id":"a","host":host}, {"id":"off","host":"x","enabled":false}],
            "inputs": [{"connection":"a","interval_secs":0.5,"binds":[
                {"value":value,"point":"p1","source":"t/a::1"},
                {"value":"x","mirror":"out","point":"","source":"t/a::2"}]}],
            "outputs": [{"connection":"a","binds":[{"point":"out"},{"point":""}]}]
        })
        .to_string()
    }

    #[test]
    fn workers_start_restart_only_on_change_and_route_writes() {
        let (log, host) = testhost::host();
        let mut p = ExtPack::new(host, "test", echo_run, ep, check);
        let before = RUNS.load(Ordering::SeqCst);
        p.configure(&cfg("h", "42")).unwrap();
        testhost::wait(log, "p1", |l| l.iter().any(|r| r.name == "p1" && r.value == Val::Int(42)));
        assert_eq!(p.statuses().len(), 1, "disabled connection gets no worker");
        assert!(p.is_connected());
        // Same config: no restart.
        p.configure(&cfg("h", "42")).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(RUNS.load(Ordering::SeqCst) - before, 1);
        // Write routing: only the bound point reaches the worker; blank-point Input uses the source as name.
        assert_eq!(p.write_point("out", "7.5"), 1);
        assert_eq!(p.write_point("other", "1"), 0);
        testhost::wait(log, "mirror", |l| l.iter().any(|r| r.name == "t/a::2" && r.value == Val::Float(7.5)));
        // Changed config restarts; the latest written value is handed to the new worker.
        p.configure(&cfg("h", "43")).unwrap();
        testhost::wait(log, "p1 = 43", |l| l.iter().any(|r| r.name == "p1" && r.value == Val::Int(43)));
        assert_eq!(RUNS.load(Ordering::SeqCst) - before, 2);
        let d: Value = serde_json::from_str(&p.detail_json()).unwrap();
        assert_eq!(d["connections"][0]["id"], "a");
        p.set_enabled(false);
        assert!(!p.is_connected());
        assert_eq!(p.state_label(), "stopped");
    }

    #[test]
    fn check_failures_show_without_a_thread_and_bad_json_is_an_error() {
        let (_log, host) = testhost::host();
        let mut p = ExtPack::new(host, "test", echo_run, ep, check);
        p.configure(&cfg("bad", "1")).unwrap();
        assert_eq!(p.last_error(), "a: bad host");
        assert!(p.configure("{nope").is_err());
        assert!(p.configure(r#"{"connections":[3]}"#).is_err());
    }

    #[test]
    fn values_parse_and_convert() {
        assert_eq!(Val::parse_text("true"), Val::Bool(true));
        assert_eq!(Val::parse_text(" 12 "), Val::Int(12));
        assert_eq!(Val::parse_text("1.5"), Val::Float(1.5));
        assert_eq!(Val::parse_text("abc"), Val::Text("abc".into()));
        assert_eq!(Val::parse_text("NaN"), Val::Text("NaN".into()));
        assert_eq!(Val::Text("on".into()).as_bool(), Some(true));
        assert_eq!(Val::from_json(&serde_json::json!(3)), Val::Int(3));
        assert_eq!(Val::from_json(&serde_json::json!(3.5)), Val::Float(3.5));
        assert_eq!(Val::Float(2.0).to_json(), serde_json::json!(2.0));
        let mut st = LinkStatus::default();
        for i in 0..(SEEN_MAX + 10) {
            st.see(&i.to_string(), leaf1("k", i), "l", "v");
        }
        assert_eq!(st.seen().count(), SEEN_MAX);
    }

    #[test]
    fn bind_field_helpers() {
        let b = Bind {
            fields: serde_json::json!({"scale":"0.5","offset":2,"flag":"yes","n":"x"}).as_object().unwrap().clone(),
            point: "p".into(),
            source: String::new(),
            interval: Duration::from_secs(1),
        };
        assert_eq!(b.scaling(), (0.5, 2.0));
        assert!(b.flag("flag"));
        assert_eq!(b.num("n"), None);
        assert_eq!(b.text("offset"), "2");
    }
}
