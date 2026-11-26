//! End-to-end: the service remembers the project it had open and reopens it after a hard kill.
//! Runs the real `pb-service` binary against a throwaway config dir (`XDG_CONFIG_HOME`), so it
//! never touches the developer's own service or project.

mod common;
use common::*;

#[test]
fn killed_service_reopens_last_project() {
    let root = scratch("reopen");
    let xdg = root.join("xdg");
    let plant = root.join("plant.json");
    write_project(&plant, "plant");

    // First run: default project, then the UI opens plant.json.
    let svc = Service::start(&xdg, None);
    let h = svc.connect();
    let st = h.status().unwrap();
    assert!(st.project_path.ends_with("project.json"), "starts on default: {}", st.project_path);
    h.open_project(&plant.display().to_string(), false).unwrap();
    assert!(same(&h.status().unwrap().project_path, &plant));
    drop(h);
    svc.kill();

    // Second run after a hard kill: comes back on plant.json, not the default.
    let svc = Service::start(&xdg, None);
    let h = svc.connect();
    let st = h.status().unwrap();
    assert!(same(&st.project_path, &plant), "reopened {}", st.project_path);
    let doc = h.get_project_json().unwrap();
    assert_eq!(doc["settings"]["title"], "plant", "project content applied");
    drop(h);
    svc.kill();

    // PB_PROJECT still wins over the remembered project.
    let other = root.join("other.json");
    write_project(&other, "other");
    let svc = Service::start(&xdg, Some(&other));
    let st = svc.connect().status().unwrap();
    assert!(same(&st.project_path, &other), "env wins: {}", st.project_path);
    svc.kill();

    // The remembered project was deleted: fall back to the default instead of failing.
    std::fs::remove_file(&plant).unwrap();
    let svc = Service::start(&xdg, None);
    let st = svc.connect().status().unwrap();
    assert!(st.project_path.ends_with("protocolbench/project.json"), "fallback: {}", st.project_path);
    svc.kill();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn save_as_updates_the_remembered_project() {
    let root = scratch("saveas");
    let xdg = root.join("xdg");
    let svc = Service::start(&xdg, None);
    let h = svc.connect();
    let target = root.join("saved-as.json");
    let doc = serde_json::json!({ "version": 11, "hubs": { "hubs": [] } });
    h.put_project_json(doc, false, Some(target.display().to_string())).unwrap();
    assert!(target.is_file(), "save as writes the file");
    drop(h);
    svc.kill();

    let svc = Service::start(&xdg, None);
    let st = svc.connect().status().unwrap();
    assert!(same(&st.project_path, &target), "reopened {}", st.project_path);
    svc.kill();
    let _ = std::fs::remove_dir_all(&root);
}
