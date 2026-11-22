//! Config-dir paths for IPC endpoint + token (plan Q2 default).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// `{ "host":"127.0.0.1", "port": … }` written by the service on listen.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcEndpoint {
    pub host: String,
    pub port: u16,
}

impl Default for IpcEndpoint {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 0,
        }
    }
}

/// Per-user config folder shared by the UI and the service (`~/.config/ProtocolBench` style). Tests redirect it with `XDG_CONFIG_HOME`.
pub fn config_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("ca", "AgileAutomation", "ProtocolBench")
        .map(|d| d.config_dir().to_path_buf())
}

/// `ipc.json` — where the running service publishes its localhost port. Not a project file: never save over it.
pub fn endpoint_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("ipc.json"))
}

/// `ipc.token` — shared secret the UI sends in `hello`. Not a project file.
pub fn token_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("ipc.token"))
}

/// Project the service opens when `PB_PROJECT` is not set.
pub fn default_project_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("project.json"))
}

/// Read the service endpoint (host + port).
pub fn read_endpoint() -> Result<IpcEndpoint, String> {
    let path = endpoint_path().ok_or_else(|| "no config dir".to_string())?;
    let text = std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

/// Read the shared auth token.
pub fn read_token() -> Result<String, String> {
    let path = token_path().ok_or_else(|| "no config dir".to_string())?;
    let text = std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    Ok(text.trim().to_string())
}

/// Publish the endpoint. Called by the service once it is listening; the port changes on every start.
pub fn write_endpoint(ep: &IpcEndpoint) -> Result<(), String> {
    let dir = config_dir().ok_or_else(|| "no config dir".to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("ipc.json");
    let text = serde_json::to_string_pretty(ep).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| e.to_string())
}

/// Persist the auth token.
pub fn write_token(token: &str) -> Result<(), String> {
    let dir = config_dir().ok_or_else(|| "no config dir".to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("ipc.token");
    std::fs::write(&path, token).map_err(|e| e.to_string())
}

/// Create token if missing; return current token.
pub fn ensure_token() -> Result<String, String> {
    let path = token_path().ok_or_else(|| "no config dir".to_string())?;
    if path.is_file() {
        return read_token();
    }
    let token = fresh_token();
    write_token(&token)?;
    Ok(token)
}

fn fresh_token() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("pb-{nanos:x}")
}
