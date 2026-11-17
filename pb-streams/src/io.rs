//! Endpoint workers: serial, TCP client, TCP listener.

use crate::frame::{FrameConfig, Framer};
use serialport::{DataBits, FlowControl, Parity, StopBits};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct EndpointConfig {
    pub id: String,
    pub kind: String,
    pub serial_port: String,
    pub baud: u32,
    pub data_bits: u8,
    pub parity: String,
    pub stop_bits: u8,
    pub flow: String,
    pub host: String,
    pub tcp_port: u16,
    pub reconnect: bool,
    pub bind: String,
    pub listen_port: u16,
    pub frame: FrameConfig,
    pub auto_connect: bool,
}

pub enum WorkerEvent {
    Connected,
    Disconnected { detail: String },
    Frame { text: String },
    Error { detail: String },
}

enum WorkerCmd {
    Write(Vec<u8>),
    Stop,
}

pub struct EndpointWorker {
    tx_cmd: Sender<WorkerCmd>,
    rx_ev: Receiver<WorkerEvent>,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl EndpointWorker {
    pub fn spawn(cfg: EndpointConfig) -> Result<Self, String> {
        let (tx_cmd, rx_cmd) = mpsc::channel::<WorkerCmd>();
        let (tx_ev, rx_ev) = mpsc::channel::<WorkerEvent>();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_t = Arc::clone(&stop);
        let kind = cfg.kind.clone();
        let join = thread::Builder::new()
            .name(format!("pb-stream-{}", cfg.id))
            .spawn(move || match kind.as_str() {
                "tcp_client" => run_tcp_client(cfg, rx_cmd, tx_ev, stop_t),
                "tcp_listener" => run_tcp_listener(cfg, rx_cmd, tx_ev, stop_t),
                _ => run_serial(cfg, rx_cmd, tx_ev, stop_t),
            })
            .map_err(|e| format!("spawn worker: {e}"))?;
        Ok(Self {
            tx_cmd,
            rx_ev,
            stop,
            join: Some(join),
        })
    }

    pub fn write_bytes(&self, bytes: &[u8]) -> Result<(), String> {
        self.tx_cmd
            .send(WorkerCmd::Write(bytes.to_vec()))
            .map_err(|e| format!("write cmd: {e}"))
    }

    pub fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.tx_cmd.send(WorkerCmd::Stop);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }

    pub fn poll_events(&self) -> Vec<WorkerEvent> {
        let mut out = Vec::new();
        loop {
            match self.rx_ev.try_recv() {
                Ok(ev) => out.push(ev),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    out.push(WorkerEvent::Disconnected {
                        detail: "worker channel closed".into(),
                    });
                    break;
                }
            }
        }
        out
    }
}

fn run_serial(
    cfg: EndpointConfig,
    rx_cmd: Receiver<WorkerCmd>,
    tx_ev: Sender<WorkerEvent>,
    stop: Arc<AtomicBool>,
) {
    let timeout = cfg
        .frame
        .idle_timeout()
        .unwrap_or(Duration::from_millis(50));
    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        match open_serial(&cfg, timeout) {
            Ok(mut port) => {
                let _ = tx_ev.send(WorkerEvent::Connected);
                let mut framer = Framer::new(cfg.frame.clone());
                loop {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    // Commands
                    match rx_cmd.try_recv() {
                        Ok(WorkerCmd::Stop) => {
                            stop.store(true, Ordering::SeqCst);
                            break;
                        }
                        Ok(WorkerCmd::Write(bytes)) => {
                            if let Err(e) = port.write_all(&bytes) {
                                let _ = tx_ev.send(WorkerEvent::Error {
                                    detail: format!("serial write: {e}"),
                                });
                            } else {
                                let _ = port.flush();
                            }
                        }
                        Err(TryRecvError::Disconnected) => {
                            stop.store(true, Ordering::SeqCst);
                            break;
                        }
                        Err(TryRecvError::Empty) => {}
                    }
                    let mut buf = [0u8; 1024];
                    match port.read(&mut buf) {
                        Ok(0) => {}
                        Ok(n) => {
                            for frame in framer.push(&buf[..n]) {
                                let _ = tx_ev.send(WorkerEvent::Frame { text: frame });
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
                            if let Some(frame) = framer.tick_idle() {
                                let _ = tx_ev.send(WorkerEvent::Frame { text: frame });
                            }
                        }
                        Err(e) => {
                            let _ = tx_ev.send(WorkerEvent::Disconnected {
                                detail: format!("serial read: {e}"),
                            });
                            break;
                        }
                    }
                }
            }
            Err(e) => {
                let _ = tx_ev.send(WorkerEvent::Error {
                    detail: format!("open {}: {e}", cfg.serial_port),
                });
                if !cfg.reconnect {
                    break;
                }
                thread::sleep(Duration::from_secs(2));
            }
        }
        if stop.load(Ordering::SeqCst) || !cfg.reconnect {
            break;
        }
        thread::sleep(Duration::from_millis(500));
    }
    let _ = tx_ev.send(WorkerEvent::Disconnected {
        detail: "stopped".into(),
    });
}

fn open_serial(
    cfg: &EndpointConfig,
    timeout: Duration,
) -> Result<Box<dyn serialport::SerialPort>, String> {
    let data_bits = match cfg.data_bits {
        5 => DataBits::Five,
        6 => DataBits::Six,
        7 => DataBits::Seven,
        _ => DataBits::Eight,
    };
    let parity = match cfg.parity.to_ascii_lowercase().as_str() {
        "even" => Parity::Even,
        "odd" => Parity::Odd,
        _ => Parity::None,
    };
    let stop_bits = if cfg.stop_bits >= 2 {
        StopBits::Two
    } else {
        StopBits::One
    };
    let flow = match cfg.flow.to_ascii_lowercase().as_str() {
        "hardware" => FlowControl::Hardware,
        "software" => FlowControl::Software,
        _ => FlowControl::None,
    };
    serialport::new(&cfg.serial_port, cfg.baud)
        .data_bits(data_bits)
        .parity(parity)
        .stop_bits(stop_bits)
        .flow_control(flow)
        .timeout(timeout)
        .open()
        .map_err(|e| e.to_string())
}

fn run_tcp_client(
    cfg: EndpointConfig,
    rx_cmd: Receiver<WorkerCmd>,
    tx_ev: Sender<WorkerEvent>,
    stop: Arc<AtomicBool>,
) {
    let addr = format!("{}:{}", cfg.host, cfg.tcp_port);
    let timeout = cfg
        .frame
        .idle_timeout()
        .unwrap_or(Duration::from_millis(50));
    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        match TcpStream::connect(&addr) {
            Ok(stream) => {
                let _ = stream.set_read_timeout(Some(timeout));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
                let _ = tx_ev.send(WorkerEvent::Connected);
                let mut stream = stream;
                let mut framer = Framer::new(cfg.frame.clone());
                loop {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    match rx_cmd.try_recv() {
                        Ok(WorkerCmd::Stop) => {
                            stop.store(true, Ordering::SeqCst);
                            break;
                        }
                        Ok(WorkerCmd::Write(bytes)) => {
                            if let Err(e) = stream.write_all(&bytes) {
                                let _ = tx_ev.send(WorkerEvent::Error {
                                    detail: format!("tcp write: {e}"),
                                });
                            } else {
                                let _ = stream.flush();
                            }
                        }
                        Err(TryRecvError::Disconnected) => {
                            stop.store(true, Ordering::SeqCst);
                            break;
                        }
                        Err(TryRecvError::Empty) => {}
                    }
                    let mut buf = [0u8; 2048];
                    match stream.read(&mut buf) {
                        Ok(0) => {
                            let _ = tx_ev.send(WorkerEvent::Disconnected {
                                detail: "peer closed".into(),
                            });
                            break;
                        }
                        Ok(n) => {
                            for frame in framer.push(&buf[..n]) {
                                let _ = tx_ev.send(WorkerEvent::Frame { text: frame });
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
                            if let Some(frame) = framer.tick_idle() {
                                let _ = tx_ev.send(WorkerEvent::Frame { text: frame });
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                        Err(e) => {
                            let _ = tx_ev.send(WorkerEvent::Disconnected {
                                detail: format!("tcp read: {e}"),
                            });
                            break;
                        }
                    }
                }
            }
            Err(e) => {
                let _ = tx_ev.send(WorkerEvent::Error {
                    detail: format!("connect {addr}: {e}"),
                });
                if !cfg.reconnect {
                    break;
                }
                thread::sleep(Duration::from_secs(2));
            }
        }
        if stop.load(Ordering::SeqCst) || !cfg.reconnect {
            break;
        }
        thread::sleep(Duration::from_millis(500));
    }
    let _ = tx_ev.send(WorkerEvent::Disconnected {
        detail: "stopped".into(),
    });
}

fn run_tcp_listener(
    cfg: EndpointConfig,
    rx_cmd: Receiver<WorkerCmd>,
    tx_ev: Sender<WorkerEvent>,
    stop: Arc<AtomicBool>,
) {
    let addr = format!("{}:{}", cfg.bind, cfg.listen_port);
    let listener = match TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(e) => {
            let _ = tx_ev.send(WorkerEvent::Error {
                detail: format!("bind {addr}: {e}"),
            });
            return;
        }
    };
    let _ = listener.set_nonblocking(true);
    let _ = tx_ev.send(WorkerEvent::Connected);
    let peers: Arc<Mutex<Vec<TcpStream>>> = Arc::new(Mutex::new(Vec::new()));
    let peers_r = Arc::clone(&peers);
    let stop_r = Arc::clone(&stop);
    let tx_ev_r = tx_ev.clone();
    let frame_cfg = cfg.frame.clone();
    let reader = thread::spawn(move || {
        let timeout = frame_cfg
            .idle_timeout()
            .unwrap_or(Duration::from_millis(50));
        let mut framers: Vec<Framer> = Vec::new();
        while !stop_r.load(Ordering::SeqCst) {
            // Accept
            match listener.accept() {
                Ok((stream, _)) => {
                    let _ = stream.set_nonblocking(false);
                    let _ = stream.set_read_timeout(Some(timeout));
                    if let Ok(mut g) = peers_r.lock() {
                        g.push(stream.try_clone().unwrap_or(stream));
                        framers.push(Framer::new(frame_cfg.clone()));
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => {}
            }
            // Read each peer
            let mut dead = Vec::new();
            if let Ok(mut g) = peers_r.lock() {
                while framers.len() < g.len() {
                    framers.push(Framer::new(frame_cfg.clone()));
                }
                for (i, stream) in g.iter_mut().enumerate() {
                    let mut buf = [0u8; 2048];
                    match stream.read(&mut buf) {
                        Ok(0) => dead.push(i),
                        Ok(n) => {
                            for frame in framers[i].push(&buf[..n]) {
                                let _ = tx_ev_r.send(WorkerEvent::Frame { text: frame });
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
                            if let Some(frame) = framers[i].tick_idle() {
                                let _ = tx_ev_r.send(WorkerEvent::Frame { text: frame });
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                        Err(_) => dead.push(i),
                    }
                }
                for i in dead.into_iter().rev() {
                    g.remove(i);
                    if i < framers.len() {
                        framers.remove(i);
                    }
                }
            }
            thread::sleep(Duration::from_millis(10));
        }
    });

    while !stop.load(Ordering::SeqCst) {
        match rx_cmd.recv_timeout(Duration::from_millis(100)) {
            Ok(WorkerCmd::Stop) => break,
            Ok(WorkerCmd::Write(bytes)) => {
                if let Ok(mut g) = peers.lock() {
                    g.retain_mut(|s| s.write_all(&bytes).and_then(|_| s.flush()).is_ok());
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    stop.store(true, Ordering::SeqCst);
    let _ = reader.join();
    let _ = tx_ev.send(WorkerEvent::Disconnected {
        detail: "stopped".into(),
    });
}
