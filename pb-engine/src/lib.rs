//! Engine host for Protocol Bench.
//!
//! The UI thread never opens a socket, serial port, file, or protocol session.
//! [`EngineHandle::spawn`] starts one OS thread named `pb-engine`. That thread
//! owns every [`ProtocolEngine`]. Protocol packs load from a modules directory
//! via the stable C ABI (`pb-abi`) and wrap as engines for their slots.

mod loader;
mod pack_engine;

pub use loader::{default_modules_dir, load_packs, verify_pack_stub, LoadedPack, PackLoadError};
pub use pack_engine::PackEngine;

use pb_points::PointStore;
use serde::{Deserialize, Serialize};

pub mod agg;
pub mod calc;
pub use agg::{
    agg_point_name, peek_one as peek_agg, AggConfig, AggOutput, AggRuntime, MemberReading,
};
pub use calc::{peek_for_point, CalcConfig, CalcRuntime};
use pb_points::PointValue;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Which protocol engine a command or snapshot row belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineId {
    Streams,
    Modbus,
    Mqtt,
    OpcUa,
    FileFollow,
    Http,
    Snmp,
    Las,
    Wits,
    WebSocket,
    Sparkplug,
    Dnp3,
    Can,
    /// Generator: synthetic signals instead of a device.
    Gen,
}

impl EngineId {
    pub const ALL: [EngineId; 14] = [
        EngineId::Streams,
        EngineId::Modbus,
        EngineId::Mqtt,
        EngineId::OpcUa,
        EngineId::FileFollow,
        EngineId::Http,
        EngineId::Snmp,
        EngineId::Las,
        EngineId::Wits,
        EngineId::WebSocket,
        EngineId::Sparkplug,
        EngineId::Dnp3,
        EngineId::Can,
        EngineId::Gen,
    ];

    /// Slot for an ext protocol section key (`wits`, `websocket`, …).
    pub fn from_ext(id: &str) -> Option<EngineId> {
        Self::ALL.into_iter().find(|e| e.pack_id() == Some(id) && e.is_ext())
    }

    /// True for the protocols configured through the generic ext path.
    pub fn is_ext(self) -> bool {
        matches!(self, Self::Wits | Self::WebSocket | Self::Sparkplug | Self::Dnp3 | Self::Can | Self::Gen)
    }

    /// Slots whose packs take host point writes (Output binds). The engine sends egress to
    /// every connected one.
    pub fn takes_egress(self) -> bool {
        !matches!(self, Self::OpcUa | Self::FileFollow | Self::Snmp | Self::Gen)
    }

    /// Human title for the engine (matches the pack `TITLE`).
    pub fn title(self) -> &'static str {
        match self {
            Self::Streams => "Streams",
            Self::Modbus => "Modbus",
            Self::Mqtt => "MQTT",
            Self::OpcUa => "OPC UA",
            Self::FileFollow => "File follow",
            Self::Http => "HTTP / REST",
            Self::Snmp => "SNMP",
            Self::Las => "LAS",
            Self::Wits => "WITS",
            Self::WebSocket => "WebSocket",
            Self::Sparkplug => "Sparkplug B",
            Self::Dnp3 => "DNP3",
            Self::Can => "CAN",
            Self::Gen => "Generator",
        }
    }

    /// Pack id that claims this slot (matches [`pb_abi::PbPackApi::id`]).
    pub fn pack_id(self) -> Option<&'static str> {
        match self {
            Self::Mqtt => Some("mqtt"),
            Self::OpcUa => Some("opcua"),
            Self::Streams => Some("streams"),
            Self::Http => Some("http"),
            Self::Modbus => Some("modbus"),
            Self::FileFollow => Some("files"),
            Self::Snmp => Some("snmp"),
            Self::Las => Some("las"),
            Self::Wits => Some("wits"),
            Self::WebSocket => Some("websocket"),
            Self::Sparkplug => Some("sparkplug"),
            Self::Dnp3 => Some("dnp3"),
            Self::Can => Some("can"),
            Self::Gen => Some("gen"),
        }
    }

    /// What this slot will own once the protocol stack lands. No I/O for idle.
    pub fn note(self) -> &'static str {
        match self {
            Self::Streams => {
                "Streams / TCP pack — serial + TCP client/listener text framing (scan / emit)."
            }
            Self::Modbus => "Modbus pack — TCP / RTU client poll (Input) and typed register writes (Output).",
            Self::Mqtt => {
                "MQTT client pack (source/output) plus optional embedded rumqttd LAN broker."
            }
            Self::OpcUa => {
                "OPC UA client pack plus optional embedded UA server hub (async-opcua)."
            }
            Self::FileFollow => {
                "File follow pack — growing CSV / JSON-lines files to points; byte offset kept across restart and rotation."
            }
            Self::Http => "HTTP / REST pack — outbound client poll (Input) and JSON body push (Output).",
            Self::Snmp => "SNMP pack — v2c GET poll of agent OIDs (Input).",
            Self::Las => "LAS pack — follow LAS 2.0 well-log files (Input) and write curve rows (Output).",
            Self::Wits => "WITS pack — WITS level 0 record streams over TCP / serial (Input + Output).",
            Self::WebSocket => "WebSocket pack — JSON messages from a ws:// client or to a listening server.",
            Self::Sparkplug => "Sparkplug B pack — host (decode NBIRTH / NDATA) and edge node (publish).",
            Self::Dnp3 => "DNP3 pack — TCP master: class polls, binary / analog / counter points, controls.",
            Self::Can => "CAN pack — SocketCAN signals in (frame decode) and out (frame encode).",
            Self::Gen => "Generator pack — synthetic signals (waveforms, random walks, process shapes, counters, formulas) on interval / random / burst triggers.",
        }
    }

    /// Build the idle engine the host thread owns when no pack is loaded.
    pub fn create_idle(self) -> Box<dyn ProtocolEngine> {
        Box::new(IdleEngine::new(self))
    }
}

/// Command from the UI / service. Handled only on `pb-engine`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum EngineCommand {
    /// Cross-thread scaffold signal. Does not open a socket, port, or file.
    Acknowledge,
    /// JSON config for the pack (host / port / credentials / transport).
    Configure { json: String },
    Connect,
    Disconnect,
    Subscribe { topic: String, qos: u8 },
    Publish {
        topic: String,
        #[serde(with = "serde_bytes_vec")]
        payload: Vec<u8>,
        qos: u8,
        retain: bool,
    },
    /// Host point write forwarded into the pack (may publish).
    WritePoint {
        name: String,
        /// UTF-8 string payload for the first cut.
        value: String,
    },
}

mod serde_bytes_vec {
    use serde::de::Error;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        // JSON-friendly base64-ish: array of numbers is fine for small payloads.
        s.collect_seq(bytes.iter().copied())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let vals: Vec<u64> = Vec::deserialize(d)?;
        vals.into_iter()
            .map(|v| u8::try_from(v).map_err(D::Error::custom))
            .collect()
    }
}

/// One protocol implementation. Only the host thread calls these methods.
pub trait ProtocolEngine: Send {
    fn id(&self) -> EngineId;
    /// One idle turn. Must not block on I/O.
    fn poll(&mut self);
    /// Handle a command. Returns `false` when the pack reported failure (e.g. not connected).
    fn handle(&mut self, command: EngineCommand) -> bool;
    fn snapshot(&self) -> SlotSnapshot;
}

/// Idle stand-in until a real protocol pack replaces it.
struct IdleEngine {
    id: EngineId,
    ticks: u64,
    commands_handled: u64,
}

impl IdleEngine {
    fn new(id: EngineId) -> Self {
        Self {
            id,
            ticks: 0,
            commands_handled: 0,
        }
    }
}

impl ProtocolEngine for IdleEngine {
    fn id(&self) -> EngineId {
        self.id
    }

    fn poll(&mut self) {
        self.ticks = self.ticks.saturating_add(1);
    }

    fn handle(&mut self, command: EngineCommand) -> bool {
        match command {
            EngineCommand::Acknowledge
            | EngineCommand::Configure { .. }
            | EngineCommand::Connect
            | EngineCommand::Disconnect
            | EngineCommand::Subscribe { .. }
            | EngineCommand::Publish { .. }
            | EngineCommand::WritePoint { .. } => {
                self.commands_handled = self.commands_handled.saturating_add(1);
            }
        }
        true
    }

    fn snapshot(&self) -> SlotSnapshot {
        SlotSnapshot {
            id: self.id,
            title: self.id.title().into(),
            state: "idle".into(),
            note: self.id.note().into(),
            ticks: self.ticks,
            commands_handled: self.commands_handled,
            connected: false,
            last_error: String::new(),
            topics: Vec::new(),
            detail_json: String::new(),
            pack_loaded: false,
        }
    }
}

/// One engine slot, copied out for the UI thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotSnapshot {
    pub id: EngineId,
    pub title: String,
    pub state: String,
    pub note: String,
    pub ticks: u64,
    pub commands_handled: u64,
    pub connected: bool,
    pub last_error: String,
    pub topics: Vec<String>,
    /// Pack-specific JSON snapshot (MQTT: clients array).
    pub detail_json: String,
    pub pack_loaded: bool,
}

impl SlotSnapshot {
    fn scaffold(id: EngineId) -> Self {
        Self {
            id,
            title: id.title().into(),
            state: "idle".into(),
            note: id.note().into(),
            ticks: 0,
            commands_handled: 0,
            connected: false,
            last_error: String::new(),
            topics: Vec::new(),
            detail_json: String::new(),
            pack_loaded: false,
        }
    }
}

/// One host point, copied for the UI (packs upsert into the live store).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PointSnapshot {
    pub name: String,
    pub value: String,
    pub quality: String,
    pub source: String,
    pub updated_ms: u64,
}

/// What the UI is allowed to know about the host.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostSnapshot {
    pub thread_name: String,
    pub alive: bool,
    pub ticks: u64,
    pub uptime_ms: u64,
    pub slots: Vec<SlotSnapshot>,
    pub modules_dir: String,
    pub packs_loaded: Vec<String>,
    pub point_count: usize,
    pub points: Vec<PointSnapshot>,
}

impl HostSnapshot {
    /// Snapshot of one engine slot, if it is loaded.
    pub fn slot(&self, id: EngineId) -> Option<&SlotSnapshot> {
        self.slots.iter().find(|slot| slot.id == id)
    }

    /// Points for one protocol (short id `mqtt_*` or address `mqtt/…`).
    pub fn points_for_source<'a>(
        &'a self,
        proto: &'a str,
    ) -> impl Iterator<Item = &'a PointSnapshot> + 'a {
        let prefix_id = format!("{proto}_");
        let prefix_addr = format!("{proto}/");
        self.points.iter().filter(move |p| {
            p.name.starts_with(&prefix_id) || p.source.starts_with(&prefix_addr)
        })
    }
}

enum HostCommand {
    Shutdown,
    ForEngine {
        id: EngineId,
        command: EngineCommand,
    },
}

/// Options for spawning the engine host.
#[derive(Debug, Clone, Default)]
pub struct EngineOptions {
    pub modules_dir: Option<PathBuf>,
    pub points: Option<Arc<Mutex<PointStore>>>,
}

/// UI-thread handle. Drop sends shutdown and joins `pb-engine`.
pub struct EngineHandle {
    tx: Sender<HostCommand>,
    snap: Arc<Mutex<HostSnapshot>>,
    points: Arc<Mutex<PointStore>>,
    /// Hub calculation steps — applied on Output egress (deadband hold, etc.).
    calcs: Arc<Mutex<Vec<CalcConfig>>>,
    /// Shared edge-runner state (deadband hold, rolling windows) for egress + UI peek.
    calc_rt: Arc<Mutex<CalcRuntime>>,
    /// Hub aggregator configs → host `agg_N` points.
    aggs: Arc<Mutex<Vec<AggConfig>>>,
    agg_rt: Arc<Mutex<AggRuntime>>,
    join: Option<JoinHandle<()>>,
}

impl EngineHandle {
    /// Start the host thread and its five protocol slots (default modules dir).
    pub fn spawn() -> Self {
        Self::spawn_with(EngineOptions::default())
    }

    /// Start with an explicit modules directory and/or shared point store.
    pub fn spawn_with(opts: EngineOptions) -> Self {
        let modules_dir = opts
            .modules_dir
            .unwrap_or_else(default_modules_dir);
        let points = opts
            .points
            .unwrap_or_else(|| Arc::new(Mutex::new(PointStore::new())));
        let calcs = Arc::new(Mutex::new(Vec::<CalcConfig>::new()));
        let calc_rt = Arc::new(Mutex::new(CalcRuntime::default()));
        let aggs = Arc::new(Mutex::new(Vec::<AggConfig>::new()));
        let agg_rt = Arc::new(Mutex::new(AggRuntime::default()));
        let (tx, rx) = mpsc::channel();
        let snap = Arc::new(Mutex::new(initial_snapshot(&modules_dir)));
        let snap_thread = Arc::clone(&snap);
        let points_thread = Arc::clone(&points);
        let calcs_thread = Arc::clone(&calcs);
        let calc_rt_thread = Arc::clone(&calc_rt);
        let aggs_thread = Arc::clone(&aggs);
        let agg_rt_thread = Arc::clone(&agg_rt);
        let join = thread::Builder::new()
            .name("pb-engine".into())
            .spawn(move || {
                host_main(
                    rx,
                    snap_thread,
                    points_thread,
                    calcs_thread,
                    calc_rt_thread,
                    aggs_thread,
                    agg_rt_thread,
                    modules_dir,
                )
            })
            .expect("spawn pb-engine thread");
        Self {
            tx,
            snap,
            points,
            calcs,
            calc_rt,
            aggs,
            agg_rt,
            join: Some(join),
        }
    }

    /// Shared host point store (packs upsert through ABI into this).
    pub fn points(&self) -> Arc<Mutex<PointStore>> {
        Arc::clone(&self.points)
    }

    /// Shared calculation runtime (UI peeks hold / rolling state here).
    pub fn calc_runtime(&self) -> Arc<Mutex<CalcRuntime>> {
        Arc::clone(&self.calc_rt)
    }

    /// Replace hub calculation configs used on Output egress.
    pub fn set_calcs(&self, next: Vec<CalcConfig>) {
        if let Ok(mut guard) = self.calcs.lock() {
            *guard = next;
        }
    }

    /// Shared aggregator runtime (UI peeks window history here).
    pub fn agg_runtime(&self) -> Arc<Mutex<AggRuntime>> {
        Arc::clone(&self.agg_rt)
    }

    /// Replace hub aggregator configs (N≥2 members → `agg_N` points).
    pub fn set_aggs(&self, next: Vec<AggConfig>) {
        if let Ok(mut guard) = self.aggs.lock() {
            *guard = next;
        }
    }

    /// Latest copy. Never borrows the engine thread's state.
    pub fn snapshot(&self) -> HostSnapshot {
        self.snap
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    /// Queue a command for one slot. The host thread is the only handler.
    pub fn submit(&self, id: EngineId, command: EngineCommand) {
        let _ = self.tx.send(HostCommand::ForEngine { id, command });
    }

    /// Ask the host to exit and wait for it. Safe to call more than once.
    pub fn shutdown(&mut self) {
        let _ = self.tx.send(HostCommand::Shutdown);
        if let Some(handle) = self.join.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for EngineHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn initial_snapshot(modules_dir: &Path) -> HostSnapshot {
    HostSnapshot {
        thread_name: "pb-engine".into(),
        alive: true,
        ticks: 0,
        uptime_ms: 0,
        slots: EngineId::ALL
            .into_iter()
            .map(SlotSnapshot::scaffold)
            .collect(),
        modules_dir: modules_dir.display().to_string(),
        packs_loaded: Vec::new(),
        point_count: 0,
        points: Vec::new(),
    }
}

fn host_main(
    rx: Receiver<HostCommand>,
    snap: Arc<Mutex<HostSnapshot>>,
    points: Arc<Mutex<PointStore>>,
    calcs: Arc<Mutex<Vec<CalcConfig>>>,
    calc_rt: Arc<Mutex<CalcRuntime>>,
    aggs: Arc<Mutex<Vec<AggConfig>>>,
    agg_rt: Arc<Mutex<AggRuntime>>,
    modules_dir: PathBuf,
) {
    let started = Instant::now();
    let loaded = load_packs(&modules_dir, Arc::clone(&points));
    let pack_ids: Vec<String> = loaded.iter().map(|p| p.id.clone()).collect();
    let mut engines = build_engines(loaded);
    let thread_name = thread::current().name().unwrap_or("pb-engine").to_string();
    // Last processed `updated_ms` per point — advances calc runtime once per change.
    let mut calc_stamp: std::collections::BTreeMap<String, u64> =
        std::collections::BTreeMap::new();
    // Last calc output per point (preview + publish source of truth).
    let mut last_out: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();
    // Last payload string published per point (deadband must not republish holds).
    let mut egress_payload: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();
    // Egress slots connected on the previous tick: a slot that just connected gets every
    // current value once (otherwise it would only see points that change later).
    let mut prev_egress: Vec<EngineId> = Vec::new();

    loop {
        let queued = match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(HostCommand::Shutdown) | Err(RecvTimeoutError::Disconnected) => {
                publish(
                    &snap,
                    snapshot_of(
                        &engines,
                        &thread_name,
                        started,
                        false,
                        &modules_dir,
                        &pack_ids,
                        &points,
                    ),
                );
                break;
            }
            Ok(HostCommand::ForEngine { id, command }) => Some((id, command)),
            Err(RecvTimeoutError::Timeout) => None,
        };
        if let Some((id, command)) = queued {
            if let Some(engine) = engines.iter_mut().find(|engine| engine.id() == id) {
                let _ = engine.handle(command);
            }
        }
        for engine in engines.iter_mut() {
            engine.poll();
        }

        // Always advance edge-runner state on point stamp changes (even if MQTT is down)
        // so UI live preview shares real deadband / rolling windows with egress.
        let rows = points
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .snapshot();
        let calc_cfgs = calcs.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let mut dirty_out: Vec<(String, String)> = Vec::new();
        {
            let mut rt = calc_rt.lock().unwrap_or_else(|e| e.into_inner());
            for p in &rows {
                if calc_stamp.get(&p.name).copied() == Some(p.updated_ms) {
                    continue;
                }
                let raw = p.value.as_display();
                // Sample time = point stamp so the rolling window follows the signal clock.
                let value =
                    calc::apply_at(&calc_cfgs, &mut rt, &p.name, &raw, p.updated_ms);
                calc_stamp.insert(p.name.clone(), p.updated_ms);
                last_out.insert(p.name.clone(), value.clone());
                dirty_out.push((p.name.clone(), value));
            }
        }

        // Aggregators: recompute every tick from member values → host `agg_N` points.
        {
            let agg_cfgs = aggs.lock().unwrap_or_else(|e| e.into_inner()).clone();
            if !agg_cfgs.is_empty() {
                let mut art = agg_rt.lock().unwrap_or_else(|e| e.into_inner());
                let lookup = |name: &str| {
                    rows.iter().find(|p| p.name == name).map(|p| agg::MemberReading {
                        name: p.name.clone(),
                        value: p.value.clone(),
                        quality: p.quality,
                        updated_ms: p.updated_ms,
                    })
                };
                let outs = agg::evaluate_all(&agg_cfgs, &mut art, &lookup);
                drop(art);
                if !outs.is_empty() {
                    let mut store = points.lock().unwrap_or_else(|e| e.into_inner());
                    for o in outs {
                        let src = format!("agg/step/{}", o.name.trim_start_matches("agg_"));
                        let same = store.get(&o.name).is_some_and(|p| {
                            p.value.as_display() == o.value && p.quality == o.quality
                        });
                        if same {
                            continue;
                        }
                        let pv = o
                            .value
                            .parse::<f64>()
                            .map(PointValue::F64)
                            .unwrap_or_else(|_| PointValue::String(o.value.clone()));
                        store.upsert(o.name.clone(), pv, o.quality, src);
                        // Feed egress when the derived point changes.
                        last_out.insert(o.name.clone(), o.value.clone());
                        dirty_out.push((o.name, o.value));
                    }
                }
            }
        }

        // Live Output publish: push transformed values to every connected egress pack.
        let egress: Vec<EngineId> = engines
            .iter()
            .filter(|e| e.id().takes_egress() && e.snapshot().connected)
            .map(|e| e.id())
            .collect();
        let fresh: Vec<EngineId> = egress.iter().copied().filter(|id| !prev_egress.contains(id)).collect();
        if !fresh.is_empty() {
            // Newly connected packs: send every current value (no deadband check).
            for (name, value) in &last_out {
                for &id in &fresh {
                    if let Some(engine) = engines.iter_mut().find(|e| e.id() == id) {
                        engine.handle(EngineCommand::WritePoint { name: name.clone(), value: value.clone() });
                    }
                }
                egress_payload.insert(name.clone(), value.clone());
            }
        }
        let steady: Vec<EngineId> = egress.iter().copied().filter(|id| prev_egress.contains(id)).collect();
        if !steady.is_empty() {
            for (name, value) in dirty_out {
                // Deadband hold: same payload as last publish → skip (no Explorer jitter).
                if egress_payload.get(&name) == Some(&value) {
                    continue;
                }
                let mut ok = false;
                for &id in &steady {
                    if let Some(engine) = engines.iter_mut().find(|e| e.id() == id) {
                        ok |= engine.handle(EngineCommand::WritePoint {
                            name: name.clone(),
                            value: value.clone(),
                        });
                    }
                }
                if ok {
                    egress_payload.insert(name, value);
                }
            }
        }
        if egress.is_empty() {
            // Keep calc_rt / last_out so preview stays coherent across disconnects.
            egress_payload.clear();
        }
        prev_egress = egress;
        publish(
            &snap,
            snapshot_of(
                &engines,
                &thread_name,
                started,
                true,
                &modules_dir,
                &pack_ids,
                &points,
            ),
        );
    }
}

fn build_engines(mut loaded: Vec<LoadedPack>) -> Vec<Box<dyn ProtocolEngine>> {
    let mut engines: Vec<Box<dyn ProtocolEngine>> = Vec::with_capacity(EngineId::ALL.len());
    for id in EngineId::ALL {
        if let Some(pack_id) = id.pack_id() {
            if let Some(idx) = loaded.iter().position(|p| p.id == pack_id) {
                let pack = loaded.remove(idx);
                match PackEngine::from_loaded(id, pack) {
                    Ok(engine) => {
                        engines.push(Box::new(engine));
                        continue;
                    }
                    Err(err) => {
                        eprintln!("pb-engine: failed to wrap pack {pack_id}: {err}");
                    }
                }
            }
        }
        engines.push(id.create_idle());
    }
    // Drop unused packs (keep library alive until end of LoadedPack drop).
    drop(loaded);
    engines
}

fn snapshot_of(
    engines: &[Box<dyn ProtocolEngine>],
    thread_name: &str,
    started: Instant,
    alive: bool,
    modules_dir: &Path,
    pack_ids: &[String],
    points: &Mutex<PointStore>,
) -> HostSnapshot {
    let slots: Vec<SlotSnapshot> = engines.iter().map(|engine| engine.snapshot()).collect();
    let ticks = slots.iter().map(|slot| slot.ticks).max().unwrap_or(0);
    let point_rows = points
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .snapshot()
        .into_iter()
        .map(|p| PointSnapshot {
            name: p.name,
            value: p.value.as_display(),
            quality: p.quality.label().into(),
            source: p.source,
            updated_ms: p.updated_ms,
        })
        .collect::<Vec<_>>();
    let point_count = point_rows.len();
    HostSnapshot {
        thread_name: thread_name.to_string(),
        alive,
        ticks,
        uptime_ms: started.elapsed().as_millis() as u64,
        slots,
        modules_dir: modules_dir.display().to_string(),
        packs_loaded: pack_ids.to_vec(),
        point_count,
        points: point_rows,
    }
}

fn publish(snap: &Mutex<HostSnapshot>, next: HostSnapshot) {
    *snap.lock().unwrap_or_else(|err| err.into_inner()) = next;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_polls_and_accepts_a_command_without_io() {
        let dir = std::env::temp_dir().join("pb-engine-empty-modules");
        let _ = std::fs::create_dir_all(&dir);
        let host = EngineHandle::spawn_with(EngineOptions {
            modules_dir: Some(dir),
            points: None,
        });
        let start = Instant::now();
        let mut snap = host.snapshot();
        while snap.ticks == 0 && start.elapsed() < Duration::from_secs(2) {
            thread::sleep(Duration::from_millis(40));
            snap = host.snapshot();
        }
        assert!(snap.alive, "engine thread not alive");
        assert_eq!(snap.thread_name, "pb-engine");
        assert!(snap.ticks >= 1, "engine thread did not poll");
        assert_eq!(snap.slots.len(), EngineId::ALL.len());
        for id in EngineId::ALL {
            let slot = snap.slot(id).expect("slot");
            assert_eq!(slot.state, "idle");
            assert_eq!(slot.commands_handled, 0);
            assert!(!slot.pack_loaded);
        }

        host.submit(EngineId::Modbus, EngineCommand::Acknowledge);
        let start = Instant::now();
        let mut handled = 0;
        while handled == 0 && start.elapsed() < Duration::from_secs(2) {
            thread::sleep(Duration::from_millis(40));
            handled = host
                .snapshot()
                .slot(EngineId::Modbus)
                .map(|slot| slot.commands_handled)
                .unwrap_or(0);
        }
        assert!(handled >= 1, "command did not cross onto pb-engine");
    }
}
