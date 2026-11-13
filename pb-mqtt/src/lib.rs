//! MQTT client protocol pack (cdylib) for Protocol Bench.
//!
//! Worker + EventLoop pattern ported from TopicBench `mqtt/live.rs` (no UI/tree).
//! Client roles only — embedded broker / hub is deferred.

mod json_path;
mod session;

pub use json_path::{
    extract_path_value, point_id_for_field, point_id_for_topic, split_point_id, topic_matches,
};

use pb_abi::{
    write_cstr, PbHostApi, PbPackApi, PB_ABI_VERSION, PB_LOG_ERROR, PB_LOG_INFO, PB_QUALITY_BAD,
    PB_QUALITY_GOOD, PB_VALUE_BYTES, PB_VALUE_STRING,
};
use serde::{Deserialize, Serialize};
use session::{MqttSession, SessionEvent, TransportKind};
use std::ffi::CStr;
use std::os::raw::{c_char, c_int, c_void};
use std::sync::mpsc::TryRecvError;

const PACK_ID: &[u8] = b"mqtt\0";
const PACK_TITLE: &[u8] = b"MQTT\0";

/// One JSON field → point rule (topic filter + dotted path).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ParseRule {
    /// MQTT topic filter (`+` / `#` allowed). Empty = match all topics.
    #[serde(default)]
    pub topic: String,
    /// Dotted / `$.a.b` / `arr[0].x` path into a JSON object payload.
    pub path: String,
    /// Optional override point id; default `mqtt/<topic>::<path>`.
    #[serde(default)]
    pub point: String,
}

/// JSON config accepted by [`configure`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MqttConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_client_id")]
    pub client_id: String,
    #[serde(default)]
    pub username: String,
    /// Password lives in config for this cut (no OS keyring).
    #[serde(default)]
    pub password: String,
    /// `tcp` | `tls` | `ws` | `wss`
    #[serde(default = "default_transport")]
    pub transport: String,
    #[serde(default = "default_keep_alive")]
    pub keep_alive_secs: u64,
    /// JSON object field paths → host points (applied on every matching inbound).
    #[serde(default)]
    pub parse_rules: Vec<ParseRule>,
}

fn default_host() -> String {
    "127.0.0.1".into()
}
fn default_port() -> u16 {
    1883
}
fn default_client_id() -> String {
    "protocol-bench".into()
}
fn default_transport() -> String {
    "tcp".into()
}
fn default_keep_alive() -> u64 {
    30
}

impl Default for MqttConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            client_id: default_client_id(),
            username: String::new(),
            password: String::new(),
            transport: default_transport(),
            keep_alive_secs: default_keep_alive(),
            parse_rules: Vec::new(),
        }
    }
}

impl MqttConfig {
    fn transport_kind(&self) -> Result<TransportKind, String> {
        match self.transport.to_ascii_lowercase().as_str() {
            "tcp" | "mqtt" => Ok(TransportKind::Tcp),
            "tls" | "mqtts" => Ok(TransportKind::Tls),
            "ws" | "websocket" => Ok(TransportKind::Ws),
            "wss" => Ok(TransportKind::Wss),
            other => Err(format!("unknown transport '{other}'")),
        }
    }
}

struct PackState {
    host: PbHostApi,
    config: MqttConfig,
    session: Option<MqttSession>,
    topics: Vec<String>,
    connected: bool,
    state: String,
    last_error: String,
    quality_bad: bool,
}

impl PackState {
    fn new(host: PbHostApi) -> Self {
        Self {
            host,
            config: MqttConfig::default(),
            session: None,
            topics: Vec::new(),
            connected: false,
            state: "disconnected".into(),
            last_error: String::new(),
            quality_bad: false,
        }
    }

    fn log(&self, level: i32, msg: &str) {
        if let Some(log) = self.host.log_message {
            if let Ok(c) = std::ffi::CString::new(msg) {
                unsafe { log(self.host.ctx, level, c.as_ptr()) };
            }
        }
    }

    fn upsert_string(&self, name: &str, value: &str, good: bool) {
        let Some(upsert) = self.host.upsert_point else {
            return;
        };
        let Ok(c_name) = std::ffi::CString::new(name) else {
            return;
        };
        let Ok(c_src) = std::ffi::CString::new("mqtt") else {
            return;
        };
        let bytes = value.as_bytes();
        unsafe {
            upsert(
                self.host.ctx,
                c_name.as_ptr(),
                PB_VALUE_STRING,
                bytes.as_ptr() as *const c_void,
                bytes.len(),
                if good {
                    PB_QUALITY_GOOD
                } else {
                    PB_QUALITY_BAD
                },
                c_src.as_ptr(),
            );
        }
    }

    fn upsert_bytes(&self, name: &str, value: &[u8], good: bool) {
        let Some(upsert) = self.host.upsert_point else {
            return;
        };
        let Ok(c_name) = std::ffi::CString::new(name) else {
            return;
        };
        let Ok(c_src) = std::ffi::CString::new("mqtt") else {
            return;
        };
        unsafe {
            upsert(
                self.host.ctx,
                c_name.as_ptr(),
                PB_VALUE_BYTES,
                value.as_ptr() as *const c_void,
                value.len(),
                if good {
                    PB_QUALITY_GOOD
                } else {
                    PB_QUALITY_BAD
                },
                c_src.as_ptr(),
            );
        }
    }

    fn apply_event(&mut self, ev: SessionEvent) {
        match ev {
            SessionEvent::State { connected, detail } => {
                self.connected = connected;
                self.state = if connected {
                    "connected".into()
                } else if detail.to_ascii_lowercase().contains("connecting") {
                    "connecting".into()
                } else {
                    "disconnected".into()
                };
                if connected {
                    self.quality_bad = false;
                    self.last_error.clear();
                    // Resubscribe after reconnect.
                    let topics = self.topics.clone();
                    if let Some(session) = self.session.as_ref() {
                        for topic in &topics {
                            let _ = session.subscribe(topic, 0);
                        }
                    }
                } else if !detail.is_empty()
                    && !detail.to_ascii_lowercase().contains("connecting")
                    && !detail.to_ascii_lowercase().contains("disconnected by user")
                {
                    self.last_error = detail.clone();
                    self.quality_bad = true;
                }
                self.log(PB_LOG_INFO, &detail);
            }
            SessionEvent::Message { topic, payload } => {
                // Whole-topic point on the host bus (channel + concrete topic).
                let point_name = point_id_for_topic(&topic);
                let text = match std::str::from_utf8(&payload) {
                    Ok(text) => {
                        self.upsert_string(&point_name, text, !self.quality_bad);
                        Some(text.to_string())
                    }
                    Err(_) => {
                        self.upsert_bytes(&point_name, &payload, !self.quality_bad);
                        None
                    }
                };
                if let Some(text) = text.as_deref() {
                    self.apply_parse_rules(&topic, text);
                }
            }
            SessionEvent::Error { detail } => {
                self.last_error = detail.clone();
                self.quality_bad = true;
                self.connected = false;
                self.state = "disconnected".into();
                self.log(PB_LOG_ERROR, &detail);
            }
        }
    }

    fn apply_parse_rules(&self, topic: &str, payload: &str) {
        let good = !self.quality_bad;
        for rule in &self.config.parse_rules {
            let path = rule.path.trim();
            if path.is_empty() {
                continue;
            }
            if !topic_matches(&rule.topic, topic) {
                continue;
            }
            let Some(value) = extract_path_value(payload, path) else {
                continue;
            };
            let name = if rule.point.trim().is_empty() {
                point_id_for_field(topic, path)
            } else {
                rule.point.trim().to_string()
            };
            self.upsert_string(&name, &value, good);
        }
    }

    fn drain(&mut self) {
        loop {
            let ev = match self.session.as_ref() {
                Some(s) => match s.try_recv() {
                    Ok(ev) => ev,
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        self.session = None;
                        self.connected = false;
                        self.state = "disconnected".into();
                        break;
                    }
                },
                None => break,
            };
            self.apply_event(ev);
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn pb_pack_abi_version() -> u32 {
    PB_ABI_VERSION
}

#[no_mangle]
pub unsafe extern "C" fn pb_pack_register(out: *mut PbPackApi) -> c_int {
    if out.is_null() {
        return -1;
    }
    *out = PbPackApi {
        abi_version: PB_ABI_VERSION,
        id: PACK_ID.as_ptr() as *const c_char,
        title: PACK_TITLE.as_ptr() as *const c_char,
        create: Some(create),
        destroy: Some(destroy),
        configure: Some(configure),
        connect: Some(connect),
        disconnect: Some(disconnect),
        subscribe: Some(subscribe),
        publish: Some(publish),
        write_point: Some(write_point),
        poll: Some(poll),
        state_label: Some(state_label),
        last_error: Some(last_error),
        is_connected: Some(is_connected),
        topics_json: Some(topics_json),
    };
    0
}

unsafe extern "C" fn create(host: *const PbHostApi) -> *mut c_void {
    if host.is_null() {
        return std::ptr::null_mut();
    }
    let host = *host;
    if host.abi_version != PB_ABI_VERSION {
        return std::ptr::null_mut();
    }
    let state = Box::new(PackState::new(host));
    Box::into_raw(state) as *mut c_void
}

unsafe extern "C" fn destroy(handle: *mut c_void) {
    if handle.is_null() {
        return;
    }
    let mut state = Box::from_raw(handle as *mut PackState);
    if let Some(session) = state.session.take() {
        session.stop();
    }
}

unsafe extern "C" fn configure(handle: *mut c_void, json: *const c_char, len: usize) -> c_int {
    if handle.is_null() || json.is_null() {
        return -1;
    }
    let state = &mut *(handle as *mut PackState);
    let slice = std::slice::from_raw_parts(json as *const u8, len);
    let text = match std::str::from_utf8(slice) {
        Ok(t) => t,
        Err(_) => {
            state.last_error = "configure: invalid utf-8".into();
            return -1;
        }
    };
    match serde_json::from_str::<MqttConfig>(text) {
        Ok(cfg) => {
            if let Err(e) = cfg.transport_kind() {
                state.last_error = e;
                return -1;
            }
            state.config = cfg;
            0
        }
        Err(e) => {
            state.last_error = format!("configure: {e}");
            -1
        }
    }
}

unsafe extern "C" fn connect(handle: *mut c_void) -> c_int {
    if handle.is_null() {
        return -1;
    }
    let state = &mut *(handle as *mut PackState);
    if let Some(prev) = state.session.take() {
        prev.stop();
    }
    let transport = match state.config.transport_kind() {
        Ok(t) => t,
        Err(e) => {
            state.last_error = e;
            return -1;
        }
    };
    match MqttSession::spawn(
        state.config.host.clone(),
        state.config.port,
        state.config.client_id.clone(),
        state.config.username.clone(),
        state.config.password.clone(),
        transport,
        state.config.keep_alive_secs,
    ) {
        Ok(session) => {
            state.session = Some(session);
            state.state = "connecting".into();
            state.last_error.clear();
            state.connected = false;
            0
        }
        Err(e) => {
            state.last_error = e;
            state.quality_bad = true;
            -1
        }
    }
}

unsafe extern "C" fn disconnect(handle: *mut c_void) -> c_int {
    if handle.is_null() {
        return -1;
    }
    let state = &mut *(handle as *mut PackState);
    if let Some(session) = state.session.take() {
        session.stop();
    }
    state.connected = false;
    state.state = "disconnected".into();
    0
}

unsafe extern "C" fn subscribe(handle: *mut c_void, topic: *const c_char, qos: u8) -> c_int {
    if handle.is_null() || topic.is_null() {
        return -1;
    }
    let state = &mut *(handle as *mut PackState);
    let topic = match CStr::from_ptr(topic).to_str() {
        Ok(t) => t.to_string(),
        Err(_) => {
            state.last_error = "subscribe: bad topic".into();
            return -1;
        }
    };
    if !state.topics.iter().any(|t| t == &topic) {
        state.topics.push(topic.clone());
    }
    let Some(session) = state.session.as_ref() else {
        // Remember topic; applied on connect / reconnect.
        return 0;
    };
    match session.subscribe(&topic, qos) {
        Ok(()) => 0,
        Err(e) => {
            state.last_error = e;
            -1
        }
    }
}

unsafe extern "C" fn publish(
    handle: *mut c_void,
    topic: *const c_char,
    payload: *const u8,
    len: usize,
    qos: u8,
    retain: c_int,
) -> c_int {
    if handle.is_null() || topic.is_null() {
        return -1;
    }
    let state = &mut *(handle as *mut PackState);
    let topic = match CStr::from_ptr(topic).to_str() {
        Ok(t) => t,
        Err(_) => {
            state.last_error = "publish: bad topic".into();
            return -1;
        }
    };
    let bytes = if payload.is_null() || len == 0 {
        &[][..]
    } else {
        std::slice::from_raw_parts(payload, len)
    };
    let Some(session) = state.session.as_ref() else {
        state.last_error = "publish: not connected".into();
        return -1;
    };
    match session.publish(topic, bytes, qos, retain != 0) {
        Ok(()) => 0,
        Err(e) => {
            state.last_error = e;
            -1
        }
    }
}

/// Point write → publish. Point names `mqtt/<topic>` (or raw topic) publish UTF-8 payload.
unsafe extern "C" fn write_point(
    handle: *mut c_void,
    name: *const c_char,
    value_kind: i32,
    value: *const c_void,
    value_len: usize,
) -> c_int {
    if handle.is_null() || name.is_null() {
        return -1;
    }
    let state = &mut *(handle as *mut PackState);
    let name = match CStr::from_ptr(name).to_str() {
        Ok(t) => t,
        Err(_) => return -1,
    };
    let (topic, _) = split_point_id(name);
    let payload: Vec<u8> = match value_kind {
        PB_VALUE_STRING | PB_VALUE_BYTES => {
            if value.is_null() {
                Vec::new()
            } else {
                std::slice::from_raw_parts(value as *const u8, value_len).to_vec()
            }
        }
        _ => {
            state.last_error = "write_point: unsupported value kind".into();
            return -1;
        }
    };
    let Some(session) = state.session.as_ref() else {
        state.last_error = "write_point: not connected".into();
        return -1;
    };
    match session.publish(topic, &payload, 0, false) {
        Ok(()) => 0,
        Err(e) => {
            state.last_error = e;
            -1
        }
    }
}

unsafe extern "C" fn poll(handle: *mut c_void) {
    if handle.is_null() {
        return;
    }
    let state = &mut *(handle as *mut PackState);
    state.drain();
}

unsafe extern "C" fn state_label(handle: *mut c_void, out: *mut c_char, out_len: usize) -> c_int {
    if handle.is_null() {
        return -1;
    }
    let state = &*(handle as *mut PackState);
    write_cstr(out, out_len, &state.state)
}

unsafe extern "C" fn last_error(handle: *mut c_void, out: *mut c_char, out_len: usize) -> c_int {
    if handle.is_null() {
        return -1;
    }
    let state = &*(handle as *mut PackState);
    write_cstr(out, out_len, &state.last_error)
}

unsafe extern "C" fn is_connected(handle: *mut c_void) -> c_int {
    if handle.is_null() {
        return 0;
    }
    let state = &*(handle as *mut PackState);
    if state.connected {
        1
    } else {
        0
    }
}

unsafe extern "C" fn topics_json(handle: *mut c_void, out: *mut c_char, out_len: usize) -> c_int {
    if handle.is_null() {
        return -1;
    }
    let state = &*(handle as *mut PackState);
    let json = serde_json::to_string(&state.topics).unwrap_or_else(|_| "[]".into());
    write_cstr(out, out_len, &json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_parses_defaults() {
        let cfg: MqttConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(cfg.host, "127.0.0.1");
        assert_eq!(cfg.port, 1883);
        assert!(cfg.transport_kind().is_ok());
    }

    #[test]
    fn register_fills_api() {
        let mut api = unsafe { std::mem::zeroed::<PbPackApi>() };
        let rc = unsafe { pb_pack_register(&mut api) };
        assert_eq!(rc, 0);
        assert_eq!(api.abi_version, PB_ABI_VERSION);
        assert!(!api.id.is_null());
        assert!(api.create.is_some());
    }
}
