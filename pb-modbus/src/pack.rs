//! C-ABI pack. One worker thread per **client** connection: connect, poll Input groups,
//! apply typed writes, mark points bad on link loss and reconnect with backoff.
//! (Server connections are handled in the next phase.)

use std::os::raw::c_int;
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pb_abi::worker::{sleep_stop, update_status};
use pb_abi::{value_text, HostRef, Pack, PB_LOG_ERROR, PB_LOG_INFO, PB_LOG_WARN, PB_QUALITY_BAD, PB_QUALITY_GOOD};
use serde::{Deserialize, Serialize};

use crate::bank::RegisterBank;
use crate::client::{open_serial, LinkConfig, Session};
use crate::server::{run_tcp_listener, serve_rtu, ServeCtx};
use crate::codec::{self, Area, ByteOrder, DataType, Value};
use crate::frame::{ModbusError, Request, Response};

const PACK_ID: &[u8] = b"modbus\0";
const PACK_TITLE: &[u8] = b"Modbus\0";
/// Registers of slack allowed inside one merged read block.
const READ_GAP: u16 = 4;

// ---------------------------------------------------------------------------
// Config (JSON from the service)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PackConfig {
    #[serde(default)]
    connections: Vec<ConnJson>,
    #[serde(default)]
    inputs: Vec<InputJson>,
    #[serde(default)]
    outputs: Vec<OutputJson>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct ConnJson {
    id: String,
    /// `client` | `server`.
    #[serde(default = "default_role")]
    role: String,
    #[serde(flatten)]
    link: LinkConfig,
    #[serde(default = "default_unit")]
    unit_id: u8,
    #[serde(default = "default_retries")]
    retries: u8,
    /// Server only: register image (host point served at each address).
    #[serde(default)]
    map: Vec<BindJson>,
    /// Server only: the service sets this false for servers no hub uses.
    #[serde(default = "default_true")]
    enabled: bool,
}

fn default_true() -> bool {
    true
}

fn default_role() -> String {
    "client".into()
}
fn default_unit() -> u8 {
    1
}
fn default_retries() -> u8 {
    3
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct BindJson {
    area: String,
    address: u16,
    dtype: String,
    #[serde(default = "default_order")]
    order: String,
    /// 0 = connection default.
    #[serde(default)]
    unit: u8,
    #[serde(default)]
    point: String,
    #[serde(default)]
    source: String,
    /// Engineering scaling: `value = raw × scale + offset` (identity when absent).
    #[serde(default = "default_scale")]
    scale: f64,
    #[serde(default)]
    offset: f64,
}

fn default_scale() -> f64 {
    1.0
}

fn default_order() -> String {
    "abcd".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct InputJson {
    connection: String,
    #[serde(default = "default_interval")]
    interval_secs: f64,
    binds: Vec<BindJson>,
}

fn default_interval() -> f64 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct OutputJson {
    connection: String,
    binds: Vec<BindJson>,
}

// ---------------------------------------------------------------------------
// Host callbacks
// ---------------------------------------------------------------------------

/// Convert a decoded register value into a host point.
trait PutValue {
    fn put(&self, name: &str, source: &str, v: &Value, quality: i32);
}

impl PutValue for HostRef {
    fn put(&self, name: &str, source: &str, v: &Value, quality: i32) {
        match v {
            Value::Bool(b) => self.upsert_bool(name, source, *b, quality),
            Value::Int(n) => self.upsert_i64(name, source, *n, quality),
            Value::UInt(n) => self.upsert_u64(name, source, *n, quality),
            Value::Float(x) => self.upsert_f64(name, source, *x, quality),
            Value::Text(t) => self.upsert_text(name, source, t, quality),
        }
    }
}

// ---------------------------------------------------------------------------
// Runtime binds
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Bind {
    unit: u8,
    area: Area,
    address: u16,
    dt: DataType,
    order: ByteOrder,
    name: String,
    source: String,
    scaling: codec::Scaling,
    last: Option<Value>,
}

impl Bind {
    fn width(&self) -> u16 {
        if self.area.is_bit() { 1 } else { self.dt.width() }
    }

    fn describe(&self) -> String {
        format!("unit {} {:?} {} {}", self.unit, self.area, self.address, self.dt.as_string())
    }
}

fn build_bind(b: &BindJson, default_unit: u8, conn: &str) -> Result<Bind, String> {
    let area = Area::parse(&b.area).ok_or_else(|| format!("unknown area \"{}\"", b.area))?;
    let dt = DataType::parse(&b.dtype).ok_or_else(|| format!("unknown type \"{}\"", b.dtype))?;
    let order = ByteOrder::parse(&b.order).ok_or_else(|| format!("unknown order \"{}\"", b.order))?;
    if area.is_bit() != (dt == DataType::Bool) {
        return Err(format!("{} cannot hold {}", b.area, b.dtype));
    }
    if b.address as u32 + dt.width() as u32 > 65_536 {
        return Err(format!("address {} + {} runs past 65535", b.address, dt.width()));
    }
    let source = if b.source.is_empty() {
        format!("modbus/{conn}::{}:{}:{}", b.area, b.address, b.dtype)
    } else {
        b.source.clone()
    };
    let name = if b.point.trim().is_empty() { source.clone() } else { b.point.trim().to_string() };
    Ok(Bind {
        unit: if b.unit == 0 { default_unit } else { b.unit },
        area,
        address: b.address,
        dt,
        order,
        name,
        source,
        scaling: codec::Scaling::new(b.scale, b.offset),
        last: None,
    })
}

struct Group {
    interval: Duration,
    next_due: Instant,
    binds: Vec<Bind>,
    blocks: Vec<codec::ReadBlock>,
}

struct WriteCmd {
    bind: Bind,
    raw: String,
}

#[derive(Default, Clone)]
struct Status {
    state: String,
    error: String,
    connected: bool,
    polls: u64,
    errors: u64,
}

// ---------------------------------------------------------------------------
// Worker
// ---------------------------------------------------------------------------

struct Worker {
    id: String,
    endpoint: String,
    key: String,
    stop: Arc<AtomicBool>,
    tx: Sender<WriteCmd>,
    status: Arc<Mutex<Status>>,
    outputs: Vec<Bind>,
    /// Present for server connections.
    server: Option<ServerRt>,
    join: Option<JoinHandle<()>>,
}

struct ServerRt {
    bank: Arc<Mutex<RegisterBank>>,
    entries: Arc<Vec<Bind>>,
    clients: Arc<AtomicUsize>,
}

impl Worker {
    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

fn run_worker(
    host: HostRef,
    conn: ConnJson,
    mut groups: Vec<Group>,
    rx: Receiver<WriteCmd>,
    stop: Arc<AtomicBool>,
    status: Arc<Mutex<Status>>,
) {
    let mut backoff = Duration::from_millis(500);
    'outer: while !stop.load(Ordering::SeqCst) {
        update_status(&status, |s| s.state = "connecting".into());
        let mut session = match Session::open(&conn.link) {
            Ok(s) => s,
            Err(e) => {
                link_down(&host, &mut groups, &status, &conn, &e);
                if !sleep_stop(&stop, backoff) {
                    break;
                }
                backoff = (backoff * 2).min(Duration::from_secs(10));
                continue;
            }
        };
        backoff = Duration::from_millis(500);
        update_status(&status, |s| {
            s.connected = true;
            s.state = "connected".into();
            s.error.clear();
        });
        host.log(PB_LOG_INFO, &format!("modbus {}: connected {}", conn.id, conn.link.endpoint()));
        for g in &mut groups {
            g.next_due = Instant::now();
        }
        while !stop.load(Ordering::SeqCst) {
            // Writes first: they are operator-visible actions.
            while let Ok(cmd) = rx.try_recv() {
                if let Err(e) = exec_write(&mut session, &conn, &cmd) {
                    if is_link_error(&e) {
                        link_down(&host, &mut groups, &status, &conn, &e);
                        continue 'outer;
                    }
                    note_error(&host, &status, &conn, &format!("write {}: {e}", cmd.bind.describe()));
                }
            }
            let now = Instant::now();
            for g in &mut groups {
                if now < g.next_due {
                    continue;
                }
                g.next_due = now + g.interval;
                if let Err(e) = poll_group(&mut session, &conn, g, &host, &status) {
                    link_down(&host, &mut groups, &status, &conn, &e);
                    continue 'outer;
                }
                update_status(&status, |s| s.polls += 1);
            }
            // Sleep until the next poll, but wake early for a queued write.
            let wait = groups
                .iter()
                .map(|g| g.next_due.saturating_duration_since(Instant::now()))
                .min()
                .unwrap_or(Duration::from_millis(100))
                .min(Duration::from_millis(100));
            match rx.recv_timeout(wait) {
                Ok(cmd) => {
                    if let Err(e) = exec_write(&mut session, &conn, &cmd) {
                        if is_link_error(&e) {
                            link_down(&host, &mut groups, &status, &conn, &e);
                            continue 'outer;
                        }
                        note_error(&host, &status, &conn, &format!("write {}: {e}", cmd.bind.describe()));
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break 'outer,
            }
        }
    }
    update_status(&status, |s| {
        s.connected = false;
        s.state = "stopped".into();
    });
}

fn is_link_error(e: &ModbusError) -> bool {
    !matches!(e, ModbusError::Exception(_))
}

fn note_error(host: &HostRef, status: &Arc<Mutex<Status>>, conn: &ConnJson, msg: &str) {
    host.log(PB_LOG_WARN, &format!("modbus {}: {msg}", conn.id));
    update_status(status, |s| {
        s.error = msg.to_string();
        s.errors += 1;
    });
}

/// Link lost / never up: every bound point goes bad (value held), state shows the cause.
fn link_down(host: &HostRef, groups: &mut [Group], status: &Arc<Mutex<Status>>, conn: &ConnJson, e: &ModbusError) {
    for g in groups.iter_mut() {
        for b in &g.binds {
            host.put(&b.name, &b.source, b.last.as_ref().unwrap_or(&Value::Text(String::new())), PB_QUALITY_BAD);
        }
    }
    host.log(PB_LOG_ERROR, &format!("modbus {}: {} — {e}", conn.id, conn.link.endpoint()));
    update_status(status, |s| {
        s.connected = false;
        s.state = "error".into();
        s.error = format!("{}: {e}", conn.link.endpoint());
        s.errors += 1;
    });
}

/// One request with the connection's retry budget. Exceptions are never retried.
fn request(session: &mut Session, conn: &ConnJson, unit: u8, req: &Request) -> Result<Response, ModbusError> {
    let mut last = ModbusError::Timeout;
    for _ in 0..=conn.retries {
        match session.transact(unit, req) {
            Ok(r) => return Ok(r),
            Err(e @ ModbusError::Exception(_)) => return Err(e),
            Err(e @ ModbusError::Io(_)) => return Err(e),
            Err(e) => last = e,
        }
    }
    Err(last)
}

fn poll_group(
    session: &mut Session,
    conn: &ConnJson,
    g: &mut Group,
    host: &HostRef,
    status: &Arc<Mutex<Status>>,
) -> Result<(), ModbusError> {
    let blocks = g.blocks.clone();
    for blk in &blocks {
        let req = if blk.area.is_bit() {
            Request::ReadBits { area: blk.area, start: blk.start, count: blk.count }
        } else {
            Request::ReadRegs { area: blk.area, start: blk.start, count: blk.count }
        };
        match request(session, conn, blk.unit, &req) {
            Ok(resp) => {
                for (key, off) in &blk.items {
                    let b = &mut g.binds[*key];
                    let off = *off as usize;
                    let decoded = match &resp {
                        Response::Bits(bits) => bits.get(off).map(|v| Ok(codec::decode_bit(*v))),
                        Response::Regs(regs) => regs
                            .get(off..off + b.width() as usize)
                            .map(|w| codec::decode(b.dt, b.order, w)),
                        Response::Written => None,
                    };
                    match decoded {
                        Some(Ok(v)) => {
                            let v = b.scaling.apply(v);
                            host.put(&b.name, &b.source, &v, PB_QUALITY_GOOD);
                            b.last = Some(v);
                        }
                        _ => host.put(&b.name, &b.source, b.last.as_ref().unwrap_or(&Value::Text(String::new())), PB_QUALITY_BAD),
                    }
                }
                update_status(status, |s| {
                    if s.state != "connected" {
                        s.state = "connected".into();
                    }
                });
            }
            Err(ModbusError::Exception(code)) => {
                // Device is alive but refused this range: those points go bad, link stays up.
                for (key, _) in &blk.items {
                    let b = &g.binds[*key];
                    host.put(&b.name, &b.source, b.last.as_ref().unwrap_or(&Value::Text(String::new())), PB_QUALITY_BAD);
                }
                note_error(
                    host,
                    status,
                    conn,
                    &format!(
                        "unit {} {:?} {}..{}: {}",
                        blk.unit,
                        blk.area,
                        blk.start,
                        blk.start as u32 + blk.count as u32 - 1,
                        ModbusError::Exception(code)
                    ),
                );
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn exec_write(session: &mut Session, conn: &ConnJson, cmd: &WriteCmd) -> Result<(), ModbusError> {
    let b = &cmd.bind;
    let raw = b.scaling.unapply_text(b.dt, &cmd.raw).map_err(|e| ModbusError::Protocol(e.to_string()))?;
    let value = codec::parse_point_value(b.dt, &raw).map_err(|e| ModbusError::Protocol(e.to_string()))?;
    let unit = b.unit;
    match (b.area, b.dt) {
        (Area::Coil, _) => {
            let on = codec::encode_bit(&value).map_err(|e| ModbusError::Protocol(e.to_string()))?;
            request(session, conn, unit, &Request::WriteCoil { addr: b.address, on })?;
        }
        (Area::Holding, DataType::Bit(bit)) => {
            let on = codec::encode_bit(&value).map_err(|e| ModbusError::Protocol(e.to_string()))?;
            let cur = match request(session, conn, unit, &Request::ReadRegs { area: Area::Holding, start: b.address, count: 1 })? {
                Response::Regs(r) if r.len() == 1 => r[0],
                _ => return Err(ModbusError::Protocol("bad read-modify-write reply".into())),
            };
            request(session, conn, unit, &Request::WriteReg { addr: b.address, value: codec::merge_bit(cur, bit, on) })?;
        }
        (Area::Holding, dt) => {
            let words = codec::encode(dt, b.order, &value).map_err(|e| ModbusError::Protocol(e.to_string()))?;
            let req = if words.len() == 1 {
                Request::WriteReg { addr: b.address, value: words[0] }
            } else {
                Request::WriteRegs { start: b.address, values: words }
            };
            request(session, conn, unit, &req)?;
        }
        (area, _) => {
            return Err(ModbusError::Protocol(format!("{area:?} is read-only")));
        }
    }
    Ok(())
}


// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

/// Reject entries whose address ranges overlap within one area.
fn map_overlap(entries: &[Bind]) -> Option<String> {
    for (i, a) in entries.iter().enumerate() {
        for b in &entries[i + 1..] {
            if a.area == b.area {
                let (a0, a1) = (a.address as u32, a.address as u32 + a.width() as u32);
                let (b0, b1) = (b.address as u32, b.address as u32 + b.width() as u32);
                if a0 < b1 && b0 < a1 {
                    return Some(format!("{} overlaps {}", a.describe(), b.describe()));
                }
            }
        }
    }
    None
}

/// Host point value → register image.
fn push_to_bank(bank: &mut RegisterBank, b: &Bind, raw: &str) -> Result<(), String> {
    let raw = b.scaling.unapply_text(b.dt, raw).map_err(|e| e.to_string())?;
    let value = codec::parse_point_value(b.dt, &raw).map_err(|e| e.to_string())?;
    match (b.area, b.dt) {
        (Area::Coil, _) | (Area::Discrete, _) => {
            let on = codec::encode_bit(&value).map_err(|e| e.to_string())?;
            if b.area == Area::Coil {
                bank.coils.insert(b.address, on);
            } else {
                bank.discrete.insert(b.address, on);
            }
        }
        (area, DataType::Bit(bit)) => {
            let on = codec::encode_bit(&value).map_err(|e| e.to_string())?;
            let cur = bank.regs(area, b.address, 1).map(|r| r[0]).unwrap_or(0);
            bank.set_regs(area, b.address, &[codec::merge_bit(cur, bit, on)]);
        }
        (area, dt) => {
            let words = codec::encode(dt, b.order, &value).map_err(|e| e.to_string())?;
            bank.set_regs(area, b.address, &words);
        }
    }
    Ok(())
}

/// Read one entry back out of the image and publish it as a host point.
fn publish_entry(host: &HostRef, bank: &RegisterBank, b: &Bind) {
    let value = if b.area.is_bit() {
        bank.bits(b.area, b.address, 1).map(|v| Value::Bool(v[0]))
    } else {
        bank.regs(b.area, b.address, b.width()).and_then(|w| codec::decode(b.dt, b.order, &w).ok())
    }
    .map(|v| b.scaling.apply(v));
    if let Some(v) = value {
        // Empty source: keep whatever address the point store already has for this point.
        host.put(&b.name, "", &v, PB_QUALITY_GOOD);
    }
}

/// A master wrote registers / coils: mirror every overlapped map entry into host points.
fn on_master_write(host: &HostRef, bank: &Mutex<RegisterBank>, entries: &[Bind], req: &Request) {
    let (family, start, len) = match req {
        Request::WriteCoil { addr, .. } => (Area::Coil, *addr as u32, 1u32),
        Request::WriteCoils { start, values } => (Area::Coil, *start as u32, values.len() as u32),
        Request::WriteReg { addr, .. } => (Area::Holding, *addr as u32, 1),
        Request::WriteRegs { start, values } => (Area::Holding, *start as u32, values.len() as u32),
        _ => return,
    };
    let Ok(bank) = bank.lock() else { return };
    for b in entries.iter().filter(|b| b.area == family) {
        let (b0, b1) = (b.address as u32, b.address as u32 + b.width() as u32);
        if b0 < start + len && start < b1 {
            publish_entry(host, &bank, b);
        }
    }
}

fn run_server(host: HostRef, conn: ConnJson, ctx: ServeCtx, status: Arc<Mutex<Status>>) {
    let endpoint = conn.link.endpoint();
    while !ctx.stop.load(Ordering::SeqCst) {
        if conn.link.is_rtu() {
            match open_serial(&conn.link) {
                Ok(mut port) => {
                    let _ = port.set_timeout(Duration::from_millis(20));
                    update_status(&status, |s| {
                        s.connected = true;
                        s.state = format!("slave {endpoint} unit {}", conn.unit_id);
                        s.error.clear();
                    });
                    host.log(PB_LOG_INFO, &format!("modbus {}: RTU slave on {endpoint}", conn.id));
                    if let Err(e) = serve_rtu(&mut port, &ctx, Duration::from_millis(100)) {
                        note_server_error(&host, &status, &conn, &format!("{endpoint}: {e}"));
                    }
                }
                Err(e) => note_server_error(&host, &status, &conn, &format!("{e}")),
            }
        } else {
            match TcpListener::bind((conn.link.host.trim(), conn.link.port)) {
                Ok(l) => {
                    update_status(&status, |s| {
                        s.connected = true;
                        s.state = format!("listening {endpoint}");
                        s.error.clear();
                    });
                    host.log(PB_LOG_INFO, &format!("modbus {}: listening on {endpoint}", conn.id));
                    run_tcp_listener(l, ctx.clone());
                }
                Err(e) => note_server_error(&host, &status, &conn, &format!("bind {endpoint}: {e}")),
            }
        }
        if !ctx.stop.load(Ordering::SeqCst) {
            update_status(&status, |s| s.connected = false);
            if !sleep_stop(&ctx.stop, Duration::from_secs(2)) {
                break;
            }
        }
    }
    update_status(&status, |s| {
        s.connected = false;
        s.state = "stopped".into();
    });
}

fn note_server_error(host: &HostRef, status: &Arc<Mutex<Status>>, conn: &ConnJson, msg: &str) {
    host.log(PB_LOG_ERROR, &format!("modbus {}: {msg}", conn.id));
    update_status(status, |s| {
        s.connected = false;
        s.state = "error".into();
        s.error = msg.to_string();
        s.errors += 1;
    });
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
        let cfg: PackConfig = serde_json::from_str(text).map_err(|e| format!("json: {e}"))?;
        self.config = cfg;
        self.sync_workers();
        Ok(())
    }

    /// Start / stop / restart workers so they match the config. Unchanged connections keep running.
    fn sync_workers(&mut self) {
        self.config_error.clear();
        let want: Vec<ConnJson> = if self.enabled {
            self.config
                .connections
                .iter()
                .filter(|c| c.enabled && (c.role.eq_ignore_ascii_case("client") || c.role.eq_ignore_ascii_case("server")))
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        let mut keep: Vec<Worker> = Vec::new();
        let mut old = std::mem::take(&mut self.workers);
        let mut next: Vec<(ConnJson, String)> = Vec::new();
        for c in want {
            let inputs: Vec<&InputJson> = self.config.inputs.iter().filter(|i| i.connection == c.id).collect();
            let outputs: Vec<&OutputJson> = self.config.outputs.iter().filter(|o| o.connection == c.id).collect();
            let key = serde_json::to_string(&(&c, &inputs, &outputs)).unwrap_or_default();
            if let Some(pos) = old.iter().position(|w| w.id == c.id && w.key == key) {
                keep.push(old.remove(pos));
            } else {
                next.push((c, key));
            }
        }
        for w in &mut old {
            w.stop.store(true, Ordering::SeqCst);
        }
        for w in &mut old {
            w.shutdown();
        }
        for (c, key) in next {
            let spawned = if c.role.eq_ignore_ascii_case("server") {
                self.spawn_server(c, key)
            } else {
                self.spawn_worker(c, key)
            };
            match spawned {
                Ok(w) => keep.push(w),
                Err(e) => {
                    self.host.log(PB_LOG_ERROR, &format!("modbus: {e}"));
                    self.config_error = e;
                }
            }
        }
        self.workers = keep;
        self.host.log(PB_LOG_INFO, &format!("modbus configured: {} link(s)", self.workers.len()));
    }

    fn spawn_worker(&self, conn: ConnJson, key: String) -> Result<Worker, String> {
        let mut groups = Vec::new();
        for inp in self.config.inputs.iter().filter(|i| i.connection == conn.id) {
            let mut binds = Vec::new();
            for b in &inp.binds {
                binds.push(build_bind(b, conn.unit_id, &conn.id).map_err(|e| format!("{}: {e}", conn.id))?);
            }
            if binds.is_empty() {
                continue;
            }
            let items: Vec<codec::ReadItem> = binds
                .iter()
                .enumerate()
                .map(|(key, b)| codec::ReadItem { key, unit: b.unit, area: b.area, address: b.address, width: b.width() })
                .collect();
            groups.push(Group {
                interval: Duration::from_secs_f64(inp.interval_secs.clamp(0.05, 3600.0)),
                next_due: Instant::now(),
                blocks: codec::plan_reads(&items, READ_GAP),
                binds,
            });
        }
        let mut outputs = Vec::new();
        for out in self.config.outputs.iter().filter(|o| o.connection == conn.id) {
            for b in &out.binds {
                if b.point.trim().is_empty() {
                    continue;
                }
                let bind = build_bind(b, conn.unit_id, &conn.id).map_err(|e| format!("{}: {e}", conn.id))?;
                if !bind.area.writable() {
                    return Err(format!("{}: {:?} is read-only", conn.id, bind.area));
                }
                outputs.push(bind);
            }
        }
        let (tx, rx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new(Status { state: "starting".into(), ..Default::default() }));
        let (h, c, st, sp) = (self.host, conn.clone(), status.clone(), stop.clone());
        let join = std::thread::Builder::new()
            .name(format!("pb-modbus-{}", conn.id))
            .spawn(move || run_worker(h, c, groups, rx, sp, st))
            .map_err(|e| format!("spawn: {e}"))?;
        Ok(Worker { endpoint: conn.link.endpoint(), id: conn.id, key, stop, tx, status, outputs, server: None, join: Some(join) })
    }

    fn spawn_server(&self, conn: ConnJson, key: String) -> Result<Worker, String> {
        let mut entries = Vec::new();
        let mut bank = RegisterBank::default();
        for b in &conn.map {
            if b.point.trim().is_empty() {
                continue;
            }
            let bind = build_bind(b, conn.unit_id, &conn.id).map_err(|e| format!("{}: {e}", conn.id))?;
            bank.define(bind.area, bind.address, bind.width());
            entries.push(bind);
        }
        if let Some(msg) = map_overlap(&entries) {
            return Err(format!("{}: register map {msg}", conn.id));
        }
        let entries = Arc::new(entries);
        let bank = Arc::new(Mutex::new(bank));
        let stop = Arc::new(AtomicBool::new(false));
        let clients = Arc::new(AtomicUsize::new(0));
        let mut ctx = ServeCtx::new(bank.clone(), conn.unit_id);
        ctx.stop = stop.clone();
        ctx.clients = clients.clone();
        let (host, hb, he) = (self.host, bank.clone(), entries.clone());
        ctx.on_write = Some(Arc::new(move |req| on_master_write(&host, &hb, &he, req)));
        let status = Arc::new(Mutex::new(Status { state: "starting".into(), ..Default::default() }));
        let (h, c, st) = (self.host, conn.clone(), status.clone());
        let join = std::thread::Builder::new()
            .name(format!("pb-modbus-srv-{}", conn.id))
            .spawn(move || run_server(h, c, ctx, st))
            .map_err(|e| format!("spawn: {e}"))?;
        let (tx, _rx) = channel();
        Ok(Worker {
            endpoint: conn.link.endpoint(),
            id: conn.id,
            key,
            stop,
            tx,
            status,
            outputs: Vec::new(),
            server: Some(ServerRt { bank, entries, clients }),
            join: Some(join),
        })
    }

    fn write_point(&mut self, name: &str, raw: &str) {
        // The host sends an empty string for points that have no value yet — nothing to write
        // (and not an error) unless the register holds text.
        let blank = raw.trim().is_empty();
        for w in &self.workers {
            for b in w.outputs.iter().filter(|b| b.name == name && !(blank && !matches!(b.dt, DataType::Str(_)))) {
                let _ = w.tx.send(WriteCmd { bind: b.clone(), raw: raw.to_string() });
            }
            if let Some(srv) = &w.server {
                for b in srv.entries.iter().filter(|b| b.name == name && !(blank && !matches!(b.dt, DataType::Str(_)))) {
                    if let Ok(mut bank) = srv.bank.lock() {
                        if let Err(e) = push_to_bank(&mut bank, b, raw) {
                            update_status(&w.status, |s| {
                                s.error = format!("{}: {e}", b.describe());
                                s.errors += 1;
                            });
                        }
                    }
                }
            }
        }
    }

    fn statuses(&self) -> Vec<(String, Status)> {
        self.workers
            .iter()
            .map(|w| (w.id.clone(), w.status.lock().map(|s| s.clone()).unwrap_or_default()))
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

    fn write_point(&mut self, name: &str, _kind: i32, bytes: &[u8]) -> c_int {
        PackState::write_point(self, name, &value_text(bytes));
        0
    }

    fn state_label(&self) -> String {
        let st = self.statuses();
        if !self.config_error.is_empty() {
            return "config error".into();
        }
        if st.is_empty() {
            return if self.enabled { "idle".into() } else { "stopped".into() };
        }
        let up = st.iter().filter(|(_, s)| s.connected).count();
        format!("{up}/{} links up", st.len())
    }

    fn last_error(&self) -> String {
        if !self.config_error.is_empty() {
            return self.config_error.clone();
        }
        self.statuses()
            .into_iter()
            .find(|(_, s)| !s.error.is_empty())
            .map(|(id, s)| format!("{id}: {}", s.error))
            .unwrap_or_default()
    }

    fn is_connected(&self) -> bool {
        self.statuses().iter().any(|(_, s)| s.connected)
    }

    fn detail_json(&self) -> String {
        let conns: Vec<serde_json::Value> = self
            .workers
            .iter()
            .map(|w| {
                let s = w.status.lock().map(|s| s.clone()).unwrap_or_default();
                serde_json::json!({
                    "id": w.id,
                    "endpoint": w.endpoint,
                    "role": if w.server.is_some() { "server" } else { "client" },
                    "state": s.state, "connected": s.connected,
                    "error": s.error, "polls": s.polls, "errors": s.errors,
                    "clients": w.server.as_ref().map(|r| r.clients.load(Ordering::SeqCst)),
                })
            })
            .collect();
        serde_json::json!({ "connections": conns }).to_string()
    }
}

pb_abi::export_pack!(PackState);

#[cfg(test)]
mod tests {
    use super::*;
    use pb_abi::{PbHostApi, PbPackApi, PB_ABI_VERSION, PB_VALUE_BOOL, PB_VALUE_F64, PB_VALUE_I64};
    use std::ffi::CStr;
    use std::os::raw::{c_char, c_void};
    use crate::bank::RegisterBank;
    use crate::server;
    use std::net::TcpListener;
    use std::sync::Mutex as StdMutex;

    #[derive(Debug, Clone)]
    struct Rec {
        name: String,
        value: Value,
        quality: i32,
    }

    type Log = StdMutex<Vec<Rec>>;

    unsafe extern "C" fn cap_upsert(ctx: *mut c_void, name: *const c_char, kind: i32, value: *const c_void, len: usize, quality: i32, _src: *const c_char) {
        let log = &*(ctx as *const Log);
        let name = CStr::from_ptr(name).to_string_lossy().into_owned();
        let v = match kind {
            PB_VALUE_BOOL => Value::Bool(*(value as *const u8) != 0),
            PB_VALUE_I64 => Value::Int(*(value as *const i64)),
            PB_VALUE_F64 => Value::Float(*(value as *const f64)),
            _ => Value::Text(String::from_utf8_lossy(std::slice::from_raw_parts(value as *const u8, len)).into_owned()),
        };
        log.lock().unwrap().push(Rec { name, value: v, quality });
    }

    fn host(log: &'static Log) -> HostRef {
        HostRef::new(PbHostApi { abi_version: PB_ABI_VERSION, ctx: log as *const Log as *mut c_void, upsert_point: Some(cap_upsert), log_message: None })
    }

    fn leak_log() -> &'static Log {
        Box::leak(Box::new(StdMutex::new(Vec::new())))
    }

    fn wait_for(log: &'static Log, what: &str, f: impl Fn(&[Rec]) -> bool) {
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end {
            if f(&log.lock().unwrap()) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("timed out waiting for {what}; log = {:?}", log.lock().unwrap());
    }

    fn cfg(port: u16) -> String {
        serde_json::json!({
            "connections": [{"id":"plc","role":"client","transport":"tcp","host":"127.0.0.1","port":port,
                             "timeout_ms":200,"unit_id":1,"retries":1}],
            "inputs": [{"connection":"plc","interval_secs":0.05,"binds":[
                {"area":"holding","address":0,"dtype":"f32","order":"cdab","point":"modbus_1"},
                {"area":"holding","address":2,"dtype":"u32","point":"modbus_2"},
                {"area":"holding","address":4,"dtype":"bit3","point":"modbus_3"},
                {"area":"holding","address":5,"dtype":"str2","point":"modbus_4"},
                {"area":"coil","address":0,"dtype":"bool","point":"modbus_5"},
                {"area":"input","address":0,"dtype":"i16","order":"dcba","point":"modbus_6"}]}],
            "outputs": [{"connection":"plc","binds":[
                {"area":"holding","address":10,"dtype":"f32","order":"cdab","point":"setpoint"},
                {"area":"coil","address":3,"dtype":"bool","point":"run"},
                {"area":"holding","address":12,"dtype":"bit1","point":"flag"}]}]
        })
        .to_string()
    }

    fn device() -> Arc<Mutex<RegisterBank>> {
        let mut b = RegisterBank::default();
        b.define(Area::Holding, 0, 16);
        b.define(Area::Coil, 0, 8);
        b.define(Area::Input, 0, 2);
        // 123.456f32 as CDAB, u32 100000, bit3 set, "Hi!"
        b.set_regs(Area::Holding, 0, &[0xE979, 0x42F6, 0x0001, 0x86A0, 0b1000, 0x4869, 0x2100]);
        b.coils.insert(0, true);
        b.set_regs(Area::Input, 0, &[0x0100]); // i16 dcba -> 1
        Arc::new(Mutex::new(b))
    }

    #[test]
    fn polls_every_type_decodes_and_writes_back() {
        let bank = device();
        let addr = server::spawn_tcp_for_tests(bank.clone(), 1);
        let log = leak_log();
        let mut st = PackState::new(host(log));
        st.configure_json(&cfg(addr.port())).unwrap();
        wait_for(log, "all six points", |l| {
            ["modbus_1", "modbus_2", "modbus_3", "modbus_4", "modbus_5", "modbus_6"]
                .iter()
                .all(|n| l.iter().any(|r| r.name == *n && r.quality == PB_QUALITY_GOOD))
        });
        let last = |n: &str| log.lock().unwrap().iter().rev().find(|r| r.name == n).unwrap().value.clone();
        assert_eq!(last("modbus_1"), Value::Float(123.456));
        assert_eq!(last("modbus_2"), Value::Int(100_000));
        assert_eq!(last("modbus_3"), Value::Bool(true));
        assert_eq!(last("modbus_4"), Value::Text("Hi!".into()));
        assert_eq!(last("modbus_5"), Value::Bool(true));
        assert_eq!(last("modbus_6"), Value::Int(1));
        assert!(st.is_connected());

        // Typed writes: f32 cdab, coil, read-modify-write bit.
        st.write_point("setpoint", "98.6");
        st.write_point("run", "true");
        st.write_point("flag", "1");
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            let b = bank.lock().unwrap();
            if b.coils[&3] && b.holding[&12] == 0b10 {
                let w = b.regs(Area::Holding, 10, 2).unwrap();
                assert_eq!(codec::decode(DataType::F32, ByteOrder::Cdab, &w).unwrap(), Value::Float(98.6));
                break;
            }
            drop(b);
            assert!(Instant::now() < end, "writes never landed");
            std::thread::sleep(Duration::from_millis(20));
        }
        // Device-side change flows through on the next poll.
        bank.lock().unwrap().set_regs(Area::Holding, 2, &[0, 7]);
        wait_for(log, "u32 update", |l| l.iter().any(|r| r.name == "modbus_2" && r.value == Value::Int(7)));
    }

    #[test]
    fn scaled_binds_read_engineering_and_write_raw_counts() {
        let bank = device();
        bank.lock().unwrap().set_regs(Area::Holding, 7, &[1234]);
        let addr = server::spawn_tcp_for_tests(bank.clone(), 1);
        let log = leak_log();
        let mut st = PackState::new(host(log));
        let cfg = serde_json::json!({
            "connections": [{"id":"plc","role":"client","transport":"tcp","host":"127.0.0.1","port":addr.port(),
                             "timeout_ms":200,"unit_id":1,"retries":1}],
            "inputs": [{"connection":"plc","interval_secs":0.05,"binds":[
                {"area":"holding","address":7,"dtype":"u16","point":"psi","scale":0.1,"offset":0.0}]}],
            "outputs": [{"connection":"plc","binds":[
                {"area":"holding","address":8,"dtype":"i16","point":"temp_sp","scale":0.1,"offset":-40.0}]}]
        });
        st.configure_json(&cfg.to_string()).unwrap();
        wait_for(log, "scaled read", |l| l.iter().any(|r| r.name == "psi" && r.value == Value::Float(123.4)));
        // 21.5 °C with offset -40, scale 0.1 → raw 615.
        st.write_point("temp_sp", "21.5");
        let end = Instant::now() + Duration::from_secs(5);
        while bank.lock().unwrap().holding.get(&8).copied() != Some(615) {
            assert!(Instant::now() < end, "scaled write never landed");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn dead_endpoint_goes_bad_then_recovers() {
        // Reserve a port, release it, so the first connect is refused.
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let log = leak_log();
        let mut st = PackState::new(host(log));
        st.configure_json(&cfg(port)).unwrap();
        wait_for(log, "bad quality", |l| l.iter().any(|r| r.name == "modbus_1" && r.quality == PB_QUALITY_BAD));
        assert!(st.last_error().contains("127.0.0.1"));
        assert_eq!(st.state_label(), "0/1 links up");
        // Device appears on that port: pack reconnects by itself.
        let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
        let bank = device();
        let b2 = bank.clone();
        std::thread::spawn(move || {
            for s in listener.incoming().flatten() {
                let b = b2.clone();
                std::thread::spawn(move || server::serve_tcp_stream(s, b, 1));
            }
        });
        wait_for(log, "recovery", |l| l.iter().any(|r| r.name == "modbus_1" && r.quality == PB_QUALITY_GOOD));
    }

    #[test]
    fn exception_marks_points_bad_but_keeps_the_link() {
        let bank = Arc::new(Mutex::new(RegisterBank::default())); // nothing defined
        let addr = server::spawn_tcp_for_tests(bank, 1);
        let log = leak_log();
        let mut st = PackState::new(host(log));
        st.configure_json(&cfg(addr.port())).unwrap();
        wait_for(log, "bad from exception", |l| l.iter().any(|r| r.quality == PB_QUALITY_BAD));
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(st.state_label(), "1/1 links up");
        assert!(st.last_error().contains("illegal data address"));
    }

    #[test]
    fn reconfigure_keeps_unchanged_workers_and_bad_config_is_reported() {
        let bank = device();
        let addr = server::spawn_tcp_for_tests(bank, 1);
        let log = leak_log();
        let mut st = PackState::new(host(log));
        st.configure_json(&cfg(addr.port())).unwrap();
        let before = st.workers[0].join.as_ref().unwrap().thread().id();
        st.configure_json(&cfg(addr.port())).unwrap();
        assert_eq!(st.workers[0].join.as_ref().unwrap().thread().id(), before);
        let bad = cfg(addr.port()).replace("\"dtype\":\"f32\"", "\"dtype\":\"nope\"");
        st.configure_json(&bad).unwrap();
        assert!(st.last_error().contains("unknown type"));
        assert_eq!(st.state_label(), "config error");
        st.stop_all();
    }

    fn free_port() -> u16 {
        TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
    }

    fn server_cfg(port: u16) -> String {
        serde_json::json!({"connections":[{"id":"srv","role":"server","transport":"tcp","host":"127.0.0.1",
            "port":port,"unit_id":1,"map":[
              {"area":"holding","address":0,"dtype":"f32","order":"cdab","point":"tank"},
              {"area":"holding","address":2,"dtype":"u16","point":"count"},
              {"area":"holding","address":3,"dtype":"bit2","point":"alarm"},
              {"area":"holding","address":4,"dtype":"str3","point":"tag"},
              {"area":"coil","address":0,"dtype":"bool","point":"pump"},
              {"area":"input","address":0,"dtype":"i32","order":"badc","point":"level"},
              {"area":"discrete","address":0,"dtype":"bool","point":"door"}]}]})
        .to_string()
    }

    fn master(port: u16) -> Session {
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            let cfg = LinkConfig {
                transport: "tcp".into(), host: "127.0.0.1".into(), port, serial_port: String::new(), baud: 9600,
                data_bits: 8, parity: "none".into(), stop_bits: 1, timeout_ms: 500,
            };
            match Session::open(&cfg) {
                Ok(s) => return s,
                Err(e) => {
                    assert!(Instant::now() < end, "server never listened: {e}");
                    std::thread::sleep(Duration::from_millis(30));
                }
            }
        }
    }

    #[test]
    fn server_serves_host_points_every_type_to_a_master() {
        let port = free_port();
        let log = leak_log();
        let mut st = PackState::new(host(log));
        st.configure_json(&server_cfg(port)).unwrap();
        let mut m = master(port);
        // Host pushes values (as the engine does for every changed point).
        st.write_point("tank", "123.456");
        st.write_point("count", "42");
        st.write_point("alarm", "true");
        st.write_point("tag", "P-7");
        st.write_point("pump", "on");
        st.write_point("level", "-70000");
        st.write_point("door", "1");
        let w = m.read_regs(1, Area::Holding, 0, 2).unwrap();
        assert_eq!(codec::decode(DataType::F32, ByteOrder::Cdab, &w).unwrap(), Value::Float(123.456));
        assert_eq!(m.read_regs(1, Area::Holding, 2, 1).unwrap(), vec![42]);
        assert_eq!(m.read_regs(1, Area::Holding, 3, 1).unwrap(), vec![0b100]);
        let t = m.read_regs(1, Area::Holding, 4, 3).unwrap();
        assert_eq!(codec::decode(DataType::Str(3), ByteOrder::Abcd, &t).unwrap(), Value::Text("P-7".into()));
        assert_eq!(m.read_bits(1, Area::Coil, 0, 1).unwrap(), vec![true]);
        assert_eq!(m.read_bits(1, Area::Discrete, 0, 1).unwrap(), vec![true]);
        let l = m.read_regs(1, Area::Input, 0, 2).unwrap();
        assert_eq!(codec::decode(DataType::I32, ByteOrder::Badc, &l).unwrap(), Value::Int(-70000));
        // Unmapped -> exception; link and map still fine.
        assert_eq!(m.read_regs(1, Area::Holding, 100, 1), Err(ModbusError::Exception(2)));
        assert!(st.state_label().contains("1/1"));
        assert!(st.detail_json().contains("\"role\":\"server\""));
        st.stop_all();
    }

    #[test]
    fn master_writes_become_host_points_without_rebinding_their_source() {
        let port = free_port();
        let log = leak_log();
        let mut st = PackState::new(host(log));
        st.configure_json(&server_cfg(port)).unwrap();
        let mut m = master(port);
        let words = codec::encode(DataType::F32, ByteOrder::Cdab, &Value::Float(7.25)).unwrap();
        m.transact(1, &Request::WriteRegs { start: 0, values: words }).unwrap();
        m.transact(1, &Request::WriteReg { addr: 2, value: 9 }).unwrap();
        m.transact(1, &Request::WriteCoil { addr: 0, on: true }).unwrap();
        // Bit write: only the alarm bit's register changes.
        m.transact(1, &Request::WriteReg { addr: 3, value: 0b100 }).unwrap();
        wait_for(log, "tank / count / pump / alarm", |l| {
            l.iter().any(|r| r.name == "tank" && r.value == Value::Float(7.25))
                && l.iter().any(|r| r.name == "count" && r.value == Value::Int(9))
                && l.iter().any(|r| r.name == "pump" && r.value == Value::Bool(true))
                && l.iter().any(|r| r.name == "alarm" && r.value == Value::Bool(true))
        });
        // Writing an unmapped address is refused and publishes nothing.
        let before = log.lock().unwrap().len();
        assert_eq!(
            m.transact(1, &Request::WriteReg { addr: 500, value: 1 }),
            Err(ModbusError::Exception(2))
        );
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(log.lock().unwrap().len(), before);
        st.stop_all();
    }

    #[test]
    fn server_bind_failure_is_reported_and_retried() {
        let blocker = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = blocker.local_addr().unwrap().port();
        let log = leak_log();
        let mut st = PackState::new(host(log));
        st.configure_json(&server_cfg(port)).unwrap();
        let end = Instant::now() + Duration::from_secs(3);
        while !st.last_error().contains("bind") && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(30));
        }
        assert!(st.last_error().contains("bind"), "{}", st.last_error());
        assert_eq!(st.state_label(), "0/1 links up");
        drop(blocker); // port frees up; the retry loop picks it up within ~2 s
        let mut m = master(port);
        assert!(m.read_regs(1, Area::Holding, 2, 1).is_ok());
        st.stop_all();
    }

    #[test]
    fn overlapping_server_map_and_disabled_servers() {
        let log = leak_log();
        let mut st = PackState::new(host(log));
        let overlap = serde_json::json!({"connections":[{"id":"srv","role":"server","transport":"tcp",
            "host":"127.0.0.1","port":free_port(),"map":[
            {"area":"holding","address":0,"dtype":"f32","point":"a"},
            {"area":"holding","address":1,"dtype":"u16","point":"b"}]}]})
        .to_string();
        st.configure_json(&overlap).unwrap();
        assert!(st.last_error().contains("overlaps"), "{}", st.last_error());
        let off = server_cfg(free_port()).replace("\"role\":\"server\"", "\"role\":\"server\",\"enabled\":false");
        st.configure_json(&off).unwrap();
        assert!(st.workers.is_empty());
        assert_eq!(st.state_label(), "idle");
    }

    #[test]
    fn client_pack_can_poll_the_pack_server_end_to_end() {
        // Server pack serving a mapped point; a second pack instance polls it as a client.
        let port = free_port();
        let slog = leak_log();
        let mut srv = PackState::new(host(slog));
        srv.configure_json(&server_cfg(port)).unwrap();
        srv.write_point("tank", "55.5");
        let clog = leak_log();
        let mut cli = PackState::new(host(clog));
        cli.configure_json(&cfg(port).replace("\"dtype\":\"bit3\"", "\"dtype\":\"bit2\"")).unwrap();
        // The client config also polls unmapped addresses (u32 @2..3 etc.) -> exceptions; f32 @0..1 is served.
        wait_for(clog, "tank via client", |l| {
            l.iter().any(|r| r.name == "modbus_1" && r.quality == PB_QUALITY_GOOD && r.value == Value::Float(55.5))
        });
        cli.stop_all();
        srv.stop_all();
    }

    #[test]
    fn blank_point_values_are_ignored_not_errors() {
        let port = free_port();
        let log = leak_log();
        let mut st = PackState::new(host(log));
        st.configure_json(&server_cfg(port)).unwrap();
        let mut m = master(port);
        st.write_point("pump", "");
        st.write_point("tank", "  ");
        assert!(st.last_error().is_empty(), "{}", st.last_error());
        assert_eq!(m.read_bits(1, Area::Coil, 0, 1).unwrap(), vec![false]);
        st.stop_all();
    }

    #[test]
    fn disabled_client_gets_no_worker() {
        let log = leak_log();
        let mut st = PackState::new(host(log));
        let off = cfg(free_port()).replace("\"role\":\"client\"", "\"role\":\"client\",\"enabled\":false");
        st.configure_json(&off).unwrap();
        assert!(st.workers.is_empty());
        assert_eq!(st.state_label(), "idle");
    }

    #[test]
    fn abi_registers() {
        let mut api = std::mem::MaybeUninit::<PbPackApi>::uninit();
        assert_eq!(unsafe { pb_pack_register(api.as_mut_ptr()) }, 0);
        assert_eq!(unsafe { pb_pack_abi_version() }, PB_ABI_VERSION);
    }
}
