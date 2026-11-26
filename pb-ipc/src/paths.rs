//! Config-dir paths for IPC endpoint + token (plan Q2 default).

use std::path::{Path, PathBuf};

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

/// `last-project` — path of the project the service had open last, so a restarted service
/// (killed, rebooted, crashed) comes back on the same project instead of the default one.
pub fn last_project_marker() -> Option<PathBuf> {
    config_dir().map(|d| d.join("last-project"))
}

/// Remember `project` as the one to reopen on the next start. Best effort.
pub fn write_last_project(project: &Path) -> Result<(), String> {
    let marker = last_project_marker().ok_or_else(|| "no config dir".to_string())?;
    write_last_project_to(&marker, project)
}

/// [`write_last_project`] with an explicit marker file (tests).
pub fn write_last_project_to(marker: &Path, project: &Path) -> Result<(), String> {
    let abs = std::fs::canonicalize(project).unwrap_or_else(|_| project.to_path_buf());
    if let Some(dir) = marker.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(marker, abs.display().to_string()).map_err(|e| e.to_string())
}

/// The remembered project, when the marker names a file that still exists.
pub fn read_last_project_from(marker: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(marker).ok()?;
    let path = PathBuf::from(text.trim());
    (!text.trim().is_empty() && path.is_file()).then_some(path)
}

/// Project the service opens at start: `PB_PROJECT` wins, then the remembered last project
/// (if it still exists), then the default `project.json`.
pub fn startup_project_path(env: Option<PathBuf>, marker: Option<&Path>, default: Option<PathBuf>) -> Option<PathBuf> {
    env.or_else(|| marker.and_then(read_last_project_from)).or(default)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pb-ipc-paths-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn last_project_roundtrip_and_missing_file() {
        let dir = scratch("roundtrip");
        let marker = dir.join("cfg").join("last-project");
        let proj = dir.join("plant.json");
        std::fs::write(&proj, "{}").unwrap();
        assert_eq!(read_last_project_from(&marker), None, "no marker yet");
        write_last_project_to(&marker, &proj).unwrap();
        let got = read_last_project_from(&marker).expect("remembered");
        assert_eq!(std::fs::canonicalize(&got).unwrap(), std::fs::canonicalize(&proj).unwrap());
        // The file goes away -> forgotten, not a dangling path.
        std::fs::remove_file(&proj).unwrap();
        assert_eq!(read_last_project_from(&marker), None);
        std::fs::write(&marker, "   \n").unwrap();
        assert_eq!(read_last_project_from(&marker), None, "blank marker");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn startup_order_env_then_last_then_default() {
        let dir = scratch("order");
        let marker = dir.join("last-project");
        let last = dir.join("last.json");
        std::fs::write(&last, "{}").unwrap();
        write_last_project_to(&marker, &last).unwrap();
        let env = Some(PathBuf::from("/env/project.json"));
        let def = Some(PathBuf::from("/cfg/project.json"));
        assert_eq!(startup_project_path(env.clone(), Some(&marker), def.clone()), env);
        let got = startup_project_path(None, Some(&marker), def.clone()).unwrap();
        assert!(got.ends_with("last.json"), "{got:?}");
        assert_eq!(startup_project_path(None, None, def.clone()), def);
        std::fs::remove_file(&last).unwrap();
        assert_eq!(startup_project_path(None, Some(&marker), def.clone()), def, "stale marker falls back");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
