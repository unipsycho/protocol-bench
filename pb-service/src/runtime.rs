//! Service-owned engine + one active project.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use pb_engine::{EngineHandle, EngineId, EngineOptions, HostSnapshot};
use pb_ipc::{status_from_host, PointBindingWire, ServiceStatus};
use pb_points::addr::{file_bind_source, http_bind_source, las_bind_source, mqtt_bind_source, snmp_bind_source, stream_bind_source};
use pb_points::{PointBinding, PointStore};

pub struct Runtime {
    pub engine: EngineHandle,
    pub project_path: PathBuf,
    pub project_json: serde_json::Value,
    pub last_error: String,
    pub auto_run_hubs: bool,
}

impl Runtime {
    /// Spawn the engine (loading packs from `modules`) and, when `project_path` exists, apply it.
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
        // Headless / demo hosts: start embedded brokers and servers without a UI attaching.
        let auto_run = std::env::var("PB_AUTO_RUN_HUBS").is_ok_and(|v| matches!(v.trim(), "1" | "true" | "yes" | "on"));
        if project_path.is_file() {
            if let Err(err) = rt.load_project_file(&project_path, auto_run) {
                rt.last_error = err;
            }
        }
        rt
    }

    /// Engine snapshot (slots + points).
    pub fn snapshot(&self) -> HostSnapshot {
        self.engine.snapshot()
    }

    /// Service status including pack list, hub state and the last project error.
    pub fn status(&self) -> ServiceStatus {
        let host = self.snapshot();
        let mut st = status_from_host(
            &host,
            &self.project_path.display().to_string(),
            &self.last_error,
        );
        // Prefer detail_json flags when packs expose them.
        if let Some(slot) = host.slot(EngineId::Mqtt) {
            // Pack reports state "broker" while rumqttd is accepting; detail also
            // carries a role=broker row when the hub listener is up (or last error).
            if slot.state == "broker"
                || slot.state.contains("listen")
                || slot.detail_json.contains("\"role\":\"broker\"")
            {
                st.mqtt_hub_running = slot.connected
                    || slot.state == "broker"
                    || slot.state.contains("listen")
                    || slot.detail_json.contains("\"state\":\"listening\"");
            }
        }
        if let Some(slot) = host.slot(EngineId::OpcUa) {
            st.opcua_hub_running = slot.detail_json.contains("\"running\":true")
                || slot.state.contains("listen");
        }
        st
    }

    /// Stop every embedded hub, client and endpoint the current project started.
    ///
    /// Each pack reconciles against an empty config (clients/endpoints/sources removed and
    /// stopped); the embedded MQTT broker and OPC UA server need an explicit stop because
    /// `Configure` never stops them. Commands queue per pack, so a following `Configure`
    /// for the next project runs after the stop.
    pub fn stop_all_hubs(&mut self) {
        let empty = serde_json::json!({});
        let configs = [
            (EngineId::Mqtt, build_mqtt_configure(&empty, None, false)),
            (EngineId::OpcUa, serde_json::json!({ "clients": [] }).to_string()),
            (EngineId::Streams, build_streams_configure(&empty, None, false)),
            (EngineId::Http, build_http_configure(&empty, None)),
            (EngineId::Modbus, build_modbus_configure(&empty, None)),
            (EngineId::FileFollow, build_files_configure(&empty, None, None)),
            (EngineId::Snmp, build_snmp_configure(&empty, None)),
            (EngineId::Las, build_las_configure(&empty, None, None)),
        ];
        let ext = pb_points::addr::EXT_PROTOCOLS
            .iter()
            .filter_map(|spec| Some((EngineId::from_ext(spec.id)?, build_ext_configure(spec, &empty, None))));
        for (id, json) in configs.into_iter().chain(ext) {
            self.engine
                .submit(id, pb_engine::EngineCommand::Configure { json });
        }
        self.engine.submit(
            EngineId::Mqtt,
            pb_engine::EngineCommand::Configure {
                json: serde_json::json!({ "op": "broker_stop" }).to_string(),
            },
        );
        self.engine.submit(
            EngineId::OpcUa,
            pb_engine::EngineCommand::Configure {
                json: serde_json::json!({ "op": "server_stop" }).to_string(),
            },
        );
    }

    /// Read, apply and remember a project file. `auto_run_hubs` starts embedded brokers / servers.
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

    /// Apply a project document sent by the UI and save it to disk.
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

    /// Write the project JSON to `project_path` (pretty-printed, trailing newline).
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
        // MQTT pack configure — merge Hub Input/Output binds into parse_rules
        // (same as UI apply_mqtt_session) so subscribed topics upsert points.
        if let Some(mqtt) = value.get("mqtt") {
            let hubs = value.get("hubs");
            let json = build_mqtt_configure(mqtt, hubs, auto_run_hubs);
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
        // Streams / TCP pack — endpoints + hub Input/Output framing maps.
        if let Some(streams) = value.get("streams") {
            let hubs = value.get("hubs");
            let json = build_streams_configure(streams, hubs, auto_run_hubs);
            self.engine.submit(
                EngineId::Streams,
                pb_engine::EngineCommand::Configure { json },
            );
        }
        // HTTP / REST pack — clients + hub poll/push binds.
        if let Some(http) = value.get("http") {
            let hubs = value.get("hubs");
            let json = build_http_configure(http, hubs);
            self.engine.submit(
                EngineId::Http,
                pb_engine::EngineCommand::Configure { json },
            );
        }
        // Modbus pack — client connections + hub Input poll / Output write binds.
        if let Some(modbus) = value.get("modbus") {
            let hubs = value.get("hubs");
            let json = build_modbus_configure(modbus, hubs);
            self.engine.submit(
                EngineId::Modbus,
                pb_engine::EngineCommand::Configure { json },
            );
        }
        // File follow pack — sources + hub Files Input column binds.
        if let Some(files) = value.get("files") {
            let hubs = value.get("hubs");
            let json = build_files_configure(files, hubs, files_state_dir("file-follow").as_deref());
            self.engine.submit(
                EngineId::FileFollow,
                pb_engine::EngineCommand::Configure { json },
            );
        }
        // SNMP pack — targets + hub SNMP Input OID binds.
        if let Some(snmp) = value.get("snmp") {
            let hubs = value.get("hubs");
            let json = build_snmp_configure(snmp, hubs);
            self.engine.submit(
                EngineId::Snmp,
                pb_engine::EngineCommand::Configure { json },
            );
        }
        // LAS pack — endpoints + hub LAS Input (follow) / Output (write) curve binds.
        if let Some(las) = value.get("las") {
            let hubs = value.get("hubs");
            let json = build_las_configure(las, hubs, files_state_dir("las-follow").as_deref());
            self.engine.submit(
                EngineId::Las,
                pb_engine::EngineCommand::Configure { json },
            );
        }
        // Ext protocols (WITS, WebSocket, Sparkplug B, DNP3, CAN) share one config shape.
        for spec in pb_points::addr::EXT_PROTOCOLS {
            let Some(id) = EngineId::from_ext(spec.id) else { continue };
            let section = value.get(spec.id).cloned().unwrap_or_else(|| serde_json::json!({}));
            let json = build_ext_configure(spec, &section, value.get("hubs"));
            self.engine.submit(id, pb_engine::EngineCommand::Configure { json });
        }
        self.last_error.clear();
        Ok(())
    }

    /// Seed the point store with saved short ids before packs start publishing.
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
                let modbus_conn = port
                    .get("modbus_connection")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if let Some(binds) = port.get("modbus_inputs").and_then(|v| v.as_array()) {
                    for bind in binds {
                        let source = modbus_bind_source(modbus_conn, bind);
                        let point = bind.get("point").and_then(|v| v.as_str()).unwrap_or("");
                        if point.is_empty() {
                            let _ = store.ensure_id("modbus", &source);
                        } else {
                            store.bind(point, &source);
                        }
                    }
                }
                let snmp_target = port.get("snmp_target").and_then(|v| v.as_str()).unwrap_or("");
                if let Some(binds) = port.get("snmp_inputs").and_then(|v| v.as_array()) {
                    for bind in binds {
                        let oid = bind.get("oid").and_then(|v| v.as_str()).unwrap_or("");
                        if oid.trim().is_empty() {
                            continue;
                        }
                        let source = snmp_bind_source(snmp_target, oid);
                        let point = bind.get("point").and_then(|v| v.as_str()).unwrap_or("");
                        if point.is_empty() {
                            let _ = store.ensure_id("snmp", &source);
                        } else {
                            store.bind(point, &source);
                        }
                    }
                }
                let las_ep = port.get("las_endpoint").and_then(|v| v.as_str()).unwrap_or("");
                if let Some(binds) = port.get("las_inputs").and_then(|v| v.as_array()) {
                    for bind in binds {
                        let curve = bind.get("curve").and_then(|v| v.as_str()).unwrap_or("");
                        if curve.trim().is_empty() {
                            continue;
                        }
                        let source = las_bind_source(las_ep, curve);
                        let point = bind.get("point").and_then(|v| v.as_str()).unwrap_or("");
                        if point.is_empty() {
                            let _ = store.ensure_id("las", &source);
                        } else {
                            store.bind(point, &source);
                        }
                    }
                }
                if let Some(spec) = pb_points::addr::ext_spec(str_of(port, "protocol")) {
                    let conn = str_of(port, "ext_connection");
                    for bind in port.get("ext_inputs").and_then(|v| v.as_array()).into_iter().flatten() {
                        let Some(leaf) = pb_points::addr::ext_leaf(spec, |k| json_field_text(bind, k)) else { continue };
                        let source = pb_points::addr::ext_bind_source(spec, conn, &leaf);
                        let point = str_of(bind, "point").trim();
                        if point.is_empty() {
                            let _ = store.ensure_id(spec.prefix, &source);
                        } else {
                            store.bind(point, &source);
                        }
                    }
                }
                let file_src = port.get("file_source").and_then(|v| v.as_str()).unwrap_or("");
                if let Some(binds) = port.get("file_inputs").and_then(|v| v.as_array()) {
                    for bind in binds {
                        let column = bind.get("column").and_then(|v| v.as_str()).unwrap_or("");
                        if column.trim().is_empty() {
                            continue;
                        }
                        let source = file_bind_source(file_src, column);
                        let point = bind.get("point").and_then(|v| v.as_str()).unwrap_or("");
                        if point.is_empty() {
                            let _ = store.ensure_id("file", &source);
                        } else {
                            store.bind(point, &source);
                        }
                    }
                }
                let stream_endpoint = port
                    .get("stream_endpoint")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if let Some(binds) = port.get("stream_inputs").and_then(|v| v.as_array()) {
                    for bind in binds {
                        let field = bind.get("field").and_then(|v| v.as_str()).unwrap_or("");
                        let source = stream_bind_source(stream_endpoint, field);
                        let point = bind.get("point").and_then(|v| v.as_str()).unwrap_or("");
                        if point.is_empty() {
                            let _ = store.ensure_id("stream", &source);
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

fn build_mqtt_configure(
    mqtt: &serde_json::Value,
    hubs: Option<&serde_json::Value>,
    auto_run_hubs: bool,
) -> String {
    // Re-shape into the pack's multi-client configure document.
    let mut clients = Vec::new();
    if let Some(embedded) = mqtt.get("embedded") {
        let enabled = embedded
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let emb_id = embedded
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("hub");
        let mut emb = embedded.clone();
        if let Some(obj) = emb.as_object_mut() {
            obj.insert("role".into(), serde_json::json!("broker"));
            obj.insert(
                "auto_start".into(),
                serde_json::json!(auto_run_hubs && enabled),
            );
            // Pack start_embedded reads `users[]` — fold username/password when needed.
            let anonymous = obj
                .get("anonymous")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if !obj.contains_key("users") {
                if anonymous {
                    obj.insert("users".into(), serde_json::json!([]));
                } else if let Some(username) = obj.get("username").and_then(|v| v.as_str()) {
                    if !username.trim().is_empty() {
                        let password = obj
                            .get("password")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        obj.insert(
                            "users".into(),
                            serde_json::json!([{
                                "username": username,
                                "password": password,
                            }]),
                        );
                    } else {
                        obj.insert("users".into(), serde_json::json!([]));
                    }
                }
            }
            merge_hub_mqtt_binds(obj, hubs, emb_id);
        }
        clients.push(emb);
    }
    if let Some(arr) = mqtt.get("clients").and_then(|v| v.as_array()) {
        for c in arr {
            let mut client = c.clone();
            if let Some(obj) = client.as_object_mut() {
                let id = obj
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                merge_hub_mqtt_binds(obj, hubs, &id);
            }
            clients.push(client);
        }
    }
    serde_json::json!({ "clients": clients }).to_string()
}

/// Fold Hub MQTT Input/Output binds for `conn_id` into a client configure object.
fn merge_hub_mqtt_binds(
    client: &mut serde_json::Map<String, serde_json::Value>,
    hubs: Option<&serde_json::Value>,
    conn_id: &str,
) {
    if conn_id.is_empty() {
        return;
    }
    let Some(hub_list) = hubs
        .and_then(|h| h.get("hubs"))
        .and_then(|v| v.as_array())
    else {
        return;
    };

    let mut rules = client
        .get("parse_rules")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut outs = client
        .get("outputs")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut subs = client
        .get("subscriptions")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    for hub in hub_list {
        for key in ["sources", "outputs"] {
            let Some(ports) = hub.get(key).and_then(|v| v.as_array()) else {
                continue;
            };
            for port in ports {
                let mqtt_conn = port
                    .get("mqtt_connection")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if mqtt_conn != conn_id {
                    continue;
                }
                if let Some(binds) = port.get("mqtt_inputs").and_then(|v| v.as_array()) {
                    for bind in binds {
                        let topic = bind.get("topic").and_then(|v| v.as_str()).unwrap_or("");
                        if topic.is_empty() {
                            continue;
                        }
                        let path = bind.get("path").and_then(|v| v.as_str()).unwrap_or("");
                        let point = bind.get("point").and_then(|v| v.as_str()).unwrap_or("");
                        let rule = serde_json::json!({
                            "topic": topic,
                            "path": path,
                            "point": point,
                        });
                        if !rules.iter().any(|r| r == &rule) {
                            rules.push(rule);
                        }
                        let topic_v = serde_json::json!(topic);
                        if !subs.iter().any(|s| s == &topic_v) {
                            subs.push(topic_v);
                        }
                    }
                }
                if let Some(binds) = port.get("mqtt_outputs").and_then(|v| v.as_array()) {
                    for bind in binds {
                        let point = bind.get("point").and_then(|v| v.as_str()).unwrap_or("");
                        let topic = bind.get("topic").and_then(|v| v.as_str()).unwrap_or("");
                        if point.is_empty() || topic.is_empty() {
                            continue;
                        }
                        let out = serde_json::json!({
                            "point": point,
                            "topic": topic,
                            "qos": bind.get("qos").and_then(|v| v.as_u64()).unwrap_or(0),
                            "retain": bind.get("retain").and_then(|v| v.as_bool()).unwrap_or(false),
                        });
                        if !outs.iter().any(|o| o == &out) {
                            outs.push(out);
                        }
                    }
                }
            }
        }
    }

    client.insert("parse_rules".into(), serde_json::Value::Array(rules));
    client.insert("outputs".into(), serde_json::Value::Array(outs));
    client.insert("subscriptions".into(), serde_json::Value::Array(subs));
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

/// Build Streams pack configure JSON from `streams` panel + hub port maps.
fn build_streams_configure(
    streams: &serde_json::Value,
    hubs: Option<&serde_json::Value>,
    auto_run_hubs: bool,
) -> String {
    let mut endpoints = Vec::new();
    if let Some(arr) = streams.get("endpoints").and_then(|v| v.as_array()) {
        for ep in arr {
            let id = ep
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if id.is_empty() {
                continue;
            }
            let mut obj = ep.clone();
            if let Some(map) = obj.as_object_mut() {
                map.insert("auto_connect".into(), serde_json::json!(auto_run_hubs));
            }
            endpoints.push(obj);
        }
    }

    let mut inputs = Vec::new();
    let mut outputs = Vec::new();
    if let Some(hubs) = hubs {
        let hub_list = hubs
            .get("hubs")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        for hub in hub_list {
            collect_stream_port_maps(&hub, "sources", true, &mut inputs, &mut outputs);
            collect_stream_port_maps(&hub, "outputs", false, &mut inputs, &mut outputs);
        }
    }

    serde_json::json!({
        "endpoints": endpoints,
        "inputs": inputs,
        "outputs": outputs,
    })
    .to_string()
}

fn collect_stream_port_maps(
    hub: &serde_json::Value,
    key: &str,
    incoming: bool,
    inputs: &mut Vec<serde_json::Value>,
    outputs: &mut Vec<serde_json::Value>,
) {
    let Some(ports) = hub.get(key).and_then(|v| v.as_array()) else {
        return;
    };
    for port in ports {
        let proto = port
            .get("protocol")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !proto.eq_ignore_ascii_case("streams") {
            continue;
        }
        let endpoint = port
            .get("stream_endpoint")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if endpoint.is_empty() {
            continue;
        }
        let frame = port
            .get("stream_frame")
            .and_then(|v| v.as_str())
            .unwrap_or("delimited")
            .to_string();
        let eol = port
            .get("stream_eol")
            .and_then(|v| v.as_str())
            .unwrap_or("lf")
            .to_string();
        let eol_custom = port
            .get("stream_eol_custom")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let idle_ms = port
            .get("stream_idle_ms")
            .and_then(|v| v.as_f64())
            .unwrap_or(50.0) as u64;
        let parser = port
            .get("stream_parser")
            .and_then(|v| v.as_str())
            .unwrap_or("csv")
            .to_string();
        let fixed_widths = port
            .get("stream_fixed_widths")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let regex = port
            .get("stream_regex")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if incoming {
            let mut binds = Vec::new();
            if let Some(arr) = port.get("stream_inputs").and_then(|v| v.as_array()) {
                for bind in arr {
                    let field = bind
                        .get("field")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let point = bind
                        .get("point")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let source = stream_bind_source(&endpoint, &field);
                    binds.push(serde_json::json!({
                        "field": field,
                        "point": point,
                        "source": source,
                    }));
                }
            }
            if !binds.is_empty() {
                inputs.push(serde_json::json!({
                    "endpoint": endpoint,
                    "frame": frame,
                    "eol": eol,
                    "eol_custom": eol_custom,
                    "idle_ms": idle_ms,
                    "parser": parser,
                    "fixed_widths": fixed_widths,
                    "regex": regex,
                    "binds": binds,
                }));
            }
        } else {
            let mut binds = Vec::new();
            if let Some(arr) = port.get("stream_outputs").and_then(|v| v.as_array()) {
                for bind in arr {
                    let field = bind
                        .get("field")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let point = bind
                        .get("point")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    if point.is_empty() {
                        continue;
                    }
                    binds.push(serde_json::json!({
                        "field": field,
                        "point": point,
                        "source": "",
                    }));
                }
            }
            if !binds.is_empty() {
                outputs.push(serde_json::json!({
                    "endpoint": endpoint,
                    "layout": parser,
                    "eol": eol,
                    "eol_custom": eol_custom,
                    "binds": binds,
                    "interval_ms": 0,
                }));
            }
        }
    }
}

fn build_http_configure(http: &serde_json::Value, hubs: Option<&serde_json::Value>) -> String {
    let mut clients = Vec::new();
    if let Some(arr) = http.get("clients").and_then(|v| v.as_array()) {
        for c in arr {
            let id = c.get("id").and_then(|v| v.as_str()).unwrap_or("");
            if id.is_empty() {
                continue;
            }
            let mut headers = Vec::new();
            if let Some(hs) = c.get("headers").and_then(|v| v.as_array()) {
                for h in hs {
                    headers.push(serde_json::json!({
                        "name": h.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                        "value": h.get("value").and_then(|v| v.as_str()).unwrap_or(""),
                    }));
                }
            }
            clients.push(serde_json::json!({
                "id": id,
                "label": c.get("label").and_then(|v| v.as_str()).unwrap_or(""),
                "base_url": c.get("base_url").and_then(|v| v.as_str()).unwrap_or(""),
                "auth": c.get("auth").and_then(|v| v.as_str()).unwrap_or("none"),
                "token": c.get("token").and_then(|v| v.as_str()).unwrap_or(""),
                "username": c.get("username").and_then(|v| v.as_str()).unwrap_or(""),
                "password": c.get("password").and_then(|v| v.as_str()).unwrap_or(""),
                "timeout_secs": c.get("timeout_secs").and_then(|v| v.as_u64()).unwrap_or(10),
                "headers": headers,
            }));
        }
    }

    let mut inputs = Vec::new();
    let mut outputs = Vec::new();
    if let Some(hubs) = hubs {
        let hub_list = hubs
            .get("hubs")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        for hub in hub_list {
            for key in ["sources", "outputs"] {
                let incoming = key == "sources";
                let Some(ports) = hub.get(key).and_then(|v| v.as_array()) else {
                    continue;
                };
                for port in ports {
                    let proto = port.get("protocol").and_then(|v| v.as_str()).unwrap_or("");
                    if !proto.eq_ignore_ascii_case("http") {
                        continue;
                    }
                    let client = port
                        .get("http_connection")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    if client.is_empty() {
                        continue;
                    }
                    let method = port
                        .get("http_method")
                        .and_then(|v| v.as_str())
                        .unwrap_or(if incoming { "GET" } else { "POST" })
                        .to_string();
                    let path = port
                        .get("http_url_path")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let interval = port
                        .get("http_interval_secs")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(5.0);
                    if incoming {
                        let mut binds = Vec::new();
                        if let Some(arr) = port.get("http_inputs").and_then(|v| v.as_array()) {
                            for bind in arr {
                                let json_path = bind
                                    .get("json_path")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let point = bind
                                    .get("point")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let url_path = bind
                                    .get("path")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or(path.as_str())
                                    .to_string();
                                let source = http_bind_source(&client, &url_path, &json_path);
                                binds.push(serde_json::json!({
                                    "json_path": json_path,
                                    "point": point,
                                    "source": source,
                                }));
                            }
                        }
                        if !binds.is_empty() {
                            inputs.push(serde_json::json!({
                                "client": client,
                                "method": method,
                                "path": path,
                                "interval_secs": interval,
                                "binds": binds,
                            }));
                        }
                    } else {
                        let mut binds = Vec::new();
                        if let Some(arr) = port.get("http_outputs").and_then(|v| v.as_array()) {
                            for bind in arr {
                                let json_path = bind
                                    .get("json_path")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let point = bind
                                    .get("point")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                if point.is_empty() || json_path.is_empty() {
                                    continue;
                                }
                                binds.push(serde_json::json!({
                                    "json_path": json_path,
                                    "point": point,
                                    "source": "",
                                }));
                            }
                        }
                        if !binds.is_empty() {
                            outputs.push(serde_json::json!({
                                "client": client,
                                "method": method,
                                "path": path,
                                "binds": binds,
                            }));
                        }
                    }
                }
            }
        }
    }

    serde_json::json!({
        "clients": clients,
        "inputs": inputs,
        "outputs": outputs,
    })
    .to_string()
}

/// Wire address of a Modbus bind given as JSON (same format the UI builds — see `pb_points::addr`).
fn modbus_bind_source(conn: &str, bind: &serde_json::Value) -> String {
    let s = |k: &str, d: &str| bind.get(k).and_then(|v| v.as_str()).unwrap_or(d).to_string();
    let n = |k: &str| bind.get(k).and_then(|v| v.as_u64()).unwrap_or(0);
    pb_points::addr::modbus_bind_source(conn, n("unit"), &s("area", "holding"), n("address"), &s("dtype", "u16"), &s("order", "abcd"))
}

/// Build Modbus pack configure JSON from the `modbus` section + hub Modbus Input / Output ports.
fn build_modbus_configure(modbus: &serde_json::Value, hubs: Option<&serde_json::Value>) -> String {
    // Connection objects already use the pack's field names (id, role, transport, host, port, …).
    // A Server connection only runs when a Modbus hub picks it (`hub.modbus_server`).
    let used_servers: std::collections::BTreeSet<String> = hubs
        .and_then(|h| h.get("hubs"))
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter(|hub| {
            hub.get("protocol")
                .and_then(|v| v.as_str())
                .is_some_and(|p| p.eq_ignore_ascii_case("modbus"))
        })
        .filter_map(|hub| hub.get("modbus_server").and_then(|v| v.as_str()))
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .collect();
    let connections: Vec<serde_json::Value> = modbus
        .get("connections")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|mut c| {
            let is_server = c.get("role").and_then(|v| v.as_str()) == Some("server");
            if is_server {
                let id = c.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let on = c.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);
                c["enabled"] = serde_json::json!(on && used_servers.contains(&id));
            }
            c
        })
        .collect();
    let mut inputs = Vec::new();
    let mut outputs = Vec::new();
    let hub_list = hubs
        .and_then(|h| h.get("hubs"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    for hub in hub_list {
        for key in ["sources", "outputs"] {
            let incoming = key == "sources";
            let Some(ports) = hub.get(key).and_then(|v| v.as_array()) else {
                continue;
            };
            for port in ports {
                let proto = port.get("protocol").and_then(|v| v.as_str()).unwrap_or("");
                if !proto.eq_ignore_ascii_case("modbus") {
                    continue;
                }
                let conn = port
                    .get("modbus_connection")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if conn.is_empty() {
                    continue;
                }
                let list_key = if incoming { "modbus_inputs" } else { "modbus_outputs" };
                let mut binds = Vec::new();
                for bind in port.get(list_key).and_then(|v| v.as_array()).into_iter().flatten() {
                    let mut b = bind.clone();
                    if incoming {
                        b["source"] = serde_json::json!(modbus_bind_source(&conn, bind));
                    } else if bind.get("point").and_then(|v| v.as_str()).unwrap_or("").is_empty() {
                        continue; // an Output bind without a point has nothing to write
                    }
                    binds.push(b);
                }
                if binds.is_empty() {
                    continue;
                }
                if incoming {
                    inputs.push(serde_json::json!({
                        "connection": conn,
                        "interval_secs": port
                            .get("modbus_interval_secs")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(1.0),
                        "binds": binds,
                    }));
                } else {
                    outputs.push(serde_json::json!({ "connection": conn, "binds": binds }));
                }
            }
        }
    }
    serde_json::json!({ "connections": connections, "inputs": inputs, "outputs": outputs }).to_string()
}

/// Where File follow keeps its per-source byte offsets (`<config dir>/file-follow`).
fn files_state_dir(sub: &str) -> Option<String> {
    pb_ipc::config_dir().map(|d| d.join(sub).to_string_lossy().into_owned())
}

/// Build File follow pack configure JSON from the `files` section + hub `Files` Input ports.
fn build_files_configure(files: &serde_json::Value, hubs: Option<&serde_json::Value>, state_dir: Option<&str>) -> String {
    let sources = files.get("sources").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let mut inputs = Vec::new();
    for hub in hubs.and_then(|h| h.get("hubs")).and_then(|v| v.as_array()).into_iter().flatten() {
        for port in hub.get("sources").and_then(|v| v.as_array()).into_iter().flatten() {
            let proto = port.get("protocol").and_then(|v| v.as_str()).unwrap_or("");
            if !proto.eq_ignore_ascii_case("files") {
                continue;
            }
            let source = port.get("file_source").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if source.is_empty() {
                continue;
            }
            let binds: Vec<serde_json::Value> = port
                .get("file_inputs")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
                .filter_map(|b| {
                    let column = b.get("column").and_then(|v| v.as_str())?.trim().to_string();
                    if column.is_empty() {
                        return None;
                    }
                    Some(serde_json::json!({
                        "column": column,
                        "point": b.get("point").and_then(|v| v.as_str()).unwrap_or(""),
                        "source": file_bind_source(&source, &column),
                    }))
                })
                .collect();
            if !binds.is_empty() {
                inputs.push(serde_json::json!({ "source": source, "binds": binds }));
            }
        }
    }
    serde_json::json!({ "state_dir": state_dir.unwrap_or(""), "sources": sources, "inputs": inputs }).to_string()
}

fn str_of<'a>(v: &'a serde_json::Value, k: &str) -> &'a str {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("")
}

/// A bind field as text whether it was saved as a string, number or bool.
fn json_field_text(v: &serde_json::Value, k: &str) -> String {
    match v.get(k) {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        Some(serde_json::Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

/// Configure JSON for an ext protocol pack:
/// `{connections, inputs: [{connection, interval_secs, binds: [{…, source}]}], outputs: [{connection, interval_secs, binds}]}`.
/// Connections are passed through (unset `enabled` = on); hub ports of this protocol become
/// inputs (Input nodes, each bind gets its wire `source`) and outputs (Output nodes, binds
/// without a point are dropped).
fn build_ext_configure(spec: &pb_points::addr::ExtSpec, section: &serde_json::Value, hubs: Option<&serde_json::Value>) -> String {
    let connections: Vec<serde_json::Value> = section
        .get("connections")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|c| !str_of(c, "id").trim().is_empty())
        .map(|mut c| {
            if c.get("enabled").is_none() {
                c["enabled"] = serde_json::json!(true);
            }
            c
        })
        .collect();
    let mut inputs = Vec::new();
    let mut outputs = Vec::new();
    for hub in hubs.and_then(|h| h.get("hubs")).and_then(|v| v.as_array()).into_iter().flatten() {
        for key in ["sources", "outputs"] {
            let incoming = key == "sources";
            for port in hub.get(key).and_then(|v| v.as_array()).into_iter().flatten() {
                if !str_of(port, "protocol").eq_ignore_ascii_case(spec.id) {
                    continue;
                }
                let conn = str_of(port, "ext_connection").trim().to_string();
                if conn.is_empty() {
                    continue;
                }
                let interval = port.get("ext_interval_secs").and_then(|v| v.as_f64()).unwrap_or(1.0);
                let list = port.get(if incoming { "ext_inputs" } else { "ext_outputs" }).and_then(|v| v.as_array());
                let mut binds = Vec::new();
                for b in list.into_iter().flatten() {
                    if incoming {
                        let Some(leaf) = pb_points::addr::ext_leaf(spec, |k| json_field_text(b, k)) else { continue };
                        let mut b = b.clone();
                        b["source"] = serde_json::json!(pb_points::addr::ext_bind_source(spec, &conn, &leaf));
                        binds.push(b);
                    } else if !str_of(b, "point").trim().is_empty() {
                        binds.push(b.clone());
                    }
                }
                if binds.is_empty() {
                    continue;
                }
                let entry = serde_json::json!({ "connection": conn, "interval_secs": interval, "binds": binds });
                if incoming {
                    inputs.push(entry);
                } else {
                    outputs.push(entry);
                }
            }
        }
    }
    serde_json::json!({ "connections": connections, "inputs": inputs, "outputs": outputs }).to_string()
}

#[cfg(test)]
mod ext_tests {
    use super::*;

    #[test]
    fn ext_config_splits_ports_and_adds_sources() {
        let spec = pb_points::addr::ext_spec("wits").unwrap();
        let section = serde_json::json!({"connections":[{"id":"van","transport":"tcp_listen","port":5100},{"id":""}]});
        let hubs = serde_json::json!({"hubs":[{"protocol":"Modbus",
            "sources":[{"protocol":"Wits","ext_connection":"van","ext_interval_secs":2.0,
                        "ext_inputs":[{"item":"0108","point":""},{"item":" ","point":""}]},
                       {"protocol":"Modbus","ext_connection":"van","ext_inputs":[{"item":"0110"}]}],
            "outputs":[{"protocol":"Wits","ext_connection":"van","ext_outputs":[{"item":"0121","point":"modbus_1"},{"item":"0122","point":""}]},
                       {"protocol":"Wits","ext_connection":"","ext_outputs":[{"item":"0121","point":"x"}]}]}]});
        let out: serde_json::Value = serde_json::from_str(&build_ext_configure(spec, &section, Some(&hubs))).unwrap();
        assert_eq!(out["connections"].as_array().unwrap().len(), 1);
        assert_eq!(out["connections"][0]["enabled"], true);
        assert_eq!(out["inputs"].as_array().unwrap().len(), 1);
        assert_eq!(out["inputs"][0]["interval_secs"], 2.0);
        assert_eq!(out["inputs"][0]["binds"].as_array().unwrap().len(), 1);
        assert_eq!(out["inputs"][0]["binds"][0]["source"], "wits/van::0108");
        assert_eq!(out["outputs"].as_array().unwrap().len(), 1);
        assert_eq!(out["outputs"][0]["binds"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn materialize_allocates_ext_ids_with_numeric_fields() {
        let mut store = PointStore::new();
        let value = serde_json::json!({"hubs":{"hubs":[{"sources":[
            {"protocol":"Dnp3","ext_connection":"rtu","ext_inputs":[{"kind":"analog","index":3,"point":""}]},
            {"protocol":"Sparkplug","ext_connection":"line","ext_inputs":[{"device":"Robot1","metric":"Torque","point":"robot_torque"}]}],
            "outputs":[]}]}});
        materialize_hub_binds_json(&value, &mut store);
        assert_eq!(store.id_for_source("dnp3/rtu::analog:3"), Some("dnp3_1"));
        assert_eq!(store.id_for_source("sparkplug/line::Robot1:Torque"), Some("robot_torque"));
    }
}

/// Build SNMP pack configure JSON from the `snmp` targets + hub SNMP Input ports.
fn build_snmp_configure(snmp: &serde_json::Value, hubs: Option<&serde_json::Value>) -> String {
    let targets = snmp.get("targets").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let mut inputs = Vec::new();
    for hub in hubs.and_then(|h| h.get("hubs")).and_then(|v| v.as_array()).into_iter().flatten() {
        for port in hub.get("sources").and_then(|v| v.as_array()).into_iter().flatten() {
            if !str_of(port, "protocol").eq_ignore_ascii_case("snmp") {
                continue;
            }
            let target = str_of(port, "snmp_target").to_string();
            if target.is_empty() {
                continue;
            }
            let binds: Vec<serde_json::Value> = port
                .get("snmp_inputs")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
                .filter(|b| !str_of(b, "oid").trim().is_empty())
                .map(|b| {
                    serde_json::json!({
                        "oid": str_of(b, "oid").trim(),
                        "point": str_of(b, "point"),
                        "source": snmp_bind_source(&target, str_of(b, "oid")),
                    })
                })
                .collect();
            if !binds.is_empty() {
                inputs.push(serde_json::json!({
                    "target": target,
                    "interval_secs": port.get("snmp_interval_secs").and_then(|v| v.as_f64()).unwrap_or(10.0),
                    "binds": binds,
                }));
            }
        }
    }
    serde_json::json!({ "targets": targets, "inputs": inputs }).to_string()
}

/// Build LAS pack configure JSON from the `las` endpoints + hub LAS Input / Output ports.
fn build_las_configure(las: &serde_json::Value, hubs: Option<&serde_json::Value>, state_dir: Option<&str>) -> String {
    let endpoints = las.get("endpoints").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let (mut inputs, mut outputs) = (Vec::new(), Vec::new());
    for hub in hubs.and_then(|h| h.get("hubs")).and_then(|v| v.as_array()).into_iter().flatten() {
        for key in ["sources", "outputs"] {
            let incoming = key == "sources";
            for port in hub.get(key).and_then(|v| v.as_array()).into_iter().flatten() {
                if !str_of(port, "protocol").eq_ignore_ascii_case("las") {
                    continue;
                }
                let endpoint = str_of(port, "las_endpoint").to_string();
                if endpoint.is_empty() {
                    continue;
                }
                let list = port.get(if incoming { "las_inputs" } else { "las_outputs" }).and_then(|v| v.as_array());
                let binds: Vec<serde_json::Value> = list
                    .into_iter()
                    .flatten()
                    .filter(|b| !str_of(b, "curve").trim().is_empty())
                    .filter(|b| incoming || !str_of(b, "point").trim().is_empty())
                    .map(|b| {
                        serde_json::json!({
                            "curve": str_of(b, "curve").trim(),
                            "point": str_of(b, "point"),
                            "source": if incoming { las_bind_source(&endpoint, str_of(b, "curve")) } else { String::new() },
                        })
                    })
                    .collect();
                if binds.is_empty() {
                    continue;
                }
                if incoming {
                    inputs.push(serde_json::json!({ "endpoint": endpoint, "binds": binds }));
                } else {
                    let index = match str_of(port, "las_index") {
                        "" => "depth",
                        other => other,
                    };
                    outputs.push(serde_json::json!({ "endpoint": endpoint, "index": index, "binds": binds }));
                }
            }
        }
    }
    serde_json::json!({ "state_dir": state_dir.unwrap_or(""), "endpoints": endpoints, "inputs": inputs, "outputs": outputs }).to_string()
}

#[cfg(test)]
mod snmp_las_tests {
    use super::*;

    #[test]
    fn snmp_config_keeps_targets_and_binds() {
        let snmp = serde_json::json!({"targets":[{"id":"sw","host":"10.0.0.2","port":161,"version":"v2c","community":"public","timeout_secs":3}]});
        let hubs = serde_json::json!({"hubs":[{"sources":[
            {"protocol":"Snmp","snmp_target":"sw","snmp_interval_secs":5.0,"snmp_inputs":[{"oid":" 1.3.6.1.2.1.1.5.0","point":"snmp_1"},{"oid":" ","point":"x"}]},
            {"protocol":"Snmp","snmp_target":"","snmp_inputs":[{"oid":"1.3","point":""}]},
            {"protocol":"Http","snmp_target":"sw","snmp_inputs":[{"oid":"1.3","point":""}]}]}]});
        let out: serde_json::Value = serde_json::from_str(&build_snmp_configure(&snmp, Some(&hubs))).unwrap();
        assert_eq!(out["targets"][0]["host"], "10.0.0.2");
        let inputs = out["inputs"].as_array().unwrap();
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0]["interval_secs"], 5.0);
        assert_eq!(inputs[0]["binds"].as_array().unwrap().len(), 1);
        assert_eq!(inputs[0]["binds"][0]["source"], "snmp/sw::1.3.6.1.2.1.1.5.0");
    }

    #[test]
    fn las_config_splits_follow_and_write_and_defaults_index() {
        let las = serde_json::json!({"endpoints":[
            {"id":"in","role":"incoming","path":"/a.las","follow":true},{"id":"out","role":"outgoing","path":"/b.las"}]});
        let hubs = serde_json::json!({"hubs":[{"sources":[
            {"protocol":"Las","las_endpoint":"in","las_inputs":[{"curve":"GR","point":"las_1"},{"curve":" ","point":""}]}],
          "outputs":[
            {"protocol":"Las","las_endpoint":"out","las_index":"","las_outputs":[{"point":"depth","curve":"DEPT"},{"point":"","curve":"GR"}]},
            {"protocol":"Las","las_endpoint":"","las_outputs":[{"point":"p","curve":"GR"}]}]}]});
        let out: serde_json::Value = serde_json::from_str(&build_las_configure(&las, Some(&hubs), Some("/cfg/las"))).unwrap();
        assert_eq!(out["state_dir"], "/cfg/las");
        assert_eq!(out["inputs"][0]["binds"].as_array().unwrap().len(), 1);
        assert_eq!(out["inputs"][0]["binds"][0]["source"], "las/in::GR");
        let o = out["outputs"].as_array().unwrap();
        assert_eq!(o.len(), 1, "endpoint-less port skipped");
        assert_eq!(o[0]["index"], "depth");
        assert_eq!(o[0]["binds"].as_array().unwrap().len(), 1, "output bind without a point dropped");
    }

    #[test]
    fn materialize_allocates_snmp_and_las_ids() {
        let value = serde_json::json!({"hubs":{"hubs":[{"sources":[
            {"protocol":"Snmp","snmp_target":"sw","snmp_inputs":[{"oid":"1.3.6.1.2.1.1.5.0","point":""}]},
            {"protocol":"Las","las_endpoint":"w","las_inputs":[{"curve":"GR","point":""}]}],"outputs":[]}]}});
        let mut store = PointStore::default();
        materialize_hub_binds_json(&value, &mut store);
        assert_eq!(store.id_for_source("snmp/sw::1.3.6.1.2.1.1.5.0"), Some("snmp_1"));
        assert_eq!(store.id_for_source("las/w::GR"), Some("las_2"));
    }
}

#[cfg(test)]
mod files_tests {
    use super::*;

    #[test]
    fn file_sources_and_binds_become_pack_config() {
        let files = serde_json::json!({"sources":[
            {"id":"meter","path":"/var/log/m.csv","parser":"csv","header_row":true,"delimiter":",","poll_ms":500,"start_at":"end","follow_rotation":true}]});
        let hubs = serde_json::json!({"hubs":[{"sources":[
            {"protocol":"Files","file_source":"meter","file_inputs":[{"column":"temp","point":"file_1"},{"column":" ","point":"x"}]},
            {"protocol":"Files","file_source":"","file_inputs":[{"column":"a","point":""}]},
            {"protocol":"Modbus","file_source":"meter","file_inputs":[{"column":"a","point":""}]}]}]});
        let out: serde_json::Value = serde_json::from_str(&build_files_configure(&files, Some(&hubs), Some("/cfg/ff"))).unwrap();
        assert_eq!(out["state_dir"], "/cfg/ff");
        assert_eq!(out["sources"][0]["id"], "meter");
        let inputs = out["inputs"].as_array().unwrap();
        assert_eq!(inputs.len(), 1, "blank-source and non-Files ports are skipped");
        assert_eq!(inputs[0]["binds"].as_array().unwrap().len(), 1, "blank column dropped");
        assert_eq!(inputs[0]["binds"][0]["source"], "file/meter::temp");
    }

    #[test]
    fn materialize_allocates_file_ids() {
        let value = serde_json::json!({"hubs":{"hubs":[{"sources":[
            {"protocol":"Files","file_source":"meter","file_inputs":[{"column":"temp","point":""}]}],"outputs":[]}]}});
        let mut store = PointStore::default();
        materialize_hub_binds_json(&value, &mut store);
        assert_eq!(store.id_for_source("file/meter::temp"), Some("file_1"));
    }
}

#[cfg(test)]
mod modbus_tests {
    use super::*;

    fn bind(area: &str, addr: u64, dt: &str, order: &str, unit: u64, point: &str) -> serde_json::Value {
        serde_json::json!({"area":area,"address":addr,"dtype":dt,"order":order,"unit":unit,"point":point})
    }

    #[test]
    fn source_strings_match_the_ui_format() {
        // Same literals asserted in pb-ui `modbus_source_strings_match_the_service`.
        assert_eq!(modbus_bind_source("plc", &bind("holding", 100, "f32", "abcd", 0, "")), "modbus/plc::holding:100:f32");
        assert_eq!(modbus_bind_source("plc", &bind("holding", 100, "f32", "cdab", 0, "")), "modbus/plc::holding:100:f32:cdab");
        assert_eq!(modbus_bind_source("plc", &bind("coil", 3, "bool", "abcd", 7, "")), "modbus/plc/u7::coil:3:bool");
        assert_eq!(modbus_bind_source("", &bind("input", 0, "u16", "abcd", 0, "")), "modbus::input:0:u16");
    }

    #[test]
    fn configure_json_splits_inputs_and_outputs_per_connection() {
        let modbus = serde_json::json!({"connections":[
            {"id":"plc","role":"client","transport":"tcp","host":"10.0.0.5","port":502,"map":[]},
            {"id":"hub","role":"server","transport":"tcp","host":"0.0.0.0","port":1502}]});
        let hubs = serde_json::json!({"hubs":[{"sources":[
            {"protocol":"Modbus","modbus_connection":"plc","modbus_interval_secs":0.5,
             "modbus_inputs":[bind("holding",0,"f32","cdab",0,"modbus_1")]},
            {"protocol":"Mqtt","modbus_connection":"plc","modbus_inputs":[bind("holding",9,"u16","abcd",0,"")]},
            {"protocol":"Modbus","modbus_connection":"","modbus_inputs":[bind("holding",1,"u16","abcd",0,"")]}],
          "outputs":[{"protocol":"Modbus","modbus_connection":"plc","modbus_outputs":[
            bind("holding",10,"f32","cdab",0,"setpoint"), bind("coil",1,"bool","abcd",0,"")]}]}]});
        let out: serde_json::Value = serde_json::from_str(&build_modbus_configure(&modbus, Some(&hubs))).unwrap();
        let conns = out["connections"].as_array().unwrap();
        assert_eq!(conns.len(), 2);
        assert_eq!(conns[1]["enabled"], false, "server no hub picks stays off");
        assert!(conns[0].get("enabled").is_none());
        let inputs = out["inputs"].as_array().unwrap();
        assert_eq!(inputs.len(), 1, "non-Modbus and unconnected ports are skipped");
        assert_eq!(inputs[0]["interval_secs"], 0.5);
        assert_eq!(inputs[0]["binds"][0]["source"], "modbus/plc::holding:0:f32:cdab");
        let outputs = out["outputs"].as_array().unwrap();
        assert_eq!(outputs[0]["binds"].as_array().unwrap().len(), 1, "pointless output bind dropped");
        assert_eq!(outputs[0]["binds"][0]["point"], "setpoint");
    }

    #[test]
    fn server_enabled_only_when_a_modbus_hub_picks_it() {
        let modbus = serde_json::json!({"connections":[
            {"id":"a","role":"server","transport":"tcp"},{"id":"b","role":"server","transport":"tcp"}]});
        let hubs = serde_json::json!({"hubs":[
            {"protocol":"Modbus","modbus_server":"b","sources":[],"outputs":[]},
            {"protocol":"Mqtt","modbus_server":"a","sources":[],"outputs":[]}]});
        let out: serde_json::Value = serde_json::from_str(&build_modbus_configure(&modbus, Some(&hubs))).unwrap();
        assert_eq!(out["connections"][0]["enabled"], false);
        assert_eq!(out["connections"][1]["enabled"], true);
    }

    #[test]
    fn materialize_allocates_modbus_ids() {
        let value = serde_json::json!({"hubs":{"hubs":[{"sources":[
            {"protocol":"Modbus","modbus_connection":"plc","modbus_inputs":[bind("holding",0,"f32","cdab",0,"")]}],
            "outputs":[]}]}});
        let mut store = PointStore::default();
        materialize_hub_binds_json(&value, &mut store);
        assert_eq!(store.id_for_source("modbus/plc::holding:0:f32:cdab"), Some("modbus_1"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_all_hubs_on_empty_engine_is_safe() {
        let mut rt = Runtime::start(None, std::env::temp_dir().join("pb-none.json"));
        rt.stop_all_hubs();
        assert!(rt.status().running_hubs().is_empty());
    }
}
