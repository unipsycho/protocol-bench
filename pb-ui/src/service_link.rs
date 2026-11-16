//! Backend service reachability for the status chip (PB-98).
//!
//! Connected only when `ipc.json` points at a live localhost listener.
//! Stale endpoint files after a stopped `pb-service` count as offline.
//! Does not drive the in-process engine / hub I/O.

use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use serde::Deserialize;

const PROBE_EVERY: Duration = Duration::from_secs(2);
const CONNECT_TIMEOUT: Duration = Duration::from_millis(200);

#[derive(Debug, Clone, Deserialize)]
struct IpcEndpoint {
    host: String,
    port: u16,
}

#[derive(Debug, Clone)]
pub struct ServiceLink {
    connected: bool,
    detail: String,
    last_probe: Instant,
}

impl Default for ServiceLink {
    fn default() -> Self {
        // Probe immediately on first poll (last_probe in the past).
        Self {
            connected: false,
            detail: "No backend service".into(),
            last_probe: Instant::now()
                .checked_sub(PROBE_EVERY)
                .unwrap_or_else(Instant::now),
        }
    }
}

impl ServiceLink {
    pub fn connected(&self) -> bool {
        self.connected
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }

    /// Refresh when due (or when `force`).
    pub fn poll(&mut self, force: bool) {
        if !force && self.last_probe.elapsed() < PROBE_EVERY {
            return;
        }
        self.last_probe = Instant::now();
        match probe_once() {
            Ok(addr) => {
                self.connected = true;
                self.detail = format!("Service IPC {addr}");
            }
            Err(err) => {
                self.connected = false;
                self.detail = err;
            }
        }
    }
}

fn probe_once() -> Result<String, String> {
    let path = endpoint_path().ok_or_else(|| "No backend service".to_string())?;
    if !path.is_file() {
        return Err("No backend service".into());
    }
    let text = std::fs::read_to_string(&path).map_err(|_| "No backend service".to_string())?;
    let ep: IpcEndpoint = serde_json::from_str(&text).map_err(|_| "No backend service".to_string())?;
    if ep.port == 0 {
        return Err("No backend service".into());
    }
    // Plan lock: localhost only — reject non-loopback hosts in the file.
    let host = ep.host.trim();
    if host != "127.0.0.1" && host != "localhost" && host != "::1" {
        return Err("Service endpoint not localhost".into());
    }
    let addr = format!("{}:{}", host, ep.port);
    let mut addrs = addr
        .to_socket_addrs()
        .map_err(|_| format!("Service offline ({addr})"))?;
    let sock: SocketAddr = addrs
        .next()
        .ok_or_else(|| format!("Service offline ({addr})"))?;
    TcpStream::connect_timeout(&sock, CONNECT_TIMEOUT)
        .map_err(|_| format!("Service offline ({addr})"))?;
    Ok(addr)
}

fn endpoint_path() -> Option<std::path::PathBuf> {
    directories::ProjectDirs::from("ca", "AgileAutomation", "ProtocolBench")
        .map(|d| d.config_dir().join("ipc.json"))
}
