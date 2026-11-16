//! Service-owned engine + one active project.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use pb_engine::{EngineHandle, EngineId, EngineOptions, HostSnapshot};
use pb_ipc::{status_from_host, PointBindingWire, ServiceStatus};
use pb_points::{PointBinding, PointStore};

pub struct Runtime {
    pub engine: EngineHandle,
    pub project_path: PathBuf,
    pub project_json: serde_json::Value,
    pub last_error: String,
    pub auto_run_hubs: bool,
}

impl Runtime {
    pub fn start(modules: Option<PathBuf>, project_path: PathBuf) -> Self {
        let points = Arc::new(Mutex::new(PointStore::new()));
        let engine = EngineHandle::spawn_with(EngineOptions {
            modules_dir: modules,
            points: Some(Arc::clone(&points)),
        });
        let mut rt = Self {
            engine,
            project_path: project_path.clone(),
            project_json: serde_json::json!({ "version": 7 }),
            last_error: String::new(),
            auto_run_hubs: false,
        };
        if project_path.is_file() {
            if let Err(err) = rt.load_project_file(&project_path, false) {
                rt.last_error = err;
            }
        }
        rt
    }

    pub fn snapshot(&self) -> HostSnapshot {
        self.engine.snapshot()
    }

    pub fn status(&self) -> ServiceStatus {
        let host = self.snapshot();
        let mut st = status_from_host(
            &host,
            &self.project_path.display().to_string(),
            &self.last_error,
        );
        // Prefer detail_json flags when packs expose them.
        if let Some(slot) = host.slot(EngineId::Mqtt) {
            if slot.detail_json.contains("\"role\":\"broker\"")
                || slot.detail_json.contains("broker")
            {
                st.mqtt_hub_running = slot.connected || slot.state.contains("listen");
            }
        }
        if let Some(slot) = host.slot(EngineId::OpcUa) {
            st.opcua_hub_running = slot.detail_json.contains("\"running\":true")
                || slot.state.contains("listen");
        }
        st
    }

    pub fn load_project_file(&mut self, path: &Path, auto_run_hubs: bool) -> Result<(), String> {
        self.project_path = path.to_path_buf();
        self.auto_run_hubs = auto_run_hubs;
        if !path.is_file() {
            // New project — keep empty doc; UI will PutProject.
            self.project_json = serde_json::json!({ "version": 7 });
            return Ok(());
        }
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let value: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        self.apply_project_value(value, auto_run_hubs)
    }

    pub fn put_project(
        &mut self,
        value: serde_json::Value,
        auto_run_hubs: bool,
    ) -> Result<(), String> {
        self.auto_run_hubs = auto_run_hubs;
        self.apply_project_value(value.clone(), auto_run_hubs)?;
        self.persist(&value)
    }

    pub fn persist(&self, value: &serde_json::Value) -> Result<(), String> {
        if let Some(parent) = self.project_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
        std::fs::write(&self.project_path, text + "\n").map_err(|e| e.to_string())
    }

    fn apply_project_value(
        &mut self,
        value: serde_json::Value,
        auto_run_hubs: bool,
    ) -> Result<(), String> {
        self.project_json = value.clone();
        // Points bindings.
        if let Some(points) = value.get("points").and_then(|v| v.as_array()) {
            let mut bindings = Vec::new();
            for p in points {
                let id = p
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let source = p
                    .get("source")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if !id.is_empty() {
                    bindings.push(PointBinding { id, source });
                }
            }
            let next = value
                .get("next_point_seq")
                .and_then(|v| v.as_u64())
                .unwrap_or(1);
            if let Ok(mut store) = self.engine.points().lock() {
                store.import_bindings(&bindings);
                store.set_next_seq_at_least(next.max(1));
            }
        }
        // MQTT pack configure — pass through mqtt section as session JSON the pack understands.
        if let Some(mqtt) = value.get("mqtt") {
            let json = build_mqtt_configure(mqtt, auto_run_hubs);
            self.engine.submit(
                EngineId::Mqtt,
                pb_engine::EngineCommand::Configure { json },
            );
        }
        if let Some(opcua) = value.get("opcua") {
            let json = build_opcua_configure(opcua, auto_run_hubs);
            self.engine.submit(
                EngineId::OpcUa,
                pb_engine::EngineCommand::Configure { json },
            );
        }
        self.last_error.clear();
        Ok(())
    }

    pub fn import_bindings(
        &mut self,
        points: Vec<PointBindingWire>,
        next_point_seq: u64,
    ) -> Result<(), String> {
        let bindings: Vec<PointBinding> = points
            .into_iter()
            .map(|p| PointBinding {
                id: p.id,
                source: p.source,
            })
            .collect();
        if let Ok(mut store) = self.engine.points().lock() {
            store.import_bindings(&bindings);
            store.set_next_seq_at_least(next_point_seq.max(1));
        }
        Ok(())
    }
}

fn build_mqtt_configure(mqtt: &serde_json::Value, auto_run_hubs: bool) -> String {
    // Re-shape into the pack's multi-client configure document.
    let mut clients = Vec::new();
    if let Some(embedded) = mqtt.get("embedded") {
        let enabled = embedded
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let mut emb = embedded.clone();
        if let Some(obj) = emb.as_object_mut() {
            obj.insert("role".into(), serde_json::json!("broker"));
            obj.insert(
                "auto_start".into(),
                serde_json::json!(auto_run_hubs && enabled),
            );
        }
        clients.push(emb);
    }
    if let Some(arr) = mqtt.get("clients").and_then(|v| v.as_array()) {
        for c in arr {
            clients.push(c.clone());
        }
    }
    serde_json::json!({ "clients": clients }).to_string()
}

fn build_opcua_configure(opcua: &serde_json::Value, auto_run_hubs: bool) -> String {
    let mut out = opcua.clone();
    if let Some(obj) = out.as_object_mut() {
        if let Some(emb) = obj.get_mut("embedded").and_then(|v| v.as_object_mut()) {
            let enabled = emb
                .get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            emb.insert(
                "auto_start".into(),
                serde_json::json!(auto_run_hubs && enabled),
            );
        }
    }
    out.to_string()
}
