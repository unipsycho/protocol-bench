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

fn stream_bind_source(endpoint: &str, field: &str) -> String {
    let endpoint = endpoint.trim();
    let field = field.trim();
    let base = if endpoint.is_empty() {
        "stream".to_string()
    } else {
        format!("stream/{endpoint}")
    };
    if field.is_empty() {
        base
    } else {
        format!("{base}::{field}")
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
