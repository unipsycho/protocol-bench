//! C-ABI pack: one worker thread per SNMP target. Polls each Input node's OIDs on its interval,
//! marks points bad when the agent stops answering, recovers by itself.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pb_abi::worker::{sleep_stop, update_status};
use pb_abi::{HostRef, Pack, PB_LOG_ERROR, PB_LOG_INFO, PB_QUALITY_BAD, PB_QUALITY_GOOD};
use serde::{Deserialize, Serialize};

use crate::ber::{normalize_oid, Value};
use crate::client::{ClientError, Manager, TargetCfg};

const PACK_ID: &[u8] = b"snmp\0";
const PACK_TITLE: &[u8] = b"SNMP\0";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PackConfig {
    #[serde(default)]
    targets: Vec<TargetJson>,
    #[serde(default)]
    inputs: Vec<InputJson>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct TargetJson {
    id: String,
    #[serde(default = "default_host")]
    host: String,
    #[serde(default = "default_port")]
    port: u16,
    #[serde(default = "default_version")]
    version: String,
    #[serde(default = "default_community")]
    community: String,
    #[serde(default = "default_timeout")]
    timeout_secs: u32,
}

fn default_host() -> String {
    "127.0.0.1".into()
}
fn default_port() -> u16 {
    161
}
fn default_version() -> String {
    "v2c".into()
}
fn default_community() -> String {
    "public".into()
}
fn default_timeout() -> u32 {
    3
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct BindJson {
    oid: String,
    #[serde(default)]
    point: String,
    #[serde(default)]
    source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct InputJson {
    target: String,
    #[serde(default = "default_interval")]
    interval_secs: f64,
    binds: Vec<BindJson>,
}

fn default_interval() -> f64 {
    10.0
}

/// Convert an SNMP value into a host point.
trait PutValue {
    fn put(&self, name: &str, source: &str, v: &Value, quality: i32);
}

impl PutValue for HostRef {
    fn put(&self, name: &str, source: &str, v: &Value, quality: i32) {
        match v {
            Value::Int(i) => self.upsert_i64(name, source, *i, quality),
            Value::UInt(u) | Value::Ticks(u) => self.upsert_u64(name, source, *u, quality),
            Value::Text(t) | Value::Oid(t) | Value::Ip(t) => self.upsert_text(name, source, t, quality),
            Value::Null | Value::Missing(_) => self.upsert_text(name, source, "", quality),
        }
    }
}

#[derive(Default, Clone)]
struct Status {
    state: String,
    error: String,
    connected: bool,
    polls: u64,
    errors: u64,
}

struct Bind {
    oid: String,
    name: String,
    source: String,
    last: Option<Value>,
}

struct Group {
    interval: Duration,
    next_due: Instant,
    binds: Vec<Bind>,
}

struct Worker {
    id: String,
    key: String,
    endpoint: String,
    stop: Arc<AtomicBool>,
    status: Arc<Mutex<Status>>,
    join: Option<JoinHandle<()>>,
}

impl Worker {
    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

fn mark_bad(host: &HostRef, g: &mut Group) {
    for b in &g.binds {
        host.put(&b.name, &b.source, b.last.as_ref().unwrap_or(&Value::Null), PB_QUALITY_BAD);
    }
}

fn run_target(host: HostRef, t: TargetJson, mut groups: Vec<Group>, stop: Arc<AtomicBool>, status: Arc<Mutex<Status>>) {
    let cfg = TargetCfg {
        host: t.host.clone(),
        port: t.port,
        version: t.version.clone(),
        community: t.community.clone(),
        timeout: Duration::from_secs(t.timeout_secs.clamp(1, 60) as u64),
        retries: 2,
    };
    let endpoint = format!("{}:{}", t.host, t.port);
    let mut mgr: Option<Manager> = None;
    while !stop.load(Ordering::SeqCst) {
        if mgr.is_none() {
            match Manager::open(&cfg) {
                Ok(m) => mgr = Some(m),
                Err(e) => {
                    for g in &mut groups {
                        mark_bad(&host, g);
                    }
                    host.log(PB_LOG_ERROR, &format!("snmp {}: {endpoint} — {e}", t.id));
                    update_status(&status, |s| {
                        s.connected = false;
                        s.state = "error".into();
                        s.error = format!("{endpoint}: {e}");
                        s.errors += 1;
                    });
                    sleep_stop(&stop, Duration::from_secs(5));
                    continue;
                }
            }
        }
        let now = Instant::now();
        for g in &mut groups {
            if now < g.next_due {
                continue;
            }
            g.next_due = now + g.interval;
            let oids: Vec<String> = g.binds.iter().map(|b| b.oid.clone()).collect();
            match mgr.as_mut().unwrap().get(&oids) {
                Ok(results) => {
                    let mut problem: Option<String> = None;
                    for (b, r) in g.binds.iter_mut().zip(results) {
                        match r {
                            Ok(v) => {
                                host.put(&b.name, &b.source, &v, PB_QUALITY_GOOD);
                                b.last = Some(v);
                            }
                            Err(why) => {
                                host.put(&b.name, &b.source, b.last.as_ref().unwrap_or(&Value::Null), PB_QUALITY_BAD);
                                problem.get_or_insert(format!("{}: {why}", b.oid));
                            }
                        }
                    }
                    update_status(&status, |s| {
                        s.connected = true;
                        s.state = "ok".into();
                        s.polls += 1;
                        match problem {
                            Some(p) => {
                                s.error = p;
                                s.errors += 1;
                            }
                            None => s.error.clear(),
                        }
                    });
                }
                Err(e @ (ClientError::Timeout | ClientError::Io(_))) => {
                    mark_bad(&host, g);
                    host.log(PB_LOG_ERROR, &format!("snmp {}: {endpoint} — {e}", t.id));
                    update_status(&status, |s| {
                        s.connected = false;
                        s.state = "no response".into();
                        s.error = format!("{endpoint}: {e}");
                        s.errors += 1;
                    });
                    // A fresh socket next time (network may have changed).
                    mgr = None;
                    break;
                }
                Err(e) => {
                    mark_bad(&host, g);
                    update_status(&status, |s| {
                        s.connected = true;
                        s.state = "error".into();
                        s.error = e.to_string();
                        s.errors += 1;
                    });
                }
            }
        }
        let wait = groups
            .iter()
            .map(|g| g.next_due.saturating_duration_since(Instant::now()))
            .min()
            .unwrap_or(Duration::from_millis(200))
            .min(Duration::from_millis(200));
        sleep_stop(&stop, wait);
    }
    update_status(&status, |s| {
        s.connected = false;
        s.state = "stopped".into();
    });
}

struct PackState {
    host: HostRef,
    config: PackConfig,
    workers: Vec<Worker>,
    enabled: bool,
    config_error: String,
}

impl PackState {

    fn stop_all(&mut self) {
        for w in &mut self.workers {
            w.stop.store(true, Ordering::SeqCst);
        }
        for w in &mut self.workers {
            w.shutdown();
        }
        self.workers.clear();
    }

    fn configure_json(&mut self, text: &str) -> Result<(), String> {
        self.config = serde_json::from_str(text).map_err(|e| format!("json: {e}"))?;
        self.sync_workers();
        Ok(())
    }

    fn sync_workers(&mut self) {
        self.config_error.clear();
        let mut old = std::mem::take(&mut self.workers);
        let mut keep = Vec::new();
        let mut spawn = Vec::new();
        if self.enabled {
            for t in &self.config.targets {
                let inputs: Vec<&InputJson> = self.config.inputs.iter().filter(|i| i.target == t.id && !i.binds.is_empty()).collect();
                if inputs.is_empty() {
                    continue;
                }
                let key = serde_json::to_string(&(t, &inputs)).unwrap_or_default();
                if let Some(pos) = old.iter().position(|w| w.id == t.id && w.key == key) {
                    keep.push(old.remove(pos));
                } else {
                    spawn.push((t.clone(), key));
                }
            }
        }
        for w in &mut old {
            w.stop.store(true, Ordering::SeqCst);
        }
        for w in &mut old {
            w.shutdown();
        }
        for (t, key) in spawn {
            match self.spawn_worker(t, key) {
                Ok(w) => keep.push(w),
                Err(e) => {
                    self.host.log(PB_LOG_ERROR, &format!("snmp: {e}"));
                    self.config_error = e;
                }
            }
        }
        self.workers = keep;
        self.host.log(PB_LOG_INFO, &format!("snmp configured: {} target(s)", self.workers.len()));
    }

    fn spawn_worker(&self, t: TargetJson, key: String) -> Result<Worker, String> {
        if t.host.trim().is_empty() {
            return Err(format!("{}: no host set", t.id));
        }
        if !matches!(t.version.to_ascii_lowercase().as_str(), "v1" | "1" | "v2c" | "2c" | "v2") {
            return Err(format!("{}: SNMP {} is not supported yet (v1 / v2c)", t.id, t.version));
        }
        let mut groups = Vec::new();
        for inp in self.config.inputs.iter().filter(|i| i.target == t.id) {
            let mut binds = Vec::new();
            for b in &inp.binds {
                let oid = normalize_oid(&b.oid);
                crate::ber::encode_oid(&oid).map_err(|_| format!("{}: invalid OID \"{}\"", t.id, b.oid))?;
                let source = if b.source.is_empty() { format!("snmp/{}::{}", t.id, b.oid.trim()) } else { b.source.clone() };
                let name = if b.point.trim().is_empty() { source.clone() } else { b.point.trim().to_string() };
                binds.push(Bind { oid, name, source, last: None });
            }
            if !binds.is_empty() {
                groups.push(Group { interval: Duration::from_secs_f64(inp.interval_secs.clamp(0.2, 86_400.0)), next_due: Instant::now(), binds });
            }
        }
        let stop = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new(Status { state: "starting".into(), ..Default::default() }));
        let (h, tt, sp, st) = (self.host, t.clone(), stop.clone(), status.clone());
        let join = std::thread::Builder::new()
            .name(format!("pb-snmp-{}", t.id))
            .spawn(move || run_target(h, tt, groups, sp, st))
            .map_err(|e| format!("spawn: {e}"))?;
        Ok(Worker { id: t.id.clone(), key, endpoint: format!("{}:{}", t.host, t.port), stop, status, join: Some(join) })
    }

    fn statuses(&self) -> Vec<(String, String, Status)> {
        self.workers.iter().map(|w| (w.id.clone(), w.endpoint.clone(), w.status.lock().map(|s| s.clone()).unwrap_or_default())).collect()
    }



}

impl Drop for PackState {
    fn drop(&mut self) {
        self.stop_all();
    }
}

impl Pack for PackState {
    const ID: &'static [u8] = PACK_ID;
    const TITLE: &'static [u8] = PACK_TITLE;

    fn new(host: HostRef) -> Self {
        Self { host, config: PackConfig::default(), workers: Vec::new(), enabled: true, config_error: String::new() }
    }

    fn configure(&mut self, json: &str) -> Result<(), String> {
        self.configure_json(json)
    }

    fn config_failed(&mut self, err: String) {
        self.host.log(PB_LOG_ERROR, &err);
        self.config_error = err;
    }

    fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
        self.sync_workers();
    }

    fn shutdown(&mut self) {
        self.stop_all();
    }

    fn state_label(&self) -> String {
        if !self.config_error.is_empty() {
            return "config error".into();
        }
        let st = self.statuses();
        if st.is_empty() {
            return if self.enabled { "idle".into() } else { "stopped".into() };
        }
        format!("{}/{} agents answering", st.iter().filter(|(_, _, s)| s.connected).count(), st.len())
    }

    fn last_error(&self) -> String {
        if !self.config_error.is_empty() {
            return self.config_error.clone();
        }
        self.statuses().into_iter().find(|(_, _, s)| !s.error.is_empty()).map(|(id, _, s)| format!("{id}: {}", s.error)).unwrap_or_default()
    }

    fn is_connected(&self) -> bool {
        self.statuses().iter().any(|(_, _, s)| s.connected)
    }

    fn detail_json(&self) -> String {
        let targets: Vec<serde_json::Value> = self
            .statuses()
            .into_iter()
            .map(|(id, endpoint, s)| {
                serde_json::json!({
                    "id": id, "endpoint": endpoint, "state": s.state, "connected": s.connected,
                    "error": s.error, "polls": s.polls, "errors": s.errors,
                })
            })
            .collect();
        serde_json::json!({ "targets": targets }).to_string()
    }
}

pb_abi::export_pack!(PackState);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::testagent::Agent;
    use pb_abi::{PbHostApi, PbPackApi, PB_ABI_VERSION, PB_VALUE_I64, PB_VALUE_F64};
    use std::ffi::CStr;
    use std::os::raw::{c_char, c_void};
    use std::sync::atomic::Ordering;
    use std::sync::Mutex as StdMutex;

    #[derive(Debug, Clone)]
    struct Rec {
        name: String,
        v: Value,
        quality: i32,
    }
    type Log = StdMutex<Vec<Rec>>;

    unsafe extern "C" fn cap(ctx: *mut c_void, name: *const c_char, kind: i32, value: *const c_void, len: usize, quality: i32, _s: *const c_char) {
        let log = &*(ctx as *const Log);
        let name = CStr::from_ptr(name).to_string_lossy().into_owned();
        let v = match kind {
            PB_VALUE_I64 => Value::Int(*(value as *const i64)),
            PB_VALUE_F64 => Value::UInt(*(value as *const f64) as u64),
            _ => Value::Text(String::from_utf8_lossy(std::slice::from_raw_parts(value as *const u8, len)).into_owned()),
        };
        log.lock().unwrap().push(Rec { name, v, quality });
    }

    fn leak() -> &'static Log {
        Box::leak(Box::new(StdMutex::new(Vec::new())))
    }

    fn host(log: &'static Log) -> HostRef {
        HostRef::new(PbHostApi { abi_version: PB_ABI_VERSION, ctx: log as *const Log as *mut c_void, upsert_point: Some(cap), log_message: None })
    }

    fn wait(log: &'static Log, what: &str, f: impl Fn(&[Rec]) -> bool) {
        let end = Instant::now() + Duration::from_secs(8);
        while Instant::now() < end {
            if f(&log.lock().unwrap()) {
                return;
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        panic!("timed out waiting for {what}; log = {:?}", log.lock().unwrap());
    }

    fn cfg(port: u16, version: &str, extra_oid: &str) -> String {
        serde_json::json!({
            "targets": [{"id":"sw","host":"127.0.0.1","port":port,"version":version,"community":"public","timeout_secs":1}],
            "inputs": [{"target":"sw","interval_secs":0.2,"binds":[
                {"oid":"1.3.6.1.2.1.1.5.0","point":"snmp_1"},
                {"oid":".1.3.6.1.2.1.2.2.1.10.1","point":"snmp_2"},
                {"oid":"1.3.6.1.2.1.1.3.0","point":"snmp_3"},
                {"oid":extra_oid,"point":"snmp_4"}]}]
        })
        .to_string()
    }

    #[test]
    fn polls_typed_values_and_flags_only_the_missing_oid() {
        let a = Agent::start("public");
        a.set("1.3.6.1.2.1.1.5.0", Value::Text("core-sw".into()));
        a.set("1.3.6.1.2.1.2.2.1.10.1", Value::UInt(5_000_000_000));
        a.set("1.3.6.1.2.1.1.3.0", Value::Ticks(4200));
        let log = leak();
        let mut st = PackState::new(host(log));
        st.configure_json(&cfg(a.addr.port(), "v2c", "1.3.6.1.2.1.9.9.9")).unwrap();
        wait(log, "values", |l| {
            l.iter().any(|r| r.name == "snmp_1" && r.quality == PB_QUALITY_GOOD)
                && l.iter().any(|r| r.name == "snmp_2" && r.quality == PB_QUALITY_GOOD)
                && l.iter().any(|r| r.name == "snmp_4" && r.quality == PB_QUALITY_BAD)
        });
        let last = |n: &str| log.lock().unwrap().iter().rev().find(|r| r.name == n).cloned().unwrap();
        assert_eq!(last("snmp_1").v, Value::Text("core-sw".into()));
        assert_eq!(last("snmp_2").v, Value::Int(5_000_000_000), "a Gauge32/Counter64 that fits i64 goes out as an integer");
        assert_eq!(last("snmp_3").v, Value::Int(4200));
        assert_eq!(last("snmp_3").quality, PB_QUALITY_GOOD);
        assert!(st.last_error().contains("1.3.6.1.2.1.9.9.9: noSuchInstance"), "{}", st.last_error());
        assert_eq!(st.state_label(), "1/1 agents answering");
        assert!(st.detail_json().contains("\"state\":\"ok\""));
        st.stop_all();
    }

    #[test]
    fn dead_agent_goes_bad_holds_last_value_and_recovers() {
        let a = Agent::start("public");
        a.set("1.3.6.1.2.1.1.5.0", Value::Text("alive".into()));
        a.set("1.3.6.1.2.1.2.2.1.10.1", Value::UInt(1));
        a.set("1.3.6.1.2.1.1.3.0", Value::Ticks(1));
        a.set("1.3.6.1.2.1.9.9.9", Value::Int(1));
        let log = leak();
        let mut st = PackState::new(host(log));
        st.configure_json(&cfg(a.addr.port(), "v2c", "1.3.6.1.2.1.9.9.9")).unwrap();
        wait(log, "good", |l| l.iter().any(|r| r.name == "snmp_1" && r.quality == PB_QUALITY_GOOD));
        // Agent stops answering (all datagrams dropped).
        a.drop_first.store(10_000, Ordering::SeqCst);
        wait(log, "bad after timeout", |l| l.iter().any(|r| r.name == "snmp_1" && r.quality == PB_QUALITY_BAD && r.v == Value::Text("alive".into())));
        assert_eq!(st.state_label(), "0/1 agents answering");
        assert!(st.last_error().contains("timeout"), "{}", st.last_error());
        // Agent is back.
        a.drop_first.store(0, Ordering::SeqCst);
        let bad_count = log.lock().unwrap().len();
        wait(log, "recovery", |l| l.len() > bad_count && l[bad_count..].iter().any(|r| r.name == "snmp_1" && r.quality == PB_QUALITY_GOOD));
        st.stop_all();
    }

    #[test]
    fn v3_and_bad_oids_are_config_errors_and_unchanged_workers_survive() {
        let a = Agent::start("public");
        let log = leak();
        let mut st = PackState::new(host(log));
        let good = cfg(a.addr.port(), "v2c", "1.3.6.1.2.1.9.9.9");
        st.configure_json(&good).unwrap();
        let id1 = st.workers[0].join.as_ref().unwrap().thread().id();
        st.configure_json(&good).unwrap();
        assert_eq!(st.workers[0].join.as_ref().unwrap().thread().id(), id1);
        st.configure_json(&cfg(a.addr.port(), "v3", "1.3.6.1.2.1.9.9.9")).unwrap();
        assert!(st.last_error().contains("not supported"), "{}", st.last_error());
        assert_eq!(st.state_label(), "config error");
        st.configure_json(&cfg(a.addr.port(), "v2c", "nonsense")).unwrap();
        assert!(st.last_error().contains("invalid OID"), "{}", st.last_error());
        st.stop_all();
    }

    #[test]
    fn abi_registers() {
        let mut api = std::mem::MaybeUninit::<PbPackApi>::uninit();
        assert_eq!(unsafe { pb_pack_register(api.as_mut_ptr()) }, 0);
        assert_eq!(unsafe { pb_pack_abi_version() }, PB_ABI_VERSION);
    }
}
