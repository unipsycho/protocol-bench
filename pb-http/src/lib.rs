//! HTTP / REST protocol pack (cdylib) for Protocol Bench.
//!
//! Outbound clients: poll JSON paths → points; Output binds assemble JSON body push.

use pb_abi::{
    write_cstr, PbHostApi, PbPackApi, PB_ABI_VERSION, PB_LOG_ERROR, PB_LOG_INFO, PB_QUALITY_GOOD,
    PB_VALUE_STRING,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::ffi::CStr;
use std::os::raw::{c_char, c_int, c_void};
use std::time::{Duration, Instant};

const PACK_ID: &[u8] = b"http\0";
const PACK_TITLE: &[u8] = b"HTTP / REST\0";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PackConfig {
    #[serde(default)]
    clients: Vec<ClientJson>,
    #[serde(default)]
    inputs: Vec<InputPoll>,
    #[serde(default)]
    outputs: Vec<OutputPush>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ClientJson {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default = "default_auth")]
    pub auth: String,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    #[serde(default)]
    pub headers: Vec<HeaderJson>,
}

fn default_auth() -> String {
    "none".into()
}
fn default_timeout() -> u64 {
    10
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HeaderJson {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InputPoll {
    pub client: String,
    #[serde(default = "default_get")]
    pub method: String,
    #[serde(default)]
    pub path: String,
    #[serde(default = "default_interval")]
    pub interval_secs: f64,
    #[serde(default)]
    pub binds: Vec<PathBind>,
}

fn default_get() -> String {
    "GET".into()
}
fn default_interval() -> f64 {
    5.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PathBind {
    #[serde(default)]
    pub json_path: String,
    pub point: String,
    #[serde(default)]
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OutputPush {
    pub client: String,
    #[serde(default = "default_post")]
    pub method: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub binds: Vec<PathBind>,
}

fn default_post() -> String {
    "POST".into()
}

struct PackState {
    host: PbHostApi,
    config: PackConfig,
    last_poll: BTreeMap<String, Instant>,
    last_error: String,
    state: String,
    last_body: String,
    poll_count: u64,
    point_values: BTreeMap<String, String>,
}

impl PackState {
    fn new(host: PbHostApi) -> Self {
        Self {
            host,
            config: PackConfig::default(),
            last_poll: BTreeMap::new(),
            last_error: String::new(),
            state: "idle".into(),
            last_body: String::new(),
            poll_count: 0,
            point_values: BTreeMap::new(),
        }
    }

    fn log(&self, level: i32, msg: &str) {
        if let Some(log) = self.host.log_message {
            if let Ok(c) = std::ffi::CString::new(msg) {
                unsafe { log(self.host.ctx, level, c.as_ptr()) };
            }
        }
    }

    fn upsert_string(&self, name: &str, source: &str, value: &str) {
        let Some(upsert) = self.host.upsert_point else {
            return;
        };
        let Ok(c_name) = std::ffi::CString::new(name) else {
            return;
        };
        let Ok(c_src) = std::ffi::CString::new(source) else {
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
                PB_QUALITY_GOOD,
                c_src.as_ptr(),
            );
        }
    }

    fn configure_json(&mut self, text: &str) -> Result<(), String> {
        let cfg: PackConfig = serde_json::from_str(text).map_err(|e| format!("json: {e}"))?;
        self.config = cfg;
        self.last_poll.clear();
        self.last_error.clear();
        self.state = if self.config.clients.is_empty() {
            "idle".into()
        } else {
            format!("{} clients", self.config.clients.len())
        };
        self.log(
            PB_LOG_INFO,
            &format!(
                "http configured: {} clients, {} inputs, {} outputs",
                self.config.clients.len(),
                self.config.inputs.len(),
                self.config.outputs.len()
            ),
        );
        Ok(())
    }

    fn client(&self, id: &str) -> Option<&ClientJson> {
        self.config.clients.iter().find(|c| c.id == id)
    }

    fn poll(&mut self) {
        let inputs = self.config.inputs.clone();
        for inp in inputs {
            let key = format!("{}:{}:{}", inp.client, inp.method, inp.path);
            let interval = Duration::from_secs_f64(inp.interval_secs.max(0.5));
            let due = match self.last_poll.get(&key) {
                Some(t) => t.elapsed() >= interval,
                None => true,
            };
            if !due {
                continue;
            }
            match self.run_input(&inp) {
                Ok(()) => {
                    self.last_poll.insert(key, Instant::now());
                    self.poll_count = self.poll_count.saturating_add(1);
                    self.state = "ok".into();
                }
                Err(e) => {
                    self.last_error = e.clone();
                    self.state = "error".into();
                    self.log(PB_LOG_ERROR, &e);
                    self.last_poll.insert(key, Instant::now());
                }
            }
        }
    }

    fn run_input(&mut self, inp: &InputPoll) -> Result<(), String> {
        let client = self
            .client(&inp.client)
            .ok_or_else(|| format!("unknown HTTP client {}", inp.client))?;
        let body = http_request(client, &inp.method, &inp.path, None)?;
        self.last_body = body.chars().take(4000).collect();
        let value: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
        for bind in &inp.binds {
            let extracted = extract_json_path(&value, &bind.json_path);
            let text = value_to_string(&extracted);
            let source = if bind.source.is_empty() {
                format!(
                    "http/{}/{}::{}",
                    inp.client,
                    inp.path.trim_start_matches('/'),
                    bind.json_path
                )
            } else {
                bind.source.clone()
            };
            let name = if bind.point.is_empty() {
                source.clone()
            } else {
                bind.point.clone()
            };
            self.point_values.insert(name.clone(), text.clone());
            self.upsert_string(&name, &source, &text);
        }
        Ok(())
    }

    fn write_point(&mut self, name: &str, value: &str) {
        self.point_values.insert(name.to_string(), value.to_string());
        let outs = self.config.outputs.clone();
        for out in outs {
            if out.binds.iter().any(|b| b.point == name) {
                if let Err(e) = self.run_output(&out) {
                    self.last_error = e.clone();
                    self.log(PB_LOG_ERROR, &e);
                }
            }
        }
    }

    fn run_output(&mut self, out: &OutputPush) -> Result<(), String> {
        let client = self
            .client(&out.client)
            .ok_or_else(|| format!("unknown HTTP client {}", out.client))?;
        let mut root = serde_json::Map::new();
        for bind in &out.binds {
            let raw = self
                .point_values
                .get(&bind.point)
                .cloned()
                .unwrap_or_default();
            set_json_path(&mut root, &bind.json_path, json_value_from_point(&raw));
        }
        let body = serde_json::to_string(&Value::Object(root)).map_err(|e| e.to_string())?;
        let _ = http_request(client, &out.method, &out.path, Some(&body))?;
        Ok(())
    }

    fn detail_json(&self) -> String {
        serde_json::json!({
            "state": self.state,
            "poll_count": self.poll_count,
            "last_error": self.last_error,
            "last_body": self.last_body,
            "clients": self.config.clients.len(),
            "inputs": self.config.inputs.len(),
            "outputs": self.config.outputs.len(),
        })
        .to_string()
    }
}

fn http_request(
    client: &ClientJson,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> Result<String, String> {
    let url = join_url(&client.base_url, path);
    if url.is_empty() {
        return Err("empty URL".into());
    }
    let timeout = Duration::from_secs(client.timeout_secs.max(1));
    let agent = ureq::AgentBuilder::new().timeout(timeout).build();
    let method = method.trim().to_ascii_uppercase();
    let mut req = agent.request(&method, &url);
    match client.auth.to_ascii_lowercase().as_str() {
        "bearer" => {
            let token = client.token.trim();
            if !token.is_empty() {
                req = req.set("Authorization", &format!("Bearer {token}"));
            }
        }
        "basic" => {
            let raw = format!("{}:{}", client.username, client.password);
            let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, raw);
            req = req.set("Authorization", &format!("Basic {b64}"));
        }
        _ => {}
    }
    for h in &client.headers {
        let name = h.name.trim();
        if name.is_empty() || name.eq_ignore_ascii_case("authorization") {
            continue;
        }
        req = req.set(name, &h.value);
    }
    let resp = if let Some(body) = body {
        req.set("Content-Type", "application/json")
            .send_string(body)
            .map_err(|e| format!("HTTP {method} {url}: {e}"))?
    } else {
        req.call()
            .map_err(|e| format!("HTTP {method} {url}: {e}"))?
    };
    let status = resp.status();
    let text = resp
        .into_string()
        .map_err(|e| format!("read body: {e}"))?;
    if !(200..300).contains(&status) {
        return Err(format!(
            "HTTP {status}: {}",
            text.chars().take(180).collect::<String>()
        ));
    }
    Ok(text)
}

fn join_url(base: &str, path: &str) -> String {
    let base = base.trim().trim_end_matches('/');
    let path = path.trim();
    if base.is_empty() {
        return String::new();
    }
    if path.is_empty() {
        return base.to_string();
    }
    if path.starts_with("http://") || path.starts_with("https://") {
        return path.to_string();
    }
    if path.starts_with('/') {
        format!("{base}{path}")
    } else {
        format!("{base}/{path}")
    }
}

fn extract_json_path(value: &Value, path: &str) -> Value {
    let path = path.trim().trim_start_matches('.');
    if path.is_empty() {
        return value.clone();
    }
    let mut cur = value;
    for part in path.split('.') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        match cur {
            Value::Object(map) => {
                cur = match map.get(part) {
                    Some(v) => v,
                    None => return Value::Null,
                };
            }
            Value::Array(arr) => {
                let Ok(idx) = part.parse::<usize>() else {
                    return Value::Null;
                };
                cur = match arr.get(idx) {
                    Some(v) => v,
                    None => return Value::Null,
                };
            }
            _ => return Value::Null,
        }
    }
    cur.clone()
}

fn value_to_string(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

fn json_value_from_point(raw: &str) -> Value {
    let t = raw.trim();
    if t.is_empty() {
        return Value::Null;
    }
    if let Ok(n) = t.parse::<i64>() {
        return serde_json::json!(n);
    }
    if let Ok(n) = t.parse::<f64>() {
        return serde_json::json!(n);
    }
    if t.eq_ignore_ascii_case("true") {
        return Value::Bool(true);
    }
    if t.eq_ignore_ascii_case("false") {
        return Value::Bool(false);
    }
    if let Ok(v) = serde_json::from_str::<Value>(t) {
        return v;
    }
    Value::String(raw.to_string())
}

fn set_json_path(root: &mut serde_json::Map<String, Value>, path: &str, value: Value) {
    let parts: Vec<&str> = path.split('.').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return;
    }
    let mut cur = root;
    for (i, part) in parts.iter().enumerate() {
        if i + 1 == parts.len() {
            cur.insert((*part).to_string(), value);
            return;
        }
        let entry = cur
            .entry((*part).to_string())
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        if !entry.is_object() {
            *entry = Value::Object(serde_json::Map::new());
        }
        cur = entry.as_object_mut().expect("object");
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
        subscribe: None,
        publish: None,
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
    Box::into_raw(Box::new(PackState::new(host))) as *mut c_void
}

unsafe extern "C" fn destroy(handle: *mut c_void) {
    if handle.is_null() {
        return;
    }
    drop(Box::from_raw(handle as *mut PackState));
}

unsafe extern "C" fn configure(handle: *mut c_void, json: *const c_char, len: usize) -> c_int {
    if handle.is_null() || json.is_null() {
        return -1;
    }
    let state = &mut *(handle as *mut PackState);
    let slice = std::slice::from_raw_parts(json as *const u8, len);
    let text = match std::str::from_utf8(slice) {
        Ok(t) => t,
        Err(_) => return -1,
    };
    match state.configure_json(text) {
        Ok(()) => 0,
        Err(e) => {
            state.log(PB_LOG_ERROR, &e);
            -1
        }
    }
}

unsafe extern "C" fn connect(handle: *mut c_void) -> c_int {
    if handle.is_null() {
        return -1;
    }
    let state = &mut *(handle as *mut PackState);
    state.state = "ready".into();
    0
}

unsafe extern "C" fn disconnect(handle: *mut c_void) -> c_int {
    if handle.is_null() {
        return -1;
    }
    let state = &mut *(handle as *mut PackState);
    state.state = "idle".into();
    0
}

unsafe extern "C" fn write_point(
    handle: *mut c_void,
    name: *const c_char,
    _value_kind: i32,
    value: *const c_void,
    value_len: usize,
) -> c_int {
    if handle.is_null() || name.is_null() {
        return -1;
    }
    let state = &mut *(handle as *mut PackState);
    let name = match CStr::from_ptr(name).to_str() {
        Ok(s) => s,
        Err(_) => return -1,
    };
    let bytes = if value.is_null() || value_len == 0 {
        &[][..]
    } else {
        std::slice::from_raw_parts(value as *const u8, value_len)
    };
    let text = String::from_utf8_lossy(bytes).into_owned();
    state.write_point(name, &text);
    0
}

unsafe extern "C" fn poll(handle: *mut c_void) {
    if handle.is_null() {
        return;
    }
    let state = &mut *(handle as *mut PackState);
    state.poll();
}

unsafe extern "C" fn state_label(handle: *mut c_void, out: *mut c_char, out_len: usize) -> c_int {
    if handle.is_null() {
        return -1;
    }
    let state = &*(handle as *mut PackState);
    unsafe { write_cstr(out, out_len, &state.state) }
}

unsafe extern "C" fn last_error(handle: *mut c_void, out: *mut c_char, out_len: usize) -> c_int {
    if handle.is_null() {
        return -1;
    }
    let state = &*(handle as *mut PackState);
    unsafe { write_cstr(out, out_len, &state.last_error) }
}

unsafe extern "C" fn is_connected(handle: *mut c_void) -> c_int {
    if handle.is_null() {
        return 0;
    }
    let state = &*(handle as *mut PackState);
    if state.config.clients.is_empty() {
        0
    } else {
        1
    }
}

unsafe extern "C" fn topics_json(handle: *mut c_void, out: *mut c_char, out_len: usize) -> c_int {
    if handle.is_null() {
        return -1;
    }
    let state = &*(handle as *mut PackState);
    let json = state.detail_json();
    unsafe { write_cstr(out, out_len, &json) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_ok() {
        let mut api = unsafe { std::mem::zeroed() };
        assert_eq!(unsafe { pb_pack_register(&mut api) }, 0);
    }

    #[test]
    fn path_extract() {
        let v: Value = serde_json::json!({"tank":{"level":12.5}});
        assert_eq!(extract_json_path(&v, "tank.level"), serde_json::json!(12.5));
    }
}
