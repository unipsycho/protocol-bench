//! Wrap a loaded C-ABI pack as a [`ProtocolEngine`].

use crate::{EngineCommand, EngineId, LoadedPack, ProtocolEngine, SlotSnapshot};
use pb_abi::{
    write_cstr, PB_LOG_ERROR, PB_LOG_INFO, PB_LOG_WARN, PB_QUALITY_BAD, PB_QUALITY_GOOD,
    PB_QUALITY_UNCERTAIN, PB_VALUE_BOOL, PB_VALUE_BYTES, PB_VALUE_F64, PB_VALUE_I64,
    PB_VALUE_STRING,
};
use pb_points::{PointStore, PointValue, Quality};
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::sync::{Arc, Mutex};

/// Context passed to pack host callbacks.
pub struct HostBridge {
    pub points: Arc<Mutex<PointStore>>,
}

pub struct PackEngine {
    id: EngineId,
    ticks: u64,
    commands_handled: u64,
    handle: *mut c_void,
    pack: LoadedPack,
    last_error: String,
}

// Pack + host callbacks are designed for single-threaded host use.
unsafe impl Send for PackEngine {}

impl PackEngine {
    pub fn from_loaded(id: EngineId, pack: LoadedPack) -> Result<Self, String> {
        let create = pack
            .api
            .create
            .ok_or_else(|| "pack missing create".to_string())?;
        let handle = unsafe { create(pack.host.as_ref()) };
        if handle.is_null() {
            return Err("pack create returned null".into());
        }
        Ok(Self {
            id,
            ticks: 0,
            commands_handled: 0,
            handle,
            pack,
            last_error: String::new(),
        })
    }

    fn set_err(&mut self, msg: impl Into<String>) {
        self.last_error = msg.into();
    }

    fn call_i(&mut self, label: &str, rc: c_int) {
        if rc != 0 {
            self.set_err(format!("{label} failed ({rc})"));
        } else if label != "poll" {
            // Clear sticky error on successful mutating calls.
            self.last_error.clear();
        }
    }

    fn read_cstr(
        &self,
        f: Option<unsafe extern "C" fn(*mut c_void, *mut c_char, usize) -> c_int>,
    ) -> String {
        let Some(f) = f else {
            return String::new();
        };
        // Grow until the pack fits (wildcard `#` traffic snapshots exceed 512).
        // write_cstr truncates to buf_len-1; n == capacity-1 means "maybe truncated".
        let mut size = 4 * 1024;
        loop {
            let mut buf = vec![0u8; size];
            let n = unsafe { f(self.handle, buf.as_mut_ptr() as *mut c_char, buf.len()) };
            if n < 0 {
                return String::new();
            }
            let end = (n as usize).min(buf.len().saturating_sub(1));
            let truncated = end >= size.saturating_sub(1);
            if truncated && size < 1024 * 1024 {
                size *= 2;
                continue;
            }
            return String::from_utf8_lossy(&buf[..end]).into_owned();
        }
    }
}

impl Drop for PackEngine {
    fn drop(&mut self) {
        if let Some(destroy) = self.pack.api.destroy {
            unsafe { destroy(self.handle) };
        }
        self.handle = std::ptr::null_mut();
    }
}

impl ProtocolEngine for PackEngine {
    fn id(&self) -> EngineId {
        self.id
    }

    fn poll(&mut self) {
        self.ticks = self.ticks.saturating_add(1);
        if let Some(poll) = self.pack.api.poll {
            unsafe { poll(self.handle) };
        }
    }

    fn handle(&mut self, command: EngineCommand) {
        self.commands_handled = self.commands_handled.saturating_add(1);
        match command {
            EngineCommand::Acknowledge => {}
            EngineCommand::Configure { json } => {
                if let Some(configure) = self.pack.api.configure {
                    let rc = unsafe {
                        configure(self.handle, json.as_ptr() as *const c_char, json.len())
                    };
                    self.call_i("configure", rc);
                }
            }
            EngineCommand::Connect => {
                if let Some(connect) = self.pack.api.connect {
                    let rc = unsafe { connect(self.handle) };
                    self.call_i("connect", rc);
                }
            }
            EngineCommand::Disconnect => {
                if let Some(disconnect) = self.pack.api.disconnect {
                    let rc = unsafe { disconnect(self.handle) };
                    self.call_i("disconnect", rc);
                }
            }
            EngineCommand::Subscribe { topic, qos } => {
                if let Some(subscribe) = self.pack.api.subscribe {
                    match CString::new(topic) {
                        Ok(c) => {
                            let rc = unsafe { subscribe(self.handle, c.as_ptr(), qos) };
                            self.call_i("subscribe", rc);
                        }
                        Err(_) => self.set_err("subscribe: topic contains NUL"),
                    }
                }
            }
            EngineCommand::Publish {
                topic,
                payload,
                qos,
                retain,
            } => {
                if let Some(publish) = self.pack.api.publish {
                    match CString::new(topic) {
                        Ok(c) => {
                            let rc = unsafe {
                                publish(
                                    self.handle,
                                    c.as_ptr(),
                                    payload.as_ptr(),
                                    payload.len(),
                                    qos,
                                    if retain { 1 } else { 0 },
                                )
                            };
                            self.call_i("publish", rc);
                        }
                        Err(_) => self.set_err("publish: topic contains NUL"),
                    }
                }
            }
            EngineCommand::WritePoint { name, value } => {
                if let Some(write_point) = self.pack.api.write_point {
                    match CString::new(name) {
                        Ok(c) => {
                            let bytes = value.as_bytes();
                            let rc = unsafe {
                                write_point(
                                    self.handle,
                                    c.as_ptr(),
                                    PB_VALUE_STRING,
                                    bytes.as_ptr() as *const c_void,
                                    bytes.len(),
                                )
                            };
                            self.call_i("write_point", rc);
                        }
                        Err(_) => self.set_err("write_point: name contains NUL"),
                    }
                }
            }
        }
    }

    fn snapshot(&self) -> SlotSnapshot {
        let state = self.read_cstr(self.pack.api.state_label);
        let pack_err = self.read_cstr(self.pack.api.last_error);
        let last_error = if !self.last_error.is_empty() {
            self.last_error.clone()
        } else {
            pack_err
        };
        let connected = self
            .pack
            .api
            .is_connected
            .map(|f| unsafe { f(self.handle) } != 0)
            .unwrap_or(false);
        let detail_json = self.read_cstr(self.pack.api.topics_json);
        let topics = mqtt_topics_from_detail(&detail_json);
        let state = if state.is_empty() {
            if connected {
                "connected".into()
            } else {
                "disconnected".into()
            }
        } else {
            state
        };
        SlotSnapshot {
            id: self.id,
            title: self.id.title(),
            state,
            note: self.id.note().into(),
            ticks: self.ticks,
            commands_handled: self.commands_handled,
            connected,
            last_error,
            topics,
            detail_json,
            pack_loaded: true,
        }
    }
}

/// Pull `client/topic` rows from an MQTT clients snapshot JSON array.
fn mqtt_topics_from_detail(raw: &str) -> Vec<String> {
    let t = raw.trim();
    if t.is_empty() || t == "[]" {
        return Vec::new();
    }
    // Lightweight extract: look for "topics":["a","b"] blocks under each client.
    let mut out = Vec::new();
    let mut rest = t;
    while let Some(idx) = rest.find("\"id\"") {
        rest = &rest[idx + 4..];
        let id = json_string_after(rest).unwrap_or_default();
        if let Some(tidx) = rest.find("\"topics\"") {
            let after = &rest[tidx + 8..];
            if let Some(bracket) = after.find('[') {
                let mut arr = &after[bracket + 1..];
                while let Some(topic) = next_json_string(&mut arr) {
                    if id.is_empty() {
                        out.push(topic);
                    } else {
                        out.push(format!("{id}/{topic}"));
                    }
                }
            }
        }
    }
    out
}

fn json_string_after(s: &str) -> Option<String> {
    let mut rest = s.trim_start_matches(|c: char| c != '"');
    if !rest.starts_with('"') {
        return None;
    }
    rest = &rest[1..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn next_json_string(rest: &mut &str) -> Option<String> {
    *rest = rest.trim_start_matches(|c: char| c == ',' || c.is_whitespace() || c == ']');
    if rest.is_empty() || rest.starts_with(']') {
        return None;
    }
    if !rest.starts_with('"') {
        return None;
    }
    *rest = &rest[1..];
    let end = rest.find('"')?;
    let val = rest[..end].to_string();
    *rest = &rest[end + 1..];
    Some(val)
}

pub unsafe extern "C" fn host_upsert_point(
    ctx: *mut c_void,
    name: *const c_char,
    value_kind: i32,
    value: *const c_void,
    value_len: usize,
    quality: i32,
    source: *const c_char,
) {
    if ctx.is_null() || name.is_null() {
        return;
    }
    let bridge = &*(ctx as *const HostBridge);
    let name = match CStr::from_ptr(name).to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return,
    };
    let source = if source.is_null() {
        String::new()
    } else {
        CStr::from_ptr(source).to_string_lossy().into_owned()
    };
    let point_value = match value_kind {
        PB_VALUE_BOOL => {
            if value.is_null() || value_len < 1 {
                return;
            }
            PointValue::Bool(*(value as *const u8) != 0)
        }
        PB_VALUE_I64 => {
            if value.is_null() || value_len < 8 {
                return;
            }
            PointValue::I64(*(value as *const i64))
        }
        PB_VALUE_F64 => {
            if value.is_null() || value_len < 8 {
                return;
            }
            PointValue::F64(*(value as *const f64))
        }
        PB_VALUE_STRING => {
            if value.is_null() {
                PointValue::String(String::new())
            } else {
                let slice = std::slice::from_raw_parts(value as *const u8, value_len);
                PointValue::String(String::from_utf8_lossy(slice).into_owned())
            }
        }
        PB_VALUE_BYTES => {
            if value.is_null() {
                PointValue::Bytes(Vec::new())
            } else {
                let slice = std::slice::from_raw_parts(value as *const u8, value_len);
                PointValue::Bytes(slice.to_vec())
            }
        }
        _ => return,
    };
    let q = match quality {
        PB_QUALITY_GOOD => Quality::Good,
        PB_QUALITY_BAD => Quality::Bad,
        PB_QUALITY_UNCERTAIN => Quality::Uncertain,
        _ => Quality::Uncertain,
    };
    if let Ok(mut store) = bridge.points.lock() {
        // Packs may pass a long address as `name`; host owns short `XXXX_yyyy` ids.
        store.upsert_from_pack(&name, point_value, q, &source);
    }
}

pub unsafe extern "C" fn host_log_message(ctx: *mut c_void, level: i32, message: *const c_char) {
    let _ = ctx;
    if message.is_null() {
        return;
    }
    let msg = CStr::from_ptr(message).to_string_lossy();
    let tag = match level {
        PB_LOG_INFO => "info",
        PB_LOG_WARN => "warn",
        PB_LOG_ERROR => "error",
        _ => "debug",
    };
    eprintln!("pb-pack[{tag}]: {msg}");
}

#[allow(dead_code)]
fn _keep_write_cstr_linked() {
    let mut buf = [0i8; 4];
    unsafe {
        let _ = write_cstr(buf.as_mut_ptr(), buf.len(), "x");
    }
}
