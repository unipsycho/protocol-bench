//! JSON-RPC-ish methods over length-prefixed frames.

use pb_engine::{AggConfig, CalcConfig, EngineCommand, EngineId, HostSnapshot};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Response {
    pub fn ok(id: u64, result: serde_json::Value) -> Self {
        Self {
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: u64, msg: impl Into<String>) -> Self {
        Self {
            id,
            result: None,
            error: Some(msg.into()),
        }
    }
}

/// What the service reports as “running” (Decision #8).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ServiceStatus {
    pub connected_ok: bool,
    pub project_path: String,
    pub packs_loaded: Vec<String>,
    pub mqtt_hub_running: bool,
    pub opcua_hub_running: bool,
    pub point_count: usize,
    pub uptime_ms: u64,
    pub last_error: String,
    pub slots: Vec<SlotStatus>,
}

impl ServiceStatus {
    /// Human-readable list of what is live right now (hubs listening, clients/endpoints up).
    /// Drives the "this will stop" confirmations and the service tray tooltip.
    pub fn running_hubs(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.mqtt_hub_running {
            out.push("MQTT broker (embedded hub)".to_string());
        }
        if self.opcua_hub_running {
            out.push("OPC UA server (embedded hub)".to_string());
        }
        for s in &self.slots {
            if !s.pack_loaded || !s.connected {
                continue;
            }
            let name = match s.id.as_str() {
                "mqtt" => "MQTT",
                "opcua" => "OPC UA",
                "streams" => "Streams / TCP",
                "modbus" => "Modbus",
                "filefollow" => "File follow",
                "http" => "HTTP / REST",
                "snmp" => "SNMP",
                "las" => "LAS",
                "wits" => "WITS",
                "websocket" => "WebSocket",
                "sparkplug" => "Sparkplug B",
                "dnp3" => "DNP3",
                "can" => "CAN",
                other => other,
            };
            out.push(format!("{name}: {}", s.state));
        }
        out
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotStatus {
    pub id: String,
    pub state: String,
    pub connected: bool,
    pub pack_loaded: bool,
    pub last_error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelloParams {
    pub token: String,
    pub client: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitParams {
    pub engine: EngineId,
    pub command: EngineCommand,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetCalcsParams {
    pub calcs: Vec<CalcConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetAggsParams {
    pub aggs: Vec<AggConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenProjectParams {
    /// Empty → service default project.json.
    #[serde(default)]
    pub path: String,
    /// App Settings auto-run hubs checkbox (Decision #2).
    #[serde(default)]
    pub auto_run_hubs: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PutProjectParams {
    /// Full project.json document as JSON value.
    pub project: serde_json::Value,
    #[serde(default)]
    pub auto_run_hubs: bool,
    /// When set, retarget the service project path before writing (Save as).
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportBindingsParams {
    pub points: Vec<PointBindingWire>,
    pub next_point_seq: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PointBindingWire {
    pub id: String,
    pub source: String,
}

pub const METHOD_HELLO: &str = "hello";
pub const METHOD_STATUS: &str = "status";
pub const METHOD_SNAPSHOT: &str = "snapshot";
pub const METHOD_SUBMIT: &str = "submit";
pub const METHOD_SET_CALCS: &str = "set_calcs";
pub const METHOD_SET_AGGS: &str = "set_aggs";
pub const METHOD_GET_PROJECT: &str = "get_project";
pub const METHOD_PUT_PROJECT: &str = "put_project";
pub const METHOD_OPEN_PROJECT: &str = "open_project";
pub const METHOD_IMPORT_BINDINGS: &str = "import_bindings";
pub const METHOD_PING: &str = "ping";
/// Stop every embedded hub and disconnect every client (project file is left as is).
pub const METHOD_STOP_ALL: &str = "stop_all";

/// Rebuild a wire snapshot for clients (engine HostSnapshot is authoritative).
pub fn status_from_host(host: &HostSnapshot, project_path: &str, last_error: &str) -> ServiceStatus {
    let mqtt = host.slot(EngineId::Mqtt);
    let opcua = host.slot(EngineId::OpcUa);
    let mqtt_hub_running = mqtt
        .map(|s| s.detail_json.contains("\"broker_running\":true") || s.state.contains("broker"))
        .unwrap_or(false)
        || mqtt.map(|s| s.state == "broker_running" || s.connected).unwrap_or(false);
    // Packs report hub listen in detail_json / state — UI also tracks panel flags.
    let opcua_hub_running = opcua
        .map(|s| s.detail_json.contains("\"server_running\":true") || s.state.contains("server"))
        .unwrap_or(false);
    ServiceStatus {
        connected_ok: host.alive,
        project_path: project_path.into(),
        packs_loaded: host.packs_loaded.clone(),
        mqtt_hub_running,
        opcua_hub_running,
        point_count: host.point_count,
        uptime_ms: host.uptime_ms,
        last_error: last_error.into(),
        slots: host
            .slots
            .iter()
            .map(|s| SlotStatus {
                id: format!("{:?}", s.id).to_ascii_lowercase(),
                state: s.state.clone(),
                connected: s.connected,
                pack_loaded: s.pack_loaded,
                last_error: s.last_error.clone(),
            })
            .collect(),
    }
}

#[cfg(test)]
mod running_hubs_tests {
    use super::*;

    fn status(mqtt: bool, opc: bool, slots: Vec<SlotStatus>) -> ServiceStatus {
        ServiceStatus {
            connected_ok: true,
            project_path: String::new(),
            packs_loaded: vec![],
            mqtt_hub_running: mqtt,
            opcua_hub_running: opc,
            point_count: 0,
            uptime_ms: 0,
            last_error: String::new(),
            slots,
        }
    }

    #[test]
    fn idle_service_lists_nothing() {
        assert!(status(false, false, vec![]).running_hubs().is_empty());
    }

    #[test]
    fn lists_hubs_and_connected_slots_only() {
        let slot = |id: &str, connected: bool| SlotStatus {
            id: id.into(),
            state: "up".into(),
            connected,
            pack_loaded: true,
            last_error: String::new(),
        };
        let got = status(true, true, vec![slot("modbus", true), slot("snmp", false)]).running_hubs();
        assert_eq!(got.len(), 3);
        assert!(got[0].contains("MQTT broker"));
        assert!(got[1].contains("OPC UA server"));
        assert!(got[2].starts_with("Modbus"));
    }
}
