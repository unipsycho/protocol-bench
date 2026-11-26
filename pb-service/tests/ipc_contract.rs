//! IPC contract the UI relies on, exercised against the real service binary.

mod common;
use common::*;

use std::io::Write;
use std::net::TcpStream;

use pb_ipc::{read_json, write_json, Request, Response, RemoteHandle};

#[test]
fn wrong_token_is_refused_and_calls_need_hello() {
    let root = scratch("auth");
    let svc = Service::start(&root.join("xdg"), None);
    let (ep, token) = svc.endpoint();
    assert!(RemoteHandle::connect_to(&ep, "not-the-token").is_err(), "bad token must fail hello");
    assert!(RemoteHandle::connect_to(&ep, &token).is_ok());

    // Raw socket: a call before hello is refused, not served.
    let mut sock = TcpStream::connect((ep.host.as_str(), ep.port)).unwrap();
    let req = Request { id: 1, method: pb_ipc::METHOD_GET_PROJECT.into(), params: serde_json::Value::Null };
    write_json(&mut sock, &req).unwrap();
    sock.flush().unwrap();
    let resp: Response = read_json(&mut sock).unwrap();
    assert!(resp.error.as_deref().unwrap_or("").contains("hello"), "{resp:?}");
    svc.kill();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn open_get_put_roundtrip() {
    let root = scratch("roundtrip");
    let svc = Service::start(&root.join("xdg"), None);
    let h = svc.connect();
    let a = root.join("a.json");
    write_project(&a, "alpha");
    h.open_project(&a.display().to_string(), false).unwrap();
    assert_eq!(h.get_project_json().unwrap()["settings"]["title"], "alpha");

    // Put with no path saves over the open project.
    let mut doc = h.get_project_json().unwrap();
    doc["settings"]["title"] = "alpha-2".into();
    h.put_project_json(doc, false, None).unwrap();
    let on_disk: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&a).unwrap()).unwrap();
    assert_eq!(on_disk["settings"]["title"], "alpha-2");
    assert!(std::fs::read_to_string(&a).unwrap().ends_with('\n'), "file ends with a newline");
    svc.kill();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn opening_a_missing_path_starts_a_new_project_there() {
    let root = scratch("newproj");
    let svc = Service::start(&root.join("xdg"), None);
    let h = svc.connect();
    let fresh = root.join("sub").join("new.json");
    h.open_project(&fresh.display().to_string(), false).unwrap();
    let st = h.status().unwrap();
    assert!(st.project_path.ends_with("sub/new.json"), "{}", st.project_path);
    assert_eq!(h.get_project_json().unwrap()["version"], 7, "stub document");
    assert!(!fresh.exists(), "nothing written until the UI saves");
    h.put_project_json(serde_json::json!({"version": 11}), false, None).unwrap();
    assert!(fresh.is_file(), "first save creates folder and file");
    svc.kill();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn malformed_project_is_refused_and_current_one_kept() {
    let root = scratch("malformed");
    let svc = Service::start(&root.join("xdg"), None);
    let h = svc.connect();
    let good = root.join("good.json");
    write_project(&good, "good");
    h.open_project(&good.display().to_string(), false).unwrap();

    for (name, text) in [("broken.json", "{\"version\": 11, \"hubs\": "), ("array.json", "[1,2]")] {
        let bad = root.join(name);
        std::fs::write(&bad, text).unwrap();
        let err = h.open_project(&bad.display().to_string(), false).unwrap_err();
        assert!(err.contains(name), "error names the file: {err}");
        let st = h.status().unwrap();
        assert!(same(&st.project_path, &good), "still on good.json after {name}: {}", st.project_path);
        assert_eq!(h.get_project_json().unwrap()["settings"]["title"], "good");
        // A save now must go to good.json and leave the broken file alone.
        h.put_project_json(h.get_project_json().unwrap(), false, None).unwrap();
        assert_eq!(std::fs::read_to_string(&bad).unwrap(), text, "{name} untouched");
    }
    svc.kill();
    // And the restart marker still names the good project.
    let svc = Service::start(&root.join("xdg"), None);
    assert!(same(&svc.connect().status().unwrap().project_path, &good));
    svc.kill();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn status_and_snapshot_answer_on_an_idle_service() {
    let root = scratch("idle");
    let svc = Service::start(&root.join("xdg"), None);
    let h = svc.connect();
    let st = h.status().unwrap();
    assert!(st.running_hubs().is_empty(), "no packs, nothing live: {:?}", st.running_hubs());
    h.ping().unwrap();
    h.stop_all_hubs().unwrap();
    let snap = h.snapshot();
    assert_eq!(snap.point_count, snap.points.len());
    svc.kill();
    let _ = std::fs::remove_dir_all(&root);
}
