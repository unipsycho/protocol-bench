//! Live broker smoke: connect, subscribe → host upsert, publish.
//! Skips when `PB_SMOKE_SKIP=1` or no broker is reachable.

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

#[test]
fn subscribe_point_and_publish() {
    if std::env::var_os("PB_SMOKE_SKIP").is_some() {
        eprintln!("PB_SMOKE_SKIP set — skipping live MQTT smoke");
        return;
    }
    let host = std::env::var("PB_SMOKE_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let port: u16 = std::env::var("PB_SMOKE_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1883);
    if !broker_reachable(&host, port) {
        eprintln!("no broker at {host}:{port} — skipping live MQTT smoke");
        return;
    }

    assert_eq!(unsafe { pb_pack_abi_version() }, PB_ABI_VERSION);

    let captured = Arc::new(Mutex::new(Captured { points: Vec::new() }));
    let host_api = PbHostApi {
        abi_version: PB_ABI_VERSION,
        ctx: Arc::as_ptr(&captured) as *mut c_void,
        upsert_point: Some(upsert),
        log_message: Some(log_message),
    };

    let mut api = unsafe { std::mem::zeroed::<PbPackApi>() };
    assert_eq!(unsafe { pb_pack_register(&mut api) }, 0);
    let create = api.create.expect("create");
    let handle = unsafe { create(&host_api) };
    assert!(!handle.is_null());

    let cfg = format!(
        r#"{{"host":"{host}","port":{port},"client_id":"pb-smoke","transport":"tcp"}}"#
    );
    let configure = api.configure.expect("configure");
    assert_eq!(unsafe { configure(handle, cfg.as_ptr() as *const c_char, cfg.len()) }, 0);

    let connect = api.connect.expect("connect");
    assert_eq!(unsafe { connect(handle) }, 0);

    let topic = "pb/smoke/in";
    let subscribe = api.subscribe.expect("subscribe");
    let c_topic = std::ffi::CString::new(topic).unwrap();
    assert_eq!(unsafe { subscribe(handle, c_topic.as_ptr(), 0) }, 0);

    // Wait for ConnAck + resubscribe path.
    let poll = api.poll.expect("poll");
    let is_connected = api.is_connected.expect("is_connected");
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    while std::time::Instant::now() < deadline {
        unsafe { poll(handle) };
        if unsafe { is_connected(handle) } != 0 {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(unsafe { is_connected(handle) }, 1, "failed to connect to {host}:{port}");

    // External publish into the subscribed topic.
    let status = std::process::Command::new("mosquitto_pub")
        .args([
            "-h",
            &host,
            "-p",
            &port.to_string(),
            "-t",
            topic,
            "-m",
            "smoke-payload",
        ])
        .status()
        .expect("mosquitto_pub");
    assert!(status.success());

    let got = deadline_points(&captured, poll, handle, "smoke-payload", deadline);
    assert!(
        got.iter().any(|(n, v)| n.contains(topic) && v == "smoke-payload"),
        "subscribe did not upsert point: {got:?}"
    );

    // Publish from the pack.
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

    // Point write → publish path.
    let write_point = api.write_point.expect("write_point");
    let point_name = std::ffi::CString::new("mqtt/pb/smoke/write").unwrap();
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

    let disconnect = api.disconnect.expect("disconnect");
    assert_eq!(unsafe { disconnect(handle) }, 0);
    let destroy = api.destroy.expect("destroy");
    unsafe { destroy(handle) };
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
