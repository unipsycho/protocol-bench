//! Live broker smoke: connect, subscribe → host upsert, publish.
//! Skips when `PB_SMOKE_SKIP=1` or no broker is reachable.
//!
//! Auth (this host): `PB_SMOKE_USER` / `PB_SMOKE_PASS` (default `mqtt`/`mqtt`).

use pb_abi::{PbHostApi, PbPackApi, PB_ABI_VERSION, PB_VALUE_STRING};
use pb_mqtt::{pb_pack_abi_version, pb_pack_register};
use std::ffi::CStr;
use std::os::raw::{c_char, c_void};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

struct Captured {
    points: Vec<(String, String)>,
}

unsafe extern "C" fn upsert(
    ctx: *mut c_void,
    name: *const c_char,
    value_kind: i32,
    value: *const c_void,
    value_len: usize,
    _quality: i32,
    _source: *const c_char,
) {
    if ctx.is_null() || name.is_null() || value_kind != PB_VALUE_STRING {
        return;
    }
    let cap = &*(ctx as *const Mutex<Captured>);
    let name = CStr::from_ptr(name).to_string_lossy().into_owned();
    let payload = if value.is_null() {
        String::new()
    } else {
        let slice = std::slice::from_raw_parts(value as *const u8, value_len);
        String::from_utf8_lossy(slice).into_owned()
    };
    if let Ok(mut g) = cap.lock() {
        g.points.push((name, payload));
    }
}

unsafe extern "C" fn log_message(_ctx: *mut c_void, _level: i32, message: *const c_char) {
    if message.is_null() {
        return;
    }
    eprintln!("smoke-pack: {}", CStr::from_ptr(message).to_string_lossy());
}

fn broker_reachable(host: &str, port: u16) -> bool {
    std::net::TcpStream::connect_timeout(
        &format!("{host}:{port}").parse().unwrap(),
        Duration::from_millis(400),
    )
    .is_ok()
}

fn smoke_env() -> Option<(String, u16, String, String)> {
    if std::env::var_os("PB_SMOKE_SKIP").is_some() {
        eprintln!("PB_SMOKE_SKIP set — skipping live MQTT smoke");
        return None;
    }
    let host = std::env::var("PB_SMOKE_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let port: u16 = std::env::var("PB_SMOKE_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1883);
    if !broker_reachable(&host, port) {
        eprintln!("no broker at {host}:{port} — skipping live MQTT smoke");
        return None;
    }
    let user = std::env::var("PB_SMOKE_USER").unwrap_or_else(|_| "mqtt".into());
    let pass = std::env::var("PB_SMOKE_PASS").unwrap_or_else(|_| "mqtt".into());
    Some((host, port, user, pass))
}

fn open_pack(captured: &Arc<Mutex<Captured>>) -> (*mut c_void, PbPackApi) {
    assert_eq!(unsafe { pb_pack_abi_version() }, PB_ABI_VERSION);
    let host_api = PbHostApi {
        abi_version: PB_ABI_VERSION,
        ctx: Arc::as_ptr(captured) as *mut c_void,
        upsert_point: Some(upsert),
        log_message: Some(log_message),
    };
    let mut api = unsafe { std::mem::zeroed::<PbPackApi>() };
    assert_eq!(unsafe { pb_pack_register(&mut api) }, 0);
    let create = api.create.expect("create");
    let handle = unsafe { create(&host_api) };
    assert!(!handle.is_null());
    (handle, api)
}

fn wait_connected(api: &PbPackApi, handle: *mut c_void, secs: u64) {
    let poll = api.poll.expect("poll");
    let is_connected = api.is_connected.expect("is_connected");
    let deadline = std::time::Instant::now() + Duration::from_secs(secs);
    while std::time::Instant::now() < deadline {
        unsafe { poll(handle) };
        if unsafe { is_connected(handle) } != 0 {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(
        unsafe { is_connected(handle) },
        1,
        "failed to connect within {secs}s"
    );
}

fn mosquitto_pub(host: &str, port: u16, user: &str, pass: &str, topic: &str, msg: &str) {
    let status = std::process::Command::new("mosquitto_pub")
        .args([
            "-h",
            host,
            "-p",
            &port.to_string(),
            "-u",
            user,
            "-P",
            pass,
            "-t",
            topic,
            "-m",
            msg,
        ])
        .status()
        .expect("mosquitto_pub");
    assert!(status.success(), "mosquitto_pub failed for {topic}");
}

#[test]
fn subscribe_point_and_publish() {
    let Some((host, port, user, pass)) = smoke_env() else {
        return;
    };
    let captured = Arc::new(Mutex::new(Captured { points: Vec::new() }));
    let (handle, api) = open_pack(&captured);

    let topic = "pb/smoke/in";
    let cfg = format!(
        r#"{{"clients":[{{"id":"smoke","label":"smoke","host":"{host}","port":{port},"client_id":"pb-smoke","username":"{user}","password":"{pass}","transport":"tcp","subscriptions":["{topic}"],"parse_rules":[{{"topic":"{topic}","path":"","point":""}}],"outputs":[{{"point":"mqtt_1","topic":"pb/smoke/write","qos":0,"retain":false}}],"auto_connect":true}}]}}"#
    );
    let configure = api.configure.expect("configure");
    assert_eq!(
        unsafe { configure(handle, cfg.as_ptr() as *const c_char, cfg.len()) },
        0
    );
    wait_connected(&api, handle, 8);

    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    mosquitto_pub(&host, port, &user, &pass, topic, "smoke-payload");
    let got = deadline_points(&captured, api.poll.expect("poll"), handle, "smoke-payload", deadline);
    assert!(
        got.iter()
            .any(|(n, v)| n.contains(topic) && v == "smoke-payload"),
        "subscribe did not upsert point: {got:?}"
    );

    let publish = api.publish.expect("publish");
    let out_topic = std::ffi::CString::new("pb/smoke/out").unwrap();
    let payload = b"from-pack";
    assert_eq!(
        unsafe {
            publish(
                handle,
                out_topic.as_ptr(),
                payload.as_ptr(),
                payload.len(),
                0,
                0,
            )
        },
        0
    );

    let write_point = api.write_point.expect("write_point");
    let point_name = std::ffi::CString::new("mqtt_1").unwrap();
    let body = b"via-point";
    assert_eq!(
        unsafe {
            write_point(
                handle,
                point_name.as_ptr(),
                PB_VALUE_STRING,
                body.as_ptr() as *const c_void,
                body.len(),
            )
        },
        0
    );

    assert_eq!(unsafe { api.disconnect.expect("disconnect")(handle) }, 0);
    unsafe { api.destroy.expect("destroy")(handle) };
}

/// `#` and `+` filters must connect, stay subscribed, and promote matching traffic.
#[test]
fn wildcard_hash_and_plus_subscribe() {
    let Some((host, port, user, pass)) = smoke_env() else {
        return;
    };
    let captured = Arc::new(Mutex::new(Captured { points: Vec::new() }));
    let (handle, api) = open_pack(&captured);

    let cfg = format!(
        r#"{{"clients":[{{"id":"wc","label":"wc","host":"{host}","port":{port},"client_id":"pb-wc","username":"{user}","password":"{pass}","transport":"tcp","subscriptions":["pb/wc/#","pb/plus/+/x"],"parse_rules":[{{"topic":"pb/wc/#","path":"","point":""}},{{"topic":"pb/plus/+/x","path":"","point":""}}],"auto_connect":true}}]}}"#
    );
    let configure = api.configure.expect("configure");
    assert_eq!(
        unsafe { configure(handle, cfg.as_ptr() as *const c_char, cfg.len()) },
        0,
        "configure with # / + filters failed"
    );
    wait_connected(&api, handle, 8);

    // Stay connected after wildcard sync.
    thread::sleep(Duration::from_millis(200));
    unsafe { api.poll.expect("poll")(handle) };
    assert_eq!(
        unsafe { api.is_connected.expect("is_connected")(handle) },
        1,
        "connection dropped after wildcard subscribe"
    );

    // Snapshot must remain valid JSON with subscriptions intact (not truncated).
    let topics_json = api.topics_json.expect("topics_json");
    let mut buf = vec![0u8; 64 * 1024];
    let n = unsafe {
        topics_json(
            handle,
            buf.as_mut_ptr() as *mut c_char,
            buf.len(),
        )
    };
    assert!(n > 0, "topics_json empty");
    let snap = String::from_utf8_lossy(&buf[..n as usize]).into_owned();
    let parsed: serde_json::Value =
        serde_json::from_str(&snap).expect(&format!("topics_json invalid: {snap}"));
    let subs = parsed[0]["topics"].as_array().expect("topics array");
    let sub_strs: Vec<&str> = subs.iter().filter_map(|v| v.as_str()).collect();
    assert!(
        sub_strs.iter().any(|s| *s == "pb/wc/#"),
        "missing # filter in snapshot: {sub_strs:?}"
    );
    assert!(
        sub_strs.iter().any(|s| *s == "pb/plus/+/x"),
        "missing + filter in snapshot: {sub_strs:?}"
    );

    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    mosquitto_pub(&host, port, &user, &pass, "pb/wc/deep/leaf", "hash-hit");
    mosquitto_pub(&host, port, &user, &pass, "pb/plus/mid/x", "plus-hit");

    let got = deadline_points(&captured, api.poll.expect("poll"), handle, "hash-hit", deadline);
    assert!(
        got.iter().any(|(n, v)| n.contains("pb/wc/deep/leaf") && v == "hash-hit"),
        "# filter did not promote: {got:?}"
    );
    let got2 = deadline_points(
        &captured,
        api.poll.expect("poll"),
        handle,
        "plus-hit",
        std::time::Instant::now() + Duration::from_secs(4),
    );
    assert!(
        got2
            .iter()
            .any(|(n, v)| n.contains("pb/plus/mid/x") && v == "plus-hit"),
        "+ filter did not promote: {got2:?}"
    );

    assert_eq!(unsafe { api.disconnect.expect("disconnect")(handle) }, 0);
    unsafe { api.destroy.expect("destroy")(handle) };
}

fn deadline_points(
    captured: &Arc<Mutex<Captured>>,
    poll: unsafe extern "C" fn(*mut c_void),
    handle: *mut c_void,
    needle: &str,
    deadline: std::time::Instant,
) -> Vec<(String, String)> {
    while std::time::Instant::now() < deadline {
        unsafe { poll(handle) };
        let snap = captured.lock().unwrap().points.clone();
        if snap.iter().any(|(_, v)| v == needle) {
            return snap;
        }
        thread::sleep(Duration::from_millis(40));
    }
    captured.lock().unwrap().points.clone()
}
