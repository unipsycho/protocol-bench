//! Engine host for Protocol Bench.
//!
//! The UI thread never opens a socket, serial port, file, or protocol session.
//! [`EngineHandle::spawn`] starts one OS thread named `pb-engine`. That thread
//! owns every [`ProtocolEngine`]. Each protocol has a slot there — streams,
//! Modbus, MQTT, OPC UA, and file follow. Slots are idle scaffolds: they poll
//! and accept commands, and they do not perform I/O.
//!
//! A later stack replaces [`EngineId::create`] for its protocol. The UI keeps
//! talking only through [`EngineHandle::snapshot`] and [`EngineHandle::submit`].

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

    /// What this slot will own once the protocol stack lands. No I/O today.
    pub fn note(self) -> &'static str {
        match self {
            Self::Streams => "Owns serial ports and TCP sockets. No port is open in this build.",
            Self::Modbus => "Owns Modbus RTU and TCP sessions. No session is open in this build.",
            Self::Mqtt => "Owns the MQTT client. No broker is contacted in this build.",
            Self::OpcUa => {
                "Owns OPC UA client and server sessions. No session is open in this build."
            }
            Self::FileFollow => {
                "Owns file tails and CSV / JSON-lines parsers. No file is open in this build."
            }
        }
    }

    /// Build the engine the host thread owns.
    /// Replace the idle body here when that protocol stack is ready.
    pub fn create(self) -> Box<dyn ProtocolEngine> {
        Box::new(IdleEngine::new(self))
    }
}

/// Command from the UI thread. Handled only on `pb-engine`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineCommand {
    /// Cross-thread scaffold signal. Does not open a socket, port, or file.
    Acknowledge,
}

/// One protocol implementation. Only the host thread calls these methods.
pub trait ProtocolEngine: Send {
    fn id(&self) -> EngineId;
    /// One idle turn. Must not block on I/O.
    fn poll(&mut self);
    fn handle(&mut self, command: EngineCommand);
    fn snapshot(&self) -> SlotSnapshot;
}

/// Idle stand-in until a real protocol engine replaces it.
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

    fn handle(&mut self, command: EngineCommand) {
        match command {
            EngineCommand::Acknowledge => {
                self.commands_handled = self.commands_handled.saturating_add(1);
            }
        }
    }

    fn snapshot(&self) -> SlotSnapshot {
        SlotSnapshot {
            id: self.id,
            title: self.id.title(),
            state: "idle",
            note: self.id.note(),
            ticks: self.ticks,
            commands_handled: self.commands_handled,
        }
    }
}

/// One engine slot, copied out for the UI thread.
#[derive(Debug, Clone)]
pub struct SlotSnapshot {
    pub id: EngineId,
    pub title: &'static str,
    pub state: &'static str,
    pub note: &'static str,
    pub ticks: u64,
    pub commands_handled: u64,
}

impl SlotSnapshot {
    fn scaffold(id: EngineId) -> Self {
        Self {
            id,
            title: id.title(),
            state: "idle",
            note: id.note(),
            ticks: 0,
            commands_handled: 0,
        }
    }
}

/// What the UI is allowed to know about the host.
#[derive(Debug, Clone)]
pub struct HostSnapshot {
    pub thread_name: String,
    pub alive: bool,
    pub ticks: u64,
    pub uptime_ms: u64,
    pub slots: Vec<SlotSnapshot>,
}

impl HostSnapshot {
    pub fn slot(&self, id: EngineId) -> Option<&SlotSnapshot> {
        self.slots.iter().find(|slot| slot.id == id)
    }
}

enum HostCommand {
    Shutdown,
    ForEngine {
        id: EngineId,
        command: EngineCommand,
    },
}

/// UI-thread handle. Drop sends shutdown and joins `pb-engine`.
pub struct EngineHandle {
    tx: Sender<HostCommand>,
    snap: Arc<Mutex<HostSnapshot>>,
    join: Option<JoinHandle<()>>,
}

impl EngineHandle {
    /// Start the host thread and its five protocol slots.
    pub fn spawn() -> Self {
        let (tx, rx) = mpsc::channel();
        let snap = Arc::new(Mutex::new(initial_snapshot()));
        let snap_thread = Arc::clone(&snap);
        let join = thread::Builder::new()
            .name("pb-engine".into())
            .spawn(move || host_main(rx, snap_thread))
            .expect("spawn pb-engine thread");
        Self {
            tx,
            snap,
            join: Some(join),
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

fn initial_snapshot() -> HostSnapshot {
    HostSnapshot {
        thread_name: "pb-engine".into(),
        alive: true,
        ticks: 0,
        uptime_ms: 0,
        slots: EngineId::ALL
            .into_iter()
            .map(SlotSnapshot::scaffold)
            .collect(),
    }
}

fn host_main(rx: Receiver<HostCommand>, snap: Arc<Mutex<HostSnapshot>>) {
    let started = Instant::now();
    let mut engines: Vec<Box<dyn ProtocolEngine>> =
        EngineId::ALL.into_iter().map(EngineId::create).collect();
    let thread_name = thread::current().name().unwrap_or("pb-engine").to_string();

    loop {
        let queued = match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(HostCommand::Shutdown) | Err(RecvTimeoutError::Disconnected) => {
                publish(&snap, snapshot_of(&engines, &thread_name, started, false));
                break;
            }
            Ok(HostCommand::ForEngine { id, command }) => Some((id, command)),
            Err(RecvTimeoutError::Timeout) => None,
        };
        if let Some((id, command)) = queued {
            if let Some(engine) = engines.iter_mut().find(|engine| engine.id() == id) {
                engine.handle(command);
            }
        }
        for engine in engines.iter_mut() {
            engine.poll();
        }
        publish(&snap, snapshot_of(&engines, &thread_name, started, true));
    }
}

fn snapshot_of(
    engines: &[Box<dyn ProtocolEngine>],
    thread_name: &str,
    started: Instant,
    alive: bool,
) -> HostSnapshot {
    let slots: Vec<SlotSnapshot> = engines.iter().map(|engine| engine.snapshot()).collect();
    let ticks = slots.iter().map(|slot| slot.ticks).max().unwrap_or(0);
    HostSnapshot {
        thread_name: thread_name.to_string(),
        alive,
        ticks,
        uptime_ms: started.elapsed().as_millis() as u64,
        slots,
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
        let host = EngineHandle::spawn();
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
