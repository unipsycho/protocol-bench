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

pub mod calc;
pub use calc::{peek_for_point, CalcConfig, CalcRuntime};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Which protocol engine a command or snapshot row belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineId {
    Streams,
    Modbus,
    Mqtt,
    OpcUa,
    FileFollow,
}

impl EngineId {
    pub const ALL: [EngineId; 5] = [
        EngineId::Streams,
        EngineId::Modbus,
        EngineId::Mqtt,
        EngineId::OpcUa,
        EngineId::FileFollow,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Streams => "Streams",
            Self::Modbus => "Modbus",
            Self::Mqtt => "MQTT",
            Self::OpcUa => "OPC UA",
            Self::FileFollow => "File follow",
        }
    }

    /// Pack id that claims this slot (matches [`pb_abi::PbPackApi::id`]).
    pub fn pack_id(self) -> Option<&'static str> {
        match self {
            Self::Mqtt => Some("mqtt"),
            Self::Streams | Self::Modbus | Self::OpcUa | Self::FileFollow => None,
        }
    }

    /// What this slot will own once the protocol stack lands. No I/O for idle.
    pub fn note(self) -> &'static str {
        match self {
            Self::Streams => "Owns serial ports and TCP sockets. No port is open in this build.",
            Self::Modbus => "Owns Modbus RTU and TCP sessions. No session is open in this build.",
            Self::Mqtt => {
                "MQTT client pack (source/output) plus optional embedded rumqttd LAN broker."
            }
            Self::OpcUa => {
                "Owns OPC UA client and server sessions. No session is open in this build."
            }
            Self::FileFollow => {
                "Owns file tails and CSV / JSON-lines parsers. No file is open in this build."
            }
        }
    }

    /// Build the idle engine the host thread owns when no pack is loaded.
    pub fn create_idle(self) -> Box<dyn ProtocolEngine> {
        Box::new(IdleEngine::new(self))
    }
}

/// Command from the UI / service. Handled only on `pb-engine`.
#[derive(Debug, Clone, PartialEq, Eq)]
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
            title: self.id.title(),
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
#[derive(Debug, Clone)]
pub struct SlotSnapshot {
    pub id: EngineId,
    pub title: &'static str,
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
            title: id.title(),
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
#[derive(Debug, Clone)]
pub struct PointSnapshot {
    pub name: String,
    pub value: String,
    pub quality: String,
    pub source: String,
    pub updated_ms: u64,
}

/// What the UI is allowed to know about the host.
#[derive(Debug, Clone)]
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
        let (tx, rx) = mpsc::channel();
        let snap = Arc::new(Mutex::new(initial_snapshot(&modules_dir)));
        let snap_thread = Arc::clone(&snap);
        let points_thread = Arc::clone(&points);
        let calcs_thread = Arc::clone(&calcs);
        let calc_rt_thread = Arc::clone(&calc_rt);
        let join = thread::Builder::new()
            .name("pb-engine".into())
            .spawn(move || {
                host_main(
                    rx,
                    snap_thread,
                    points_thread,
                    calcs_thread,
                    calc_rt_thread,
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
    let mut mqtt_was_connected = false;

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
                let value = calc::apply_for_egress(&calc_cfgs, &mut rt, &p.name, &raw);
                calc_stamp.insert(p.name.clone(), p.updated_ms);
                last_out.insert(p.name.clone(), value.clone());
                dirty_out.push((p.name.clone(), value));
            }
        }

        // Live Output publish: when MQTT pack is connected, push transformed values.
        let mqtt_connected = engines
            .iter()
            .find(|e| e.id() == EngineId::Mqtt)
            .map(|e| e.snapshot().connected)
            .unwrap_or(false);
        if mqtt_connected {
            let force_all = !mqtt_was_connected;
            mqtt_was_connected = true;
            if let Some(engine) = engines.iter_mut().find(|e| e.id() == EngineId::Mqtt) {
                let to_pub: Vec<(String, String)> = if force_all {
                    last_out
                        .iter()
                        .map(|(n, v)| (n.clone(), v.clone()))
                        .collect()
                } else {
                    dirty_out
                };
                for (name, value) in to_pub {
                    // Deadband hold: same payload as last publish → skip (no Explorer jitter).
                    if !force_all && egress_payload.get(&name) == Some(&value) {
                        continue;
                    }
                    let ok = engine.handle(EngineCommand::WritePoint {
                        name: name.clone(),
                        value: value.clone(),
                    });
                    if ok {
                        egress_payload.insert(name, value);
                    }
                }
            }
        } else {
            mqtt_was_connected = false;
            // Keep calc_rt / last_out so preview stays coherent across disconnects.
            egress_payload.clear();
        }
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
