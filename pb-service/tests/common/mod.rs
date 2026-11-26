//! Shared harness: run the real `pb-service` binary against a throwaway config dir.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use pb_ipc::{IpcEndpoint, RemoteHandle};

pub struct Service {
    pub child: Child,
    pub cfg: PathBuf,
}

impl Service {
    pub fn start(xdg: &Path, env_project: Option<&Path>) -> Self {
        let cfg = xdg.join("protocolbench");
        let _ = std::fs::remove_file(cfg.join("ipc.json"));
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_pb-service"));
        cmd.env("XDG_CONFIG_HOME", xdg)
            .env("PB_NO_TRAY", "1")
            .env("PB_IPC_PORT", "0")
            .env_remove("PB_PROJECT")
            .env_remove("PB_MODULES")
            .env_remove("PB_AUTO_RUN_HUBS")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some(p) = env_project {
            cmd.env("PB_PROJECT", p);
        }
        let child = cmd.spawn().expect("spawn pb-service");
        Self { child, cfg }
    }

    pub fn connect(&self) -> RemoteHandle {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let ep = std::fs::read_to_string(self.cfg.join("ipc.json"))
                .ok()
                .and_then(|t| serde_json::from_str::<IpcEndpoint>(&t).ok());
            let token = std::fs::read_to_string(self.cfg.join("ipc.token")).ok();
            if let (Some(ep), Some(token)) = (ep, token) {
                if ep.port != 0 {
                    if let Ok(h) = RemoteHandle::connect_to(&ep, token.trim()) {
                        return h;
                    }
                }
            }
            assert!(Instant::now() < deadline, "service never came up");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    pub fn kill(mut self) {
        let _ = self.child.kill(); // SIGKILL: no clean shutdown, like a crash or `kill -9`
        let _ = self.child.wait();
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pb-svc-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

pub fn write_project(path: &Path, title: &str) {
    let doc = serde_json::json!({ "version": 11, "hubs": { "hubs": [] }, "settings": { "title": title } });
    std::fs::write(path, serde_json::to_string_pretty(&doc).unwrap()).unwrap();
}

pub fn same(a: &str, b: &Path) -> bool {
    let a = std::fs::canonicalize(a).unwrap_or_else(|_| PathBuf::from(a));
    let b = std::fs::canonicalize(b).unwrap_or_else(|_| b.to_path_buf());
    a == b
}


impl Service {
    /// Endpoint + token as written by the service (for raw-socket tests).
    pub fn endpoint(&self) -> (IpcEndpoint, String) {
        let _ = self.connect();
        let ep = serde_json::from_str(&std::fs::read_to_string(self.cfg.join("ipc.json")).unwrap()).unwrap();
        let token = std::fs::read_to_string(self.cfg.join("ipc.token")).unwrap().trim().to_string();
        (ep, token)
    }
}
