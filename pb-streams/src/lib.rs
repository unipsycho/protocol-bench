//! Streams / TCP protocol pack (cdylib) for Protocol Bench.
//!
//! Serial + TCP client + TCP listener text endpoints with framing / scan / emit.

mod frame;
mod io;
mod parse;

use frame::FrameConfig;
use io::{EndpointConfig, EndpointWorker, WorkerEvent};
use parse::{join_fields, sample_fields, trailer_bytes};
use pb_abi::{value_text, HostRef, Pack, PB_LOG_ERROR, PB_LOG_INFO, PB_QUALITY_GOOD};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::os::raw::c_int;
use std::time::{Duration, Instant};

const PACK_ID: &[u8] = b"streams\0";
const PACK_TITLE: &[u8] = b"Streams / TCP\0";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PackConfig {
    #[serde(default)]
    endpoints: Vec<EndpointJson>,
    #[serde(default)]
    inputs: Vec<InputMap>,
    #[serde(default)]
    outputs: Vec<OutputMap>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EndpointJson {
    pub id: String,
    #[serde(default = "default_kind")]
    pub kind: String,
    #[serde(default)]
    pub serial_port: String,
    #[serde(default = "default_baud")]
    pub baud: u32,
    #[serde(default = "default_data_bits")]
    pub data_bits: u8,
    #[serde(default = "default_parity")]
    pub parity: String,
    #[serde(default = "default_stop")]
    pub stop_bits: u8,
    #[serde(default = "default_flow")]
    pub flow: String,
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_tcp")]
    pub tcp_port: u16,
    #[serde(default = "default_true")]
    pub reconnect: bool,
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default = "default_listen")]
    pub listen_port: u16,
    #[serde(default = "default_true")]
    pub auto_connect: bool,
}

fn default_kind() -> String {
    "serial".into()
}
fn default_baud() -> u32 {
    9600
}
fn default_data_bits() -> u8 {
    8
}
fn default_parity() -> String {
    "none".into()
}
fn default_stop() -> u8 {
    1
}
fn default_flow() -> String {
    "none".into()
}
fn default_host() -> String {
    "127.0.0.1".into()
}
fn default_tcp() -> u16 {
    4001
}
fn default_true() -> bool {
    true
}
fn default_bind() -> String {
    "0.0.0.0".into()
}
fn default_listen() -> u16 {
    5001
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InputMap {
    pub endpoint: String,
    #[serde(default = "default_frame")]
    pub frame: String,
    #[serde(default = "default_eol")]
    pub eol: String,
    #[serde(default)]
    pub eol_custom: String,
    #[serde(default = "default_idle")]
    pub idle_ms: u64,
    #[serde(default = "default_parser")]
    pub parser: String,
    #[serde(default)]
    pub fixed_widths: String,
    #[serde(default)]
    pub regex: String,
    #[serde(default)]
    pub binds: Vec<FieldBind>,
}

fn default_frame() -> String {
    "delimited".into()
}
fn default_eol() -> String {
    "lf".into()
}
fn default_idle() -> u64 {
    50
}
fn default_parser() -> String {
    "csv".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FieldBind {
    #[serde(default)]
    pub field: String,
    pub point: String,
    #[serde(default)]
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OutputMap {
    pub endpoint: String,
    #[serde(default = "default_parser")]
    pub layout: String,
    #[serde(default = "default_eol")]
    pub eol: String,
    #[serde(default)]
    pub eol_custom: String,
    #[serde(default)]
    pub binds: Vec<FieldBind>,
    #[serde(default)]
    pub interval_ms: u64,
}

struct EndpointRt {
    cfg: EndpointConfig,
    worker: Option<EndpointWorker>,
    connected: bool,
    state: String,
    last_error: String,
    last_frame: String,
    frame_count: u64,
}

struct PackState {
    host: HostRef,
    config: PackConfig,
    endpoints: BTreeMap<String, EndpointRt>,
    /// point name → last value (for emit assembly)
    point_values: BTreeMap<String, String>,
    last_emit: BTreeMap<String, Instant>,
}

impl PackState {
    fn new(host: HostRef) -> Self {
        Self {
            host,
            config: PackConfig::default(),
            endpoints: BTreeMap::new(),
            point_values: BTreeMap::new(),
            last_emit: BTreeMap::new(),
        }
    }

    fn log(&self, level: i32, msg: &str) {
        self.host.log(level, msg);
    }

    fn upsert_string(&self, name: &str, source: &str, value: &str) {
        self.host.upsert_text(name, source, value, PB_QUALITY_GOOD);
    }

    fn configure_json(&mut self, text: &str) -> Result<(), String> {
        let cfg: PackConfig = serde_json::from_str(text).map_err(|e| format!("json: {e}"))?;
        self.stop_all();
        self.config = cfg;
        self.endpoints.clear();
        for ep in &self.config.endpoints {
            let frame = self.frame_for_endpoint(&ep.id);
            let ecfg = EndpointConfig {
                id: ep.id.clone(),
                kind: ep.kind.clone(),
                serial_port: ep.serial_port.clone(),
                baud: ep.baud,
                data_bits: ep.data_bits,
                parity: ep.parity.clone(),
                stop_bits: ep.stop_bits,
                flow: ep.flow.clone(),
                host: ep.host.clone(),
                tcp_port: ep.tcp_port,
                reconnect: ep.reconnect,
                bind: ep.bind.clone(),
                listen_port: ep.listen_port,
                frame,
                auto_connect: ep.auto_connect,
            };
            self.endpoints.insert(
                ep.id.clone(),
                EndpointRt {
                    cfg: ecfg,
                    worker: None,
                    connected: false,
                    state: "disconnected".into(),
                    last_error: String::new(),
                    last_frame: String::new(),
                    frame_count: 0,
                },
            );
        }
        self.log(
            PB_LOG_INFO,
            &format!(
                "streams configured: {} endpoints, {} inputs, {} outputs",
                self.endpoints.len(),
                self.config.inputs.len(),
                self.config.outputs.len()
            ),
        );
        // Auto-connect endpoints flagged (default true).
        let ids: Vec<String> = self
            .endpoints
            .iter()
            .filter(|(_, rt)| rt.cfg.auto_connect)
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            let _ = self.start_endpoint(&id);
        }
        Ok(())
    }

    fn frame_for_endpoint(&self, endpoint_id: &str) -> FrameConfig {
        if let Some(inp) = self
            .config
            .inputs
            .iter()
            .find(|i| i.endpoint == endpoint_id)
        {
            return FrameConfig {
                mode: inp.frame.clone(),
                eol: inp.eol.clone(),
                eol_custom: inp.eol_custom.clone(),
                idle_ms: inp.idle_ms,
            };
        }
        if let Some(out) = self
            .config
            .outputs
            .iter()
            .find(|o| o.endpoint == endpoint_id)
        {
            return FrameConfig {
                mode: "delimited".into(),
                eol: out.eol.clone(),
                eol_custom: out.eol_custom.clone(),
                idle_ms: 50,
            };
        }
        FrameConfig::default()
    }

    fn stop_all(&mut self) {
        for rt in self.endpoints.values_mut() {
            if let Some(w) = rt.worker.take() {
                w.stop();
            }
            rt.connected = false;
            rt.state = "disconnected".into();
        }
    }

    fn start_endpoint(&mut self, id: &str) -> Result<(), String> {
        let Some(rt) = self.endpoints.get_mut(id) else {
            return Err(format!("unknown endpoint {id}"));
        };
        if let Some(w) = rt.worker.take() {
            w.stop();
        }
        match EndpointWorker::spawn(rt.cfg.clone()) {
            Ok(w) => {
                rt.worker = Some(w);
                rt.state = "starting".into();
                rt.last_error.clear();
                Ok(())
            }
            Err(e) => {
                rt.state = "error".into();
                rt.last_error = e.clone();
                Err(e)
            }
        }
    }

    fn poll_endpoints(&mut self) {
        let ids: Vec<String> = self.endpoints.keys().cloned().collect();
        for id in ids {
            let events = {
                let Some(rt) = self.endpoints.get(&id) else {
                    continue;
                };
                match &rt.worker {
                    Some(w) => w.poll_events(),
                    None => Vec::new(),
                }
            };
            for ev in events {
                self.handle_event(&id, ev);
            }
        }
        self.emit_intervals();
    }

    fn handle_event(&mut self, id: &str, ev: WorkerEvent) {
        match ev {
            WorkerEvent::Connected => {
                if let Some(rt) = self.endpoints.get_mut(id) {
                    rt.connected = true;
                    rt.state = "connected".into();
                    rt.last_error.clear();
                }
                self.log(PB_LOG_INFO, &format!("streams/{id} connected"));
            }
            WorkerEvent::Disconnected { detail } => {
                if let Some(rt) = self.endpoints.get_mut(id) {
                    rt.connected = false;
                    rt.state = "disconnected".into();
                    if detail != "stopped" {
                        rt.last_error = detail.clone();
                    }
                }
            }
            WorkerEvent::Error { detail } => {
                if let Some(rt) = self.endpoints.get_mut(id) {
                    rt.last_error = detail.clone();
                    rt.state = "error".into();
                }
                self.log(PB_LOG_ERROR, &format!("streams/{id}: {detail}"));
            }
            WorkerEvent::Frame { text } => {
                if let Some(rt) = self.endpoints.get_mut(id) {
                    rt.last_frame = text.clone();
                    rt.frame_count = rt.frame_count.saturating_add(1);
                }
                self.apply_input_frame(id, &text);
            }
        }
    }

    fn apply_input_frame(&mut self, endpoint_id: &str, line: &str) {
        let maps: Vec<InputMap> = self
            .config
            .inputs
            .iter()
            .filter(|i| i.endpoint == endpoint_id)
            .cloned()
            .collect();
        for map in maps {
            let fields = sample_fields(line, &map.parser, &map.fixed_widths, &map.regex);
            for bind in &map.binds {
                let value = if bind.field.is_empty() || map.parser == "whole_line" {
                    line.to_string()
                } else if let Ok(idx) = bind.field.parse::<usize>() {
                    fields.get(idx).cloned().unwrap_or_default()
                } else {
                    // named regex group not tracked separately — treat as miss
                    String::new()
                };
                let source = if bind.source.is_empty() {
                    if bind.field.is_empty() {
                        format!("stream/{endpoint_id}")
                    } else {
                        format!("stream/{endpoint_id}::{}", bind.field)
                    }
                } else {
                    bind.source.clone()
                };
                let name = if bind.point.is_empty() {
                    source.clone()
                } else {
                    bind.point.clone()
                };
                self.point_values.insert(name.clone(), value.clone());
                self.upsert_string(&name, &source, &value);
            }
        }
    }

    fn write_text(&mut self, name: &str, value: &str) {
        self.point_values.insert(name.to_string(), value.to_string());
        // Emit any output maps that include this point (on-change).
        let outs: Vec<OutputMap> = self.config.outputs.clone();
        for out in outs {
            if out.binds.iter().any(|b| b.point == name) {
                let _ = self.emit_output(&out);
            }
        }
    }

    fn emit_intervals(&mut self) {
        let outs: Vec<OutputMap> = self
            .config
            .outputs
            .iter()
            .filter(|o| o.interval_ms > 0)
            .cloned()
            .collect();
        for out in outs {
            let key = out.endpoint.clone();
            let due = match self.last_emit.get(&key) {
                Some(t) => t.elapsed() >= Duration::from_millis(out.interval_ms),
                None => true,
            };
            if due
                && self.emit_output(&out).is_ok() {
                    self.last_emit.insert(key, Instant::now());
                }
        }
    }

    fn emit_output(&mut self, out: &OutputMap) -> Result<(), String> {
        let mut slots: Vec<(usize, String)> = out
            .binds
            .iter()
            .map(|b| {
                let idx = b.field.parse::<usize>().unwrap_or(0);
                let val = self
                    .point_values
                    .get(&b.point)
                    .cloned()
                    .unwrap_or_default();
                (idx, val)
            })
            .collect();
        if slots.is_empty() {
            return Ok(());
        }
        slots.sort_by_key(|(i, _)| *i);
        let max = slots.last().map(|(i, _)| *i).unwrap_or(0);
        let mut values = vec![String::new(); max + 1];
        for (i, v) in slots {
            if i < values.len() {
                values[i] = v;
            }
        }
        let mut line = join_fields(&out.layout, &values);
        line.push_str(&String::from_utf8_lossy(&trailer_bytes(
            &out.eol,
            &out.eol_custom,
        )));
        let Some(rt) = self.endpoints.get(&out.endpoint) else {
            return Err(format!("no endpoint {}", out.endpoint));
        };
        let Some(w) = rt.worker.as_ref() else {
            return Err("endpoint not running".into());
        };
        w.write_bytes(line.as_bytes())
    }

    fn detail_json(&self) -> String {
        let mut endpoints = Vec::new();
        for (id, rt) in &self.endpoints {
            endpoints.push(serde_json::json!({
                "id": id,
                "kind": rt.cfg.kind,
                "connected": rt.connected,
                "state": rt.state,
                "last_error": rt.last_error,
                "last_frame": truncate(&rt.last_frame, 200),
                "frame_count": rt.frame_count,
            }));
        }
        serde_json::json!({ "endpoints": endpoints }).to_string()
    }
}

fn truncate(s: &str, max: usize) -> String {
    let mut chars = s.chars();
    let taken: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        format!("{taken}…")
    } else {
        taken
    }
}

impl Pack for PackState {
    const ID: &'static [u8] = PACK_ID;
    const TITLE: &'static [u8] = PACK_TITLE;

    fn new(host: HostRef) -> Self {
        PackState::new(host)
    }

    fn configure(&mut self, json: &str) -> Result<(), String> {
        self.configure_json(json)
    }

    fn config_failed(&mut self, err: String) {
        self.log(PB_LOG_ERROR, &err);
    }

    /// Start every configured endpoint; `-1` when any failed (the error is logged per endpoint).
    fn connect(&mut self) -> c_int {
        let ids: Vec<String> = self.endpoints.keys().cloned().collect();
        let mut ok = true;
        for id in ids {
            if let Err(e) = self.start_endpoint(&id) {
                self.log(PB_LOG_ERROR, &e);
                ok = false;
            }
        }
        if ok { 0 } else { -1 }
    }

    fn disconnect(&mut self) -> c_int {
        self.stop_all();
        0
    }

    fn shutdown(&mut self) {
        self.stop_all();
    }

    fn write_point(&mut self, name: &str, _kind: i32, bytes: &[u8]) -> c_int {
        self.write_text(name, &value_text(bytes));
        0
    }

    fn poll(&mut self) {
        self.poll_endpoints();
    }

    fn state_label(&self) -> String {
        let total = self.endpoints.len();
        if total == 0 {
            return "idle".into();
        }
        format!("{}/{total} up", self.endpoints.values().filter(|e| e.connected).count())
    }

    fn last_error(&self) -> String {
        self.endpoints.values().find(|e| !e.last_error.is_empty()).map(|e| e.last_error.clone()).unwrap_or_default()
    }

    fn is_connected(&self) -> bool {
        self.endpoints.values().any(|e| e.connected)
    }

    /// Per-endpoint status for the UI (rides the `topics_json` slot of the ABI).
    fn detail_json(&self) -> String {
        PackState::detail_json(self)
    }
}

pb_abi::export_pack!(PackState);

#[cfg(test)]
mod tests {
    use super::*;
    use pb_abi::PB_ABI_VERSION;

    #[test]
    fn register_ok() {
        let mut api = unsafe { std::mem::zeroed() };
        let rc = unsafe { pb_pack_register(&mut api) };
        assert_eq!(rc, 0);
        assert_eq!(api.abi_version, PB_ABI_VERSION);
    }
}
