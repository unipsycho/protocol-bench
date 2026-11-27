//! Project loading: every protocol section survives file → panels → file (what the UI does on
//! open and autosave), old / partial / malformed files, layout seeding and point bindings.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::files_panel::FilesPanelState;
use crate::http_panel::HttpPanelState;
use crate::las_panel::LasPanelState;
use crate::modbus_panel::ModbusPanelState;
use crate::mqtt_panel::MqttPanelState;
use crate::opcua_panel::OpcUaPanelState;
use crate::persist::UiState;
use crate::project::*;
use crate::snmp_panel::SnmpPanelState;
use crate::streams_panel::StreamsPanelState;

/// The UI's open path for the protocol sections: apply each to a fresh panel, read it back.
fn through_panels(file: &ProjectFile) -> ProjectFile {
    let mut mqtt = MqttPanelState::default();
    let mut opcua = OpcUaPanelState::default();
    let mut http = HttpPanelState::default();
    let mut streams = StreamsPanelState::default();
    let mut las = LasPanelState::default();
    let mut snmp = SnmpPanelState::default();
    let mut modbus = ModbusPanelState::default();
    let mut files = FilesPanelState::default();
    let mut ext = crate::ext_proto::ExtPanels::default();
    file.mqtt.apply_to_panel(&mut mqtt);
    file.opcua.apply_to_panel(&mut opcua);
    file.http.apply_to_panel(&mut http);
    file.streams.apply_to_panel(&mut streams);
    file.las.apply_to_panel(&mut las);
    file.snmp.apply_to_panel(&mut snmp);
    file.modbus.apply_to_panel(&mut modbus);
    file.files.apply_to_panel(&mut files);
    file.ext.apply_to_panels(&mut ext);
    ProjectFile {
        mqtt: MqttProject::from_panel(&mqtt),
        opcua: OpcUaProject::from_panel(&opcua),
        http: HttpProject::from_panel(&http),
        streams: StreamsProject::from_panel(&streams),
        las: LasProject::from_panel(&las),
        snmp: SnmpProject::from_panel(&snmp),
        modbus: ModbusProject::from_panel(&modbus),
        files: FilesProject::from_panel(&files),
        ext: crate::ext_proto::ExtSections::from_panels(&ext),
        ..file.clone()
    }
}

/// Section-by-section comparison so a failure names the protocol that lost data.
fn assert_sections_eq(name: &str, a: &ProjectFile, b: &ProjectFile) {
    let ja = serde_json::to_value(a).unwrap();
    let jb = serde_json::to_value(b).unwrap();
    let keys = [
        "mqtt", "opcua", "http", "streams", "las", "snmp", "modbus", "files", "wits", "websocket",
        "sparkplug", "dnp3", "can", "gen", "hubs", "points", "settings", "layout", "next_point_seq",
    ];
    for key in keys {
        assert_eq!(ja.get(key), jb.get(key), "{name}: section `{key}` changed on load/save");
    }
    assert_eq!(ja, jb, "{name}: whole document changed");
}

fn demo_projects() -> Vec<(&'static str, ProjectFile)> {
    vec![
        ("frac_spread", crate::demos::frac_spread("/tmp/pb-data")),
        ("bottling_line", crate::demos::bottling_line()),
        ("water_lift_station", crate::demos::water_lift_station()),
        ("generator_plant", crate::demos::generator_plant()),
        ("default", ProjectFile::default()),
    ]
}

fn sample_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../protocol-bench-projects")
}

fn sample_projects() -> Vec<(String, ProjectFile)> {
    let Ok(entries) = std::fs::read_dir(sample_dir()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let file: ProjectFile = serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        out.push((path.file_name().unwrap().to_string_lossy().into_owned(), file));
    }
    out
}

#[test]
fn demo_projects_survive_open_and_save_unchanged() {
    for (name, file) in demo_projects() {
        let back = through_panels(&file);
        assert_sections_eq(name, &file, &back);
    }
}

#[test]
fn sample_projects_survive_open_and_save() {
    for (name, file) in sample_projects() {
        // First pass may normalise (defaults filled, legacy fields migrated); after that the
        // panels must be a fixed point or autosave would rewrite the file every frame.
        let once = through_panels(&file);
        let twice = through_panels(&once);
        assert_sections_eq(&name, &once, &twice);
        // No connection may be lost on the way.
        let count = |f: &ProjectFile| {
            f.mqtt.clients.len()
                + f.modbus.connections.len()
                + f.ext.wits.connections.len()
                + f.ext.websocket.connections.len()
                + f.ext.sparkplug.connections.len()
                + f.ext.dnp3.connections.len()
                + f.ext.can.connections.len()
                + f.ext.gen.connections.len()
        };
        assert_eq!(count(&file), count(&once), "{name}: connections dropped");
    }
}

#[test]
fn file_text_roundtrip_is_lossless() {
    for (name, file) in demo_projects() {
        let text = serde_json::to_string_pretty(&file).unwrap();
        let back: ProjectFile = serde_json::from_str(&text).unwrap();
        // Compare what is saved: transient UI state (hub selection) is not written on purpose.
        let again: Value = serde_json::to_value(&back).unwrap();
        let first: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(first, again, "{name}");
    }
}

#[test]
fn empty_object_loads_with_defaults() {
    let file: ProjectFile = serde_json::from_str("{}").unwrap();
    assert_eq!(file.version, PROJECT_VERSION);
    assert_eq!(file.next_point_seq, 1);
    assert!(file.points.is_empty());
    assert!(file.hubs.is_none(), "no hubs key keeps ui.toml hubs");
    assert!(file.layout.is_none(), "no layout key seeds from ui.toml");
    assert_eq!(file, ProjectFile { version: PROJECT_VERSION, ..ProjectFile::default() });
}

#[test]
fn service_new_project_stub_loads() {
    // pb-service hands out `{ "version": 7 }` for a path that does not exist yet.
    let file: ProjectFile = serde_json::from_value(json!({ "version": 7 })).unwrap();
    assert_eq!(file.version, 7);
    let back = through_panels(&file);
    assert_sections_eq("stub", &file, &back);
}

#[test]
fn unknown_keys_from_newer_builds_are_ignored() {
    let file: Result<ProjectFile, _> = serde_json::from_value(json!({
        "version": 99,
        "future_protocol": { "connections": [{ "id": "x" }] },
        "settings": { "name": "Later", "colour": "teal" }
    }));
    let file = file.expect("newer files still open");
    assert_eq!(file.settings.name, "Later");
}

#[test]
fn malformed_projects_are_rejected_not_defaulted() {
    for (label, text) in [
        ("truncated", r#"{"version": 11, "mqtt": {"#),
        ("not an object", "[1,2,3]"),
        ("wrong section type", r#"{"version": 11, "modbus": 5}"#),
        ("wrong version type", r#"{"version": "eleven"}"#),
        ("points not a list", r#"{"points": {"id": "a"}}"#),
    ] {
        assert!(serde_json::from_str::<ProjectFile>(text).is_err(), "{label} must fail to load");
    }
}

#[test]
fn layout_seeds_once_then_applies() {
    let mut ui = UiState::default();
    ui.sanitize();
    let mut file = ProjectFile::default();
    // No layout in the project: keep ui.toml layout and ask for a seed save.
    let before = ui.tree.clone();
    assert!(apply_or_seed_layout(&file, &mut ui));
    assert_eq!(ui.tree, before);
    // Seeded layout with a hidden pane is restored on the next open.
    let mut shaped = ui.clone();
    let pane = crate::dock::DockPane::ALL[1];
    assert!(shaped.toggle_pane_visible(pane));
    file.layout = Some(DockLayout::from_ui(&shaped));
    let mut fresh = UiState::default();
    fresh.sanitize();
    assert!(!apply_or_seed_layout(&file, &mut fresh));
    assert!(!fresh.pane_visible(pane), "hidden pane restored from project");
    assert_eq!(DockLayout::from_ui(&fresh), DockLayout::from_ui(&shaped));
}

#[test]
fn layout_with_retired_pane_is_sanitized_on_apply() {
    let mut ui = UiState::default();
    ui.sanitize();
    let mut layout = DockLayout::from_ui(&ui);
    layout.maximized = Some(crate::dock::DockPane::Calculations);
    layout.hidden_panes.push(crate::dock::DockPane::Calculations);
    let file = ProjectFile { layout: Some(layout), ..ProjectFile::default() };
    let mut target = UiState::default();
    apply_or_seed_layout(&file, &mut target);
    assert_eq!(target.maximized, None, "retired pane cannot stay maximized");
    assert!(!target.hidden_panes.contains(&crate::dock::DockPane::Calculations));
    for pane in crate::dock::DockPane::ALL {
        assert!(target.tree.contains_pane(pane), "{pane:?} restored");
    }
}

#[test]
fn point_bindings_import_and_export_roundtrip() {
    let bindings = vec![
        PointBindingSerde { id: "mqtt_1".into(), source: "mqtt/local/plant/temp".into() },
        PointBindingSerde { id: "mb_4".into(), source: "modbus/plc::hr:40001".into() },
    ];
    let mut store = pb_points::PointStore::new();
    import_bindings_to_store(&mut store, &bindings, 9);
    let mut back = bindings_from_store(&store);
    back.sort_by(|a, b| a.id.cmp(&b.id));
    let mut want = bindings.clone();
    want.sort_by(|a, b| a.id.cmp(&b.id));
    assert_eq!(back, want);
    assert!(store.next_seq() >= 9, "next id never reuses a saved one");
    // A zero sequence from a hand-edited file is clamped, not honoured.
    let mut store = pb_points::PointStore::new();
    import_bindings_to_store(&mut store, &[], 0);
    assert!(store.next_seq() >= 1);
}

#[test]
fn hub_binds_materialize_identically_on_every_load() {
    // Loading the same project twice must hand out the same point ids (no drift between runs).
    for (name, file) in demo_projects() {
        let Some(hubs) = file.hubs.clone() else { continue };
        let ids = |mut hubs: crate::hubs::HubModel| {
            let mut store = pb_points::PointStore::new();
            import_bindings_to_store(&mut store, &file.points, file.next_point_seq);
            materialize_hub_point_binds(&mut hubs, &mut store);
            let mut b = bindings_from_store(&store);
            b.sort_by(|a, b| a.id.cmp(&b.id));
            b
        };
        assert_eq!(ids(hubs.clone()), ids(hubs), "{name}");
    }
}

/// A project with every protocol section present but empty, as a user who deleted the
/// sample connections would save it.
fn sparse_project() -> ProjectFile {
    serde_json::from_value(json!({
        "version": PROJECT_VERSION,
        "mqtt": { "clients": [], "selected_id": "" },
        "opcua": { "clients": [], "selected_client_id": "" },
        "http": { "clients": [], "selected_id": "" },
        "streams": { "endpoints": [], "selected_id": "" },
        "las": { "endpoints": [], "selected_id": "" },
        "snmp": { "targets": [], "selected_id": "" },
        "modbus": { "connections": [], "selected_id": "" },
        "files": { "sources": [], "selected_id": "" },
        "wits": { "connections": [] },
        "websocket": { "connections": [] },
        "sparkplug": { "connections": [] },
        "dnp3": { "connections": [] },
        "can": { "connections": [] },
        "gen": { "connections": [] },
        "hubs": { "hubs": [] }
    }))
    .expect("sparse project parses")
}

#[test]
fn empty_sections_stay_empty_after_open_and_save() {
    // Regression: panels used to insert a stub connection (an enabled Modbus client on
    // 127.0.0.1, a /dev/ttyUSB0 serial port, …) into an empty list, and autosave wrote it back.
    let file = sparse_project();
    let back = through_panels(&file);
    assert_sections_eq("sparse", &file, &back);
}

#[test]
fn stale_selection_is_dropped_on_open() {
    let mut file = sparse_project();
    file.modbus.selected_id = "gone".into();
    file.las.selected_id = "gone".into();
    let back = through_panels(&file);
    assert_eq!(back.modbus.selected_id, "");
    assert_eq!(back.las.selected_id, "");
}

#[test]
fn blank_project_has_nothing_and_survives_open_and_save() {
    let file = ProjectFile::blank("  Pump skid  ");
    assert_eq!(file.settings.name, "Pump skid");
    let v = serde_json::to_value(&file).unwrap();
    for (sec, key) in [
        ("mqtt", "clients"), ("opcua", "clients"), ("http", "clients"), ("streams", "endpoints"),
        ("las", "endpoints"), ("snmp", "targets"), ("modbus", "connections"), ("files", "sources"),
        ("wits", "connections"), ("websocket", "connections"), ("sparkplug", "connections"),
        ("dnp3", "connections"), ("can", "connections"), ("gen", "connections"),
    ] {
        let n = v[sec][key].as_array().map_or(0, |a| a.len());
        assert_eq!(n, 0, "blank project has no {sec} {key}");
    }
    let hubs = file.hubs.as_ref().expect("hubs present so ui.toml sample hubs are not used");
    assert!(hubs.hubs.is_empty() && hubs.feeds.is_empty());
    assert!(!file.mqtt.embedded.enabled || file.mqtt.embedded.port > 0);
    // Written and read back as the UI would.
    let text = serde_json::to_string_pretty(&file).unwrap();
    let back: ProjectFile = serde_json::from_str(&text).unwrap();
    assert_sections_eq("blank", &file, &through_panels(&back));
    // A blank project materializes no points.
    let mut store = pb_points::PointStore::new();
    let mut hubs = back.hubs.clone().unwrap();
    materialize_hub_point_binds(&mut hubs, &mut store);
    assert!(bindings_from_store(&store).is_empty());
}

#[test]
fn empty_hub_model_survives_sanitize_and_adds_from_id_one() {
    let mut m = crate::hubs::HubModel::empty();
    m.sanitize();
    assert!(m.hubs.is_empty(), "sanitize must not bring the sample back");
    m.add_hub();
    assert_eq!(m.hubs.len(), 1);
}
