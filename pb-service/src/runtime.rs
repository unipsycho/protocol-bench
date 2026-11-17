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
        path: Option<PathBuf>,
    ) -> Result<(), String> {
        if let Some(path) = path {
            self.project_path = path;
        }
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
        // Points bindings from project.points[], then Hub Input/agg binds (IPC wipe recovery).
        let next = value
            .get("next_point_seq")
            .and_then(|v| v.as_u64())
            .unwrap_or(1);
        if let Ok(mut store) = self.engine.points().lock() {
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
                    if !id.is_empty() && !source.is_empty() {
                        bindings.push(PointBinding { id, source });
                    }
                }
                store.import_bindings(&bindings);
            }
            materialize_hub_binds_json(&value, &mut store);
            store.set_next_seq_at_least(next.max(1));
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

/// Derive point-bus rows from hub Input / OPC UA / aggregator JSON when
/// `points[]` was wiped by an empty UI mirror (PB-108).
fn materialize_hub_binds_json(value: &serde_json::Value, store: &mut PointStore) {
    let Some(hubs) = value
        .get("hubs")
        .and_then(|h| h.get("hubs"))
        .and_then(|v| v.as_array())
    else {
        return;
    };
    for hub in hubs {
        for key in ["sources", "outputs"] {
            let Some(ports) = hub.get(key).and_then(|v| v.as_array()) else {
                continue;
            };
            for port in ports {
                let mqtt_conn = port
                    .get("mqtt_connection")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if let Some(binds) = port.get("mqtt_inputs").and_then(|v| v.as_array()) {
                    for bind in binds {
                        let topic = bind.get("topic").and_then(|v| v.as_str()).unwrap_or("");
                        if topic.is_empty() {
                            continue;
                        }
                        let path = bind.get("path").and_then(|v| v.as_str()).unwrap_or("");
                        let source = mqtt_bind_source(mqtt_conn, topic, path);
                        let point = bind.get("point").and_then(|v| v.as_str()).unwrap_or("");
                        if point.is_empty() {
                            let _ = store.ensure_id("mqtt", &source);
                        } else {
                            store.bind(point, &source);
                        }
                    }
                }
                let opc_conn = port
                    .get("opcua_connection")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if let Some(binds) = port.get("opcua_inputs").and_then(|v| v.as_array()) {
                    for bind in binds {
                        let node = bind.get("node_id").and_then(|v| v.as_str()).unwrap_or("");
                        if node.is_empty() {
                            continue;
                        }
                        let source = if opc_conn.is_empty() {
                            format!("opcua/{node}")
                        } else {
                            format!("opcua/{opc_conn}/{node}")
                        };
                        let point = bind.get("point").and_then(|v| v.as_str()).unwrap_or("");
                        if point.is_empty() {
                            let _ = store.ensure_id("opc", &source);
                        } else {
                            store.bind(point, &source);
                        }
                    }
                }
                let http_conn = port
                    .get("http_connection")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if let Some(binds) = port.get("http_inputs").and_then(|v| v.as_array()) {
                    for bind in binds {
                        let json_path =
                            bind.get("json_path").and_then(|v| v.as_str()).unwrap_or("");
                        if json_path.is_empty() {
                            continue;
                        }
                        let url_path = bind.get("path").and_then(|v| v.as_str()).unwrap_or("");
                        let source = http_bind_source(http_conn, url_path, json_path);
                        let point = bind.get("point").and_then(|v| v.as_str()).unwrap_or("");
                        if point.is_empty() {
                            let _ = store.ensure_id("http", &source);
                        } else {
                            store.bind(point, &source);
                        }
                    }
                }
            }
        }
        if let Some(aggs) = hub.get("aggregators").and_then(|v| v.as_array()) {
            for step in aggs {
                let Some(id) = step.get("id").and_then(|v| v.as_u64()) else {
                    continue;
                };
                store.bind(&format!("agg_{id}"), &format!("agg/step/{id}"));
            }
        }
    }
}

fn mqtt_bind_source(conn: &str, topic: &str, path: &str) -> String {
    let base = if conn.is_empty() {
        format!("mqtt/{topic}")
    } else {
        format!("mqtt/{conn}/{topic}")
    };
    if path.is_empty() {
        base
    } else {
        format!("{base}::{path}")
    }
}

fn http_bind_source(conn: &str, url_path: &str, json_path: &str) -> String {
    let conn = conn.trim();
    let url_path = url_path.trim().trim_start_matches('/');
    let json_path = json_path.trim();
    let base = if conn.is_empty() {
        if url_path.is_empty() {
            "http".to_string()
        } else {
            format!("http/{url_path}")
        }
    } else if url_path.is_empty() {
        format!("http/{conn}")
    } else {
        format!("http/{conn}/{url_path}")
    };
    if json_path.is_empty() {
        base
    } else {
        format!("{base}::{json_path}")
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
