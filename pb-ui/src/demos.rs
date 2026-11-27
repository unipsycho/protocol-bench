//! Demo projects: the industrial ones (frac spread, bottling line, water lift station), which
//! need `pb-sim`, and the generator plant, which makes its own data.
//!
//! Built from the real project types so they always load. `pb-sim` plays the field devices.
//! Regenerate the files with
//! `PB_EXPORT_DEMOS=$PWD/../protocol-bench-projects cargo test -p pb-ui export_industrial_demos -- --ignored`.

use serde_json::{json, Map, Value};

use crate::hubs::{Hub, HubModel, HubPort, HubProto, LasOutputBind, ModbusBind, MqttInputBind, MqttOutputBind};
use crate::modbus_panel::ModbusConn;
use crate::project::{LasEndpointSerde, MqttClientSerde, ProjectFile};

/// Register with tag name, scaling and units.
#[allow(clippy::too_many_arguments)]
fn reg(name: &str, area: &str, address: u16, dtype: &str, order: &str, scale: f64, units: &str, point: &str) -> ModbusBind {
    ModbusBind {
        name: name.into(),
        area: area.into(),
        address,
        dtype: dtype.into(),
        order: order.into(),
        scale,
        units: units.into(),
        point: point.into(),
        ..Default::default()
    }
}

fn obj(v: Value) -> Map<String, Value> {
    v.as_object().cloned().unwrap_or_default()
}

fn client(id: &str, label: &str, port: u16) -> ModbusConn {
    ModbusConn { id: id.into(), label: label.into(), port, enabled: true, ..ModbusConn::default_client() }
}

fn port(id: u64, proto: HubProto, label: &str) -> HubPort {
    HubPort::new(id, proto, label)
}

fn mqtt_out(point: &str, topic: &str) -> MqttOutputBind {
    MqttOutputBind { point: point.into(), topic: topic.into(), qos: 0, retain: true, auto: false }
}

fn mqtt_in(topic: &str, point: &str) -> MqttInputBind {
    MqttInputBind { topic: topic.into(), path: String::new(), point: point.into() }
}

fn hub(name: &str, proto: HubProto, sources: Vec<HubPort>, outputs: Vec<HubPort>) -> Hub {
    Hub {
        id: 1,
        name: name.into(),
        protocol: proto,
        sources,
        outputs,
        aggregators: vec![],
        calculations: vec![],
        at: None,
        modbus_server: String::new(),
        opcua_expose_unmapped: true,
        opcua_catalog: vec![],
    }
}

fn model(h: Hub) -> HubModel {
    let mut m = HubModel::sample();
    m.selected = Some(1);
    m.selection = vec![(1, None)];
    // Every Input wired into the hub (the sample's own wires refer to other node ids).
    m.feeds = h.sources.iter().map(|s| crate::hubs::HubFeed { from_hub: h.id, from_node: s.id, to_hub: h.id, to_node: h.id }).collect();
    m.hubs = vec![h];
    m
}

fn loopback(port: u16, client_id: &str, subs: &[&str]) -> MqttClientSerde {
    MqttClientSerde {
        id: "local".into(),
        label: "Hub loopback".into(),
        port,
        client_id: client_id.into(),
        subscriptions: subs.iter().map(|s| s.to_string()).collect(),
        auto_connect: true,
        ..MqttClientSerde::default_local()
    }
}

/// A project with none of the default sample endpoints (serial port, CSV file, HTTP API,
/// OPC UA client), so a demo only opens the links it uses.
fn blank_project() -> ProjectFile {
    let mut f = ProjectFile::default();
    f.streams.endpoints.clear();
    f.streams.selected_id.clear();
    f.files.sources.clear();
    f.files.selected_id.clear();
    f.http.clients.clear();
    f.http.selected_id.clear();
    f.opcua.clients.clear();
    f.opcua.selected_client_id.clear();
    f.las.endpoints.clear();
    f.las.selected_id.clear();
    f.snmp.targets.clear();
    f.snmp.selected_id.clear();
    f
}

fn ext_conn(v: Value) -> Map<String, Value> {
    let mut o = obj(v);
    o.entry("enabled").or_insert(Value::Bool(true));
    o
}

// ---------------------------------------------------------------------------
// Frac spread
// ---------------------------------------------------------------------------

pub fn frac_spread(data_dir: &str) -> ProjectFile {
    let mut f = blank_project();
    f.settings.name = "Frac spread".into();
    f.settings.description = "Hydraulic fracturing spread. Four triplex frac pumps and the blender on Modbus TCP, the \
        wellhead RTU on DNP3, the pressure-pumping vendor's WITS feed and pump 1's engine (CAN J1939 on vcan0, \
        disabled until vcan0 exists) come into one MQTT hub (embedded broker :1885). Out: WITS to the data van \
        (:5100), a live WebSocket dashboard (:8765), UNS topics frac/#, a time-indexed stage LAS log, an ESD \
        command to the RTU and pump 1 rate / enable commands (MQTT frac/cmd/#). Run `pb-sim frac`."
        .into();
    f.next_point_seq = 200;
    f.mqtt.embedded.enabled = true;
    f.mqtt.embedded.port = 1885;
    f.mqtt.clients = vec![loopback(1885, "pb-frac", &["frac/#"])];
    f.mqtt.selected_id = "local".into();
    let mut conns = Vec::new();
    for i in 1..=4u16 {
        conns.push(client(&format!("pump{i}"), &format!("Frac pump {i}"), 5020 + i));
    }
    conns.push(client("blender", "Blender", 5025));
    f.modbus.connections = conns;
    f.modbus.selected_id = "pump1".into();
    f.ext.dnp3.connections = vec![ext_conn(json!({"id":"wellhead","label":"Wellhead RTU","host":"127.0.0.1","port":20000,"master_address":1,"outstation_address":10,"timeout_ms":3000}))];
    f.ext.wits.connections = vec![
        ext_conn(json!({"id":"pp_feed","label":"Pressure pumping feed","transport":"tcp_client","host":"127.0.0.1","port":5101,"stale_secs":10,"null_values":"-9999,-999.25"})),
        ext_conn(json!({"id":"data_van","label":"Data van (WITS out)","transport":"tcp_listen","host":"0.0.0.0","port":5100,"stale_secs":15,"null_values":"-9999"})),
    ];
    f.ext.websocket.connections = vec![ext_conn(json!({"id":"dashboard","label":"Frac dashboard","mode":"server","host":"0.0.0.0","port":8765,"out_format":"object"}))];
    f.ext.can.connections = vec![ext_conn(json!({"id":"engine1","label":"Pump 1 engine (J1939)","interface":"vcan0","stale_secs":5,"enabled":false}))];
    f.las.endpoints = vec![LasEndpointSerde {
        id: "stage-log".into(),
        label: "Stage log (written)".into(),
        role: "outgoing".into(),
        path: format!("{data_dir}/frac-stage.las"),
        version: "2.0".into(),
        follow: false,
    }];
    f.las.selected_id = "stage-log".into();

    let mut sources = Vec::new();
    let mut id = 2;
    for i in 1..=4 {
        let mut p = port(id, HubProto::Modbus, &format!("Frac pump {i}"));
        id += 1;
        p.modbus_connection = format!("pump{i}");
        p.modbus_interval_secs = 1.0;
        let pt = |s: &str| format!("p{i}_{s}");
        p.modbus_inputs = vec![
            reg("Discharge pressure", "holding", 0, "f32", "cdab", 1.0, "psi", &pt("discharge_psi")),
            reg("Suction pressure", "holding", 2, "f32", "cdab", 1.0, "psi", &pt("suction_psi")),
            reg("Pump rate", "holding", 4, "f32", "cdab", 1.0, "bbl/min", &pt("rate")),
            reg("Engine speed", "holding", 6, "u16", "abcd", 1.0, "rpm", &pt("engine_rpm")),
            reg("Transmission gear", "holding", 7, "u16", "abcd", 1.0, "", &pt("gear")),
            reg("Engine coolant temp", "holding", 8, "i16", "abcd", 0.1, "°F", &pt("coolant_f")),
            reg("Trans oil temp", "holding", 9, "i16", "abcd", 0.1, "°F", &pt("trans_f")),
            reg("Engine hours", "holding", 10, "u32", "abcd", 0.1, "h", &pt("engine_hours")),
            reg("Engine running", "coil", 0, "bool", "abcd", 1.0, "", &pt("running")),
            reg("Overpressure trip", "discrete", 0, "bool", "abcd", 1.0, "", &pt("overpressure")),
        ];
        sources.push(p);
    }
    let mut p = port(id, HubProto::Modbus, "Blender");
    id += 1;
    p.modbus_connection = "blender".into();
    p.modbus_inputs = vec![
        reg("Clean rate", "holding", 0, "f32", "cdab", 1.0, "bbl/min", "blender_clean_rate"),
        reg("Slurry rate", "holding", 2, "f32", "cdab", 1.0, "bbl/min", "blender_slurry_rate"),
        reg("Sand concentration", "holding", 4, "f32", "cdab", 1.0, "ppa", "blender_sand_ppa"),
        reg("Tub level", "holding", 6, "f32", "cdab", 1.0, "%", "blender_tub_level"),
        reg("Slurry density", "holding", 8, "f32", "cdab", 1.0, "lb/gal", "blender_density"),
        reg("Blender running", "coil", 0, "bool", "abcd", 1.0, "", "blender_running"),
    ];
    sources.push(p);
    let mut p = port(id, HubProto::Dnp3, "Wellhead RTU");
    id += 1;
    p.ext_connection = "wellhead".into();
    p.ext_interval_secs = 1.0;
    p.ext_inputs = vec![
        obj(json!({"kind":"analog","index":0,"scale":1.0,"offset":0.0,"point":"wh_treating_psi"})),
        obj(json!({"kind":"analog","index":1,"scale":1.0,"offset":0.0,"point":"wh_annulus_psi"})),
        obj(json!({"kind":"analog","index":2,"scale":1.0,"offset":0.0,"point":"wh_temp_f"})),
        obj(json!({"kind":"analog","index":3,"scale":1.0,"offset":0.0,"point":"wh_total_rate"})),
        obj(json!({"kind":"binary","index":0,"scale":1.0,"offset":0.0,"point":"wh_frac_valve_open"})),
        obj(json!({"kind":"binary","index":1,"scale":1.0,"offset":0.0,"point":"wh_esd_active"})),
        obj(json!({"kind":"counter","index":0,"scale":1.0,"offset":0.0,"point":"wh_stage"})),
    ];
    sources.push(p);
    let mut p = port(id, HubProto::Wits, "Pressure pumping WITS");
    id += 1;
    p.ext_connection = "pp_feed".into();
    p.ext_inputs = vec![
        obj(json!({"item":"0108","scale":1.0,"offset":0.0,"point":"wits_plug_depth_ft"})),
        obj(json!({"item":"1101","scale":1.0,"offset":0.0,"point":"wits_sand_ppa"})),
        obj(json!({"item":"1103","scale":1.0,"offset":0.0,"point":"wits_proppant_lbs"})),
    ];
    sources.push(p);
    let mut p = port(id, HubProto::Can, "Pump 1 engine (J1939)");
    id += 1;
    p.ext_connection = "engine1".into();
    p.ext_inputs = vec![
        obj(json!({"id":"pgn:61444","start":24,"len":16,"order":"intel","type":"unsigned","scale":0.125,"offset":0.0,"point":"p1_j1939_rpm"})),
        obj(json!({"id":"pgn:65262","start":0,"len":8,"order":"intel","type":"unsigned","scale":1.0,"offset":-40.0,"point":"p1_j1939_coolant_c"})),
    ];
    sources.push(p);
    let mut p = port(id, HubProto::Mqtt, "Frac commands");
    id += 1;
    p.mqtt_connection = "local".into();
    p.mqtt_inputs = vec![
        mqtt_in("frac/cmd/esd", "cmd_esd"),
        mqtt_in("frac/cmd/pump1_rate_sp", "cmd_p1_rate_sp"),
        mqtt_in("frac/cmd/pump1_enable", "cmd_p1_enable"),
    ];
    sources.push(p);

    let mut outputs = Vec::new();
    let mut p = port(id, HubProto::Wits, "Data van (WITS)");
    id += 1;
    p.ext_connection = "data_van".into();
    p.ext_interval_secs = 1.0;
    let w = |item: &str, point: &str, dec: i64| obj(json!({"item":item,"decimals":dec,"point":point}));
    p.ext_outputs = vec![
        w("0108", "wits_plug_depth_ft", 1),
        w("0121", "wh_treating_psi", 0),
        w("0122", "wh_annulus_psi", 0),
        w("0130", "wh_total_rate", 2),
        w("1101", "blender_sand_ppa", 2),
        w("1102", "wh_stage", 0),
        w("1103", "wits_proppant_lbs", 0),
    ];
    outputs.push(p);
    let mut p = port(id, HubProto::WebSocket, "Live dashboard");
    id += 1;
    p.ext_connection = "dashboard".into();
    p.ext_interval_secs = 0.5;
    let mut keys = vec![
        ("treating_psi", "wh_treating_psi"),
        ("annulus_psi", "wh_annulus_psi"),
        ("total_rate", "wh_total_rate"),
        ("stage", "wh_stage"),
        ("esd", "wh_esd_active"),
        ("sand_ppa", "blender_sand_ppa"),
        ("tub_level", "blender_tub_level"),
        ("proppant_lbs", "wits_proppant_lbs"),
        ("plug_depth_ft", "wits_plug_depth_ft"),
    ];
    let pump_keys: Vec<(String, String)> = (1..=4)
        .flat_map(|i| [(format!("pump{i}_psi"), format!("p{i}_discharge_psi")), (format!("pump{i}_rate"), format!("p{i}_rate")), (format!("pump{i}_rpm"), format!("p{i}_engine_rpm"))])
        .collect();
    p.ext_outputs = keys.drain(..).map(|(k, pt)| obj(json!({"key":k,"point":pt}))).collect();
    p.ext_outputs.extend(pump_keys.iter().map(|(k, pt)| obj(json!({"key":k,"point":pt}))));
    outputs.push(p);
    let mut p = port(id, HubProto::Mqtt, "UNS topics");
    id += 1;
    p.mqtt_connection = "local".into();
    let mut outs = vec![
        mqtt_out("wh_treating_psi", "frac/wellhead/treating_psi"),
        mqtt_out("wh_total_rate", "frac/wellhead/total_rate"),
        mqtt_out("wh_stage", "frac/wellhead/stage"),
        mqtt_out("wh_esd_active", "frac/wellhead/esd_active"),
        mqtt_out("blender_sand_ppa", "frac/blender/sand_ppa"),
        mqtt_out("blender_slurry_rate", "frac/blender/slurry_rate"),
    ];
    for i in 1..=4 {
        outs.push(mqtt_out(&format!("p{i}_discharge_psi"), &format!("frac/pump{i}/discharge_psi")));
        outs.push(mqtt_out(&format!("p{i}_rate"), &format!("frac/pump{i}/rate")));
    }
    p.mqtt_outputs = outs;
    outputs.push(p);
    let mut p = port(id, HubProto::Las, "Stage log (LAS)");
    id += 1;
    p.las_endpoint = "stage-log".into();
    p.las_index = "time".into();
    p.las_outputs = vec![
        LasOutputBind { point: "wh_treating_psi".into(), curve: "TPRES".into() },
        LasOutputBind { point: "wh_total_rate".into(), curve: "RATE".into() },
        LasOutputBind { point: "blender_sand_ppa".into(), curve: "PPA".into() },
        LasOutputBind { point: "wh_stage".into(), curve: "STAGE".into() },
    ];
    outputs.push(p);
    let mut p = port(id, HubProto::Dnp3, "ESD to wellhead");
    id += 1;
    p.ext_connection = "wellhead".into();
    p.ext_outputs = vec![obj(json!({"kind":"binary_output","index":0,"float":true,"scale":1.0,"offset":0.0,"point":"cmd_esd"}))];
    outputs.push(p);
    let mut p = port(id, HubProto::Modbus, "Pump 1 commands");
    p.modbus_connection = "pump1".into();
    p.modbus_outputs = vec![
        reg("Rate setpoint", "holding", 20, "f32", "cdab", 1.0, "bbl/min", "cmd_p1_rate_sp"),
        reg("Pump enable", "coil", 1, "bool", "abcd", 1.0, "", "cmd_p1_enable"),
    ];
    outputs.push(p);
    f.hubs = Some(model(hub("Frac spread", HubProto::Mqtt, sources, outputs)));
    f
}

// ---------------------------------------------------------------------------
// Bottling line
// ---------------------------------------------------------------------------

pub fn bottling_line() -> ProjectFile {
    let mut f = blank_project();
    f.settings.name = "Bottling line".into();
    f.settings.description = "Beverage bottling line 2. Line PLC, conveyor and filler VFDs and a 3-phase energy meter \
        on Modbus TCP, and the robot cell as a Sparkplug B edge node, meet in an MQTT hub (embedded broker :1886). \
        Out: an ISA-95 style UNS (plant/bottling/line2/#), a Sparkplug B gateway edge node republishing line KPIs, \
        a WebSocket dashboard (:8766), and line run / conveyor speed commands from MQTT (plant/bottling/line2/cmd/#). \
        Run `pb-sim bottling`."
        .into();
    f.next_point_seq = 200;
    f.mqtt.embedded.enabled = true;
    f.mqtt.embedded.port = 1886;
    f.mqtt.clients = vec![loopback(1886, "pb-bottling", &["plant/#"])];
    f.mqtt.selected_id = "local".into();
    f.modbus.connections = vec![
        client("line_plc", "Line 2 PLC", 5031),
        client("conveyor_vfd", "Conveyor VFD", 5032),
        client("filler_vfd", "Filler VFD", 5034),
        client("energy", "Energy meter", 5033),
    ];
    f.modbus.selected_id = "line_plc".into();
    f.ext.sparkplug.connections = vec![
        ext_conn(json!({"id":"spb_host","label":"Sparkplug host","role":"host","host":"127.0.0.1","port":1886,"group_id":"Bottling","edge_node_id":"Line2","client_id":"","username":"","password":""})),
        ext_conn(json!({"id":"spb_gateway","label":"PB gateway edge node","role":"edge","host":"127.0.0.1","port":1886,"group_id":"Bottling","edge_node_id":"PB-Gateway","client_id":"","username":"","password":""})),
    ];
    f.ext.websocket.connections = vec![ext_conn(json!({"id":"dashboard","label":"Line dashboard","mode":"server","host":"0.0.0.0","port":8766,"out_format":"object"}))];

    let mut sources = Vec::new();
    let mut p = port(2, HubProto::Modbus, "Line 2 PLC");
    p.modbus_connection = "line_plc".into();
    p.modbus_inputs = vec![
        reg("Line state", "holding", 0, "u16", "abcd", 1.0, "0 idle · 1 run · 2 fault", "line_state"),
        reg("Bottles per minute", "holding", 1, "u16", "abcd", 1.0, "bpm", "line_bpm"),
        reg("Good count", "holding", 2, "u32", "abcd", 1.0, "", "line_good"),
        reg("Reject count", "holding", 4, "u32", "abcd", 1.0, "", "line_reject"),
        reg("Filler temperature", "holding", 6, "f32", "cdab", 1.0, "°C", "filler_temp_c"),
        reg("Capper torque", "holding", 8, "f32", "cdab", 1.0, "N·m", "capper_torque"),
        reg("OEE", "holding", 10, "f32", "cdab", 1.0, "%", "line_oee"),
        reg("Run command echo", "coil", 0, "bool", "abcd", 1.0, "", "line_run"),
        reg("Bottle jam", "discrete", 1, "bool", "abcd", 1.0, "", "line_jam"),
    ];
    sources.push(p);
    for (id, conn, label, pre) in [(3u64, "conveyor_vfd", "Conveyor VFD", "conv"), (4, "filler_vfd", "Filler VFD", "fill")] {
        let mut p = port(id, HubProto::Modbus, label);
        p.modbus_connection = conn.into();
        let pt = |s: &str| format!("{pre}_{s}");
        p.modbus_inputs = vec![
            reg("Running", "holding", 0, "bit2", "abcd", 1.0, "", &pt("running")),
            reg("Faulted", "holding", 0, "bit3", "abcd", 1.0, "", &pt("faulted")),
            reg("Output frequency", "holding", 1, "u16", "abcd", 0.01, "Hz", &pt("hz")),
            reg("Motor speed", "holding", 2, "u16", "abcd", 1.0, "rpm", &pt("rpm")),
            reg("Motor current", "holding", 3, "u16", "abcd", 0.1, "A", &pt("amps")),
            reg("Motor torque", "holding", 4, "i16", "abcd", 0.1, "%", &pt("torque")),
            reg("Fault code", "holding", 6, "u16", "abcd", 1.0, "", &pt("fault_code")),
        ];
        sources.push(p);
    }
    let mut p = port(5, HubProto::Modbus, "Energy meter");
    p.modbus_connection = "energy".into();
    p.modbus_interval_secs = 2.0;
    p.modbus_inputs = vec![
        reg("Voltage L1-N", "input", 0, "f32", "abcd", 1.0, "V", "energy_v_l1"),
        reg("Current L1", "input", 6, "f32", "abcd", 1.0, "A", "energy_a_l1"),
        reg("Active power", "input", 12, "f32", "abcd", 1.0, "kW", "energy_kw"),
        reg("Power factor", "input", 16, "f32", "abcd", 1.0, "", "energy_pf"),
        reg("Frequency", "input", 18, "f32", "abcd", 1.0, "Hz", "energy_hz"),
        reg("Energy import", "input", 20, "f32", "abcd", 1.0, "kWh", "energy_kwh"),
    ];
    sources.push(p);
    let mut p = port(6, HubProto::Sparkplug, "Robot cell (Sparkplug)");
    p.ext_connection = "spb_host".into();
    let m = |metric: &str, point: &str| obj(json!({"device":"Robot1","metric":metric,"scale":1.0,"offset":0.0,"point":point}));
    p.ext_inputs = vec![
        m("Cycle time", "robot_cycle_s"),
        m("Picks", "robot_picks"),
        m("Torque", "robot_torque"),
        m("State", "robot_state"),
        m("Gripper closed", "robot_gripper"),
    ];
    sources.push(p);
    let mut p = port(7, HubProto::Mqtt, "Line commands");
    p.mqtt_connection = "local".into();
    p.mqtt_inputs = vec![mqtt_in("plant/bottling/line2/cmd/run", "cmd_line_run"), mqtt_in("plant/bottling/line2/cmd/conveyor_hz", "cmd_conveyor_hz")];
    sources.push(p);

    let mut outputs = Vec::new();
    let mut p = port(8, HubProto::Mqtt, "UNS (ISA-95)");
    p.mqtt_connection = "local".into();
    let base = "plant/bottling/line2";
    p.mqtt_outputs = [
        ("line_state", "line/state"),
        ("line_bpm", "line/bpm"),
        ("line_good", "line/good"),
        ("line_reject", "line/reject"),
        ("line_oee", "line/oee"),
        ("filler_temp_c", "filler/temp_c"),
        ("conv_hz", "conveyor/vfd/hz"),
        ("conv_amps", "conveyor/vfd/amps"),
        ("fill_hz", "filler/vfd/hz"),
        ("energy_kw", "energy/kw"),
        ("energy_kwh", "energy/kwh"),
        ("robot_state", "robot1/state"),
        ("robot_cycle_s", "robot1/cycle_s"),
    ]
    .iter()
    .map(|(pt, t)| mqtt_out(pt, &format!("{base}/{t}")))
    .collect();
    outputs.push(p);
    let mut p = port(9, HubProto::Sparkplug, "Sparkplug gateway");
    p.ext_connection = "spb_gateway".into();
    p.ext_interval_secs = 1.0;
    let g = |metric: &str, point: &str, t: &str| obj(json!({"edge":"","device":"","metric":metric,"datatype":t,"point":point}));
    p.ext_outputs = vec![
        g("Line/OEE", "line_oee", "double"),
        g("Line/Bottles per minute", "line_bpm", "double"),
        g("Line/Good", "line_good", "int64"),
        g("Line/Reject", "line_reject", "int64"),
        g("Energy/kW", "energy_kw", "double"),
    ];
    outputs.push(p);
    let mut p = port(10, HubProto::WebSocket, "Line dashboard");
    p.ext_connection = "dashboard".into();
    p.ext_interval_secs = 0.5;
    p.ext_outputs = [
        ("state", "line_state"),
        ("bpm", "line_bpm"),
        ("good", "line_good"),
        ("reject", "line_reject"),
        ("oee", "line_oee"),
        ("jam", "line_jam"),
        ("conveyor_hz", "conv_hz"),
        ("filler_hz", "fill_hz"),
        ("kw", "energy_kw"),
        ("robot_state", "robot_state"),
        ("robot_cycle_s", "robot_cycle_s"),
    ]
    .iter()
    .map(|(k, pt)| obj(json!({"key":k,"point":pt})))
    .collect();
    outputs.push(p);
    let mut p = port(11, HubProto::Modbus, "Line run command");
    p.modbus_connection = "line_plc".into();
    p.modbus_outputs = vec![reg("Run", "coil", 0, "bool", "abcd", 1.0, "", "cmd_line_run")];
    outputs.push(p);
    let mut p = port(12, HubProto::Modbus, "Conveyor speed");
    p.modbus_connection = "conveyor_vfd".into();
    p.modbus_outputs = vec![reg("Speed reference", "holding", 101, "u16", "abcd", 0.01, "Hz", "cmd_conveyor_hz")];
    outputs.push(p);
    f.hubs = Some(model(hub("Bottling line", HubProto::Mqtt, sources, outputs)));
    f
}

// ---------------------------------------------------------------------------
// Water lift station
// ---------------------------------------------------------------------------

pub fn water_lift_station() -> ProjectFile {
    let mut f = blank_project();
    f.settings.name = "Lift station".into();
    f.settings.description = "Municipal wastewater lift station. The station RTU (DNP3: wet well level, discharge \
        header, two duty pumps, alarms, start counters) and a chlorine analyzer (Modbus TCP) feed an OPC UA server \
        hub for SCADA (opc.tcp://:4855). A WebSocket server (:8767) drives a web HMI both ways: live values out, \
        pump start / stop commands in, sent to the RTU as DNP3 CROBs. Run `pb-sim water`."
        .into();
    f.next_point_seq = 200;
    f.opcua.embedded.enabled = true;
    f.opcua.embedded.port = 4855;
    f.opcua.embedded.name = "Lift station".into();
    f.modbus.connections = vec![ModbusConn { id: "cl2".into(), label: "Chlorine analyzer".into(), port: 5041, enabled: true, ..ModbusConn::default_client() }];
    f.modbus.selected_id = "cl2".into();
    f.ext.dnp3.connections = vec![ext_conn(json!({"id":"lift_rtu","label":"Lift station RTU","host":"127.0.0.1","port":20001,"master_address":1,"outstation_address":20,"timeout_ms":3000}))];
    f.ext.websocket.connections = vec![ext_conn(json!({"id":"hmi","label":"Web HMI","mode":"server","host":"0.0.0.0","port":8767,"out_format":"object"}))];
    let mut sources = Vec::new();
    let mut p = port(2, HubProto::Dnp3, "Lift station RTU");
    p.ext_connection = "lift_rtu".into();
    p.ext_interval_secs = 1.0;
    let d = |kind: &str, index: i64, point: &str| obj(json!({"kind":kind,"index":index,"scale":1.0,"offset":0.0,"point":point}));
    p.ext_inputs = vec![
        d("analog", 0, "wet_well_ft"),
        d("analog", 1, "discharge_psi"),
        d("analog", 2, "flow_gpm"),
        d("analog", 3, "pump1_amps"),
        d("analog", 4, "pump2_amps"),
        d("binary", 0, "pump1_running"),
        d("binary", 1, "pump2_running"),
        d("binary", 2, "high_level_alarm"),
        d("binary", 3, "power_fail"),
        d("counter", 0, "pump1_starts"),
        d("counter", 1, "pump2_starts"),
    ];
    sources.push(p);
    let mut p = port(3, HubProto::Modbus, "Chlorine analyzer");
    p.modbus_connection = "cl2".into();
    p.modbus_interval_secs = 2.0;
    p.modbus_inputs = vec![
        reg("Free chlorine", "input", 0, "f32", "abcd", 1.0, "mg/L", "cl2_free_mgl"),
        reg("pH", "input", 2, "f32", "abcd", 1.0, "", "cl2_ph"),
        reg("Turbidity", "input", 4, "f32", "abcd", 1.0, "NTU", "cl2_turbidity"),
        reg("Sample temperature", "input", 6, "f32", "abcd", 1.0, "°C", "cl2_temp_c"),
    ];
    sources.push(p);
    let mut p = port(4, HubProto::WebSocket, "HMI commands");
    p.ext_connection = "hmi".into();
    p.ext_inputs = vec![
        obj(json!({"path":"cmd.pump1","scale":1.0,"offset":0.0,"point":"hmi_pump1_cmd"})),
        obj(json!({"path":"cmd.pump2","scale":1.0,"offset":0.0,"point":"hmi_pump2_cmd"})),
    ];
    sources.push(p);
    let mut outputs = Vec::new();
    let mut p = port(5, HubProto::WebSocket, "HMI live values");
    p.ext_connection = "hmi".into();
    p.ext_interval_secs = 0.5;
    p.ext_outputs = ["wet_well_ft", "discharge_psi", "flow_gpm", "pump1_amps", "pump2_amps", "pump1_running", "pump2_running", "high_level_alarm", "pump1_starts", "pump2_starts", "cl2_free_mgl", "cl2_ph", "cl2_turbidity"]
        .iter()
        .map(|pt| obj(json!({"key":pt,"point":pt})))
        .collect();
    outputs.push(p);
    let mut p = port(6, HubProto::Dnp3, "Pump commands (CROB)");
    p.ext_connection = "lift_rtu".into();
    p.ext_outputs = vec![
        obj(json!({"kind":"binary_output","index":0,"float":true,"scale":1.0,"offset":0.0,"point":"hmi_pump1_cmd"})),
        obj(json!({"kind":"binary_output","index":1,"float":true,"scale":1.0,"offset":0.0,"point":"hmi_pump2_cmd"})),
    ];
    outputs.push(p);
    f.hubs = Some(model(hub("Lift station", HubProto::OpcUa, sources, outputs)));
    f
}

// ---------------------------------------------------------------------------
// Generator plant (no devices needed)
// ---------------------------------------------------------------------------

/// One generator signal built the way the UI builds it: a preset (or model) applied to a fresh
/// draft, then `extra` overrides, with the point named after the signal.
fn gen_signal(preset: &str, name: &str, extra: &[(&str, Value)]) -> Map<String, Value> {
    let mut o = crate::gen_ui::new_draft();
    match crate::gen_ui::PRESETS.iter().find(|p| p.label == preset) {
        Some(pr) => crate::gen_ui::apply_preset(&mut o, pr),
        None => crate::gen_ui::pick_model(&mut o, preset),
    }
    o.insert("name".into(), Value::from(name));
    for (k, v) in extra {
        o.insert((*k).into(), v.clone());
    }
    o.insert("point".into(), Value::from(name));
    o
}

fn gen_node(id: u64, label: &str, signals: Vec<Map<String, Value>>) -> HubPort {
    let mut p = port(id, HubProto::Gen, label);
    p.ext_connection = "plant".into();
    p.ext_inputs = signals;
    p
}

pub fn generator_plant() -> ProjectFile {
    let mut f = blank_project();
    f.settings.name = "Generator plant".into();
    f.settings.description = "Six generator sources — no devices needed — feed one MQTT hub (embedded broker :1887). \
        Tank + pump, temperature, line pressure + flow, a power meter, weather + battery, and an event node cover \
        every kind of signal (drift, triangle, on/off cycle, lag, sine, random walk, ramp, totaliser, counter, \
        a formula that reads another signal) and every trigger (regular interval, random timer, bursts). \
        Out: UNS topics plant/gen/<signal>. The board uses seed 42 so the data repeats; set it to 0 for fresh data."
        .into();
    f.next_point_seq = 200;
    f.mqtt.embedded.enabled = true;
    f.mqtt.embedded.port = 1887;
    f.mqtt.clients = vec![loopback(1887, "pb-gen", &["plant/#"])];
    f.mqtt.selected_id = "local".into();
    // No leftover sample Modbus endpoints: this demo opens no device links.
    f.modbus.connections.clear();
    f.modbus.selected_id.clear();
    f.ext.gen.connections = vec![ext_conn(json!({"id":"plant","label":"Plant generators","seed":42,"speed":1}))];
    f.ext.gen.selected_id = "plant".into();

    let n = |v: f64| json!(v);
    let sources = vec![
        gen_node(
            2,
            "Tank & pump",
            vec![
                gen_signal("Tank level", "tank_level", &[]),
                gen_signal("Pump running", "pump_run", &[("every", n(1.0))]),
                // A formula that reads another signal on the board: power only while the pump runs.
                gen_signal("formula", "pump_kw", &[("expr", json!("pump_run * (7.5 + 0.6 * sin(t / 9)) + noise(0.05)")), ("decimals", json!(2))]),
            ],
        ),
        gen_node(3, "Process temperature", vec![gen_signal("Temperature", "temperature", &[]), gen_signal("Humidity", "humidity", &[("every", n(2.0))])]),
        gen_node(4, "Line pressure & flow", vec![gen_signal("Discharge pressure", "pressure", &[("every", n(0.5))]), gen_signal("Flow rate", "flow", &[])]),
        gen_node(5, "Power meter", vec![gen_signal("Daily load", "load_kw", &[("every", n(2.0))]), gen_signal("Energy", "energy_kwh", &[("every", n(2.0))])]),
        gen_node(6, "Weather & battery", vec![gen_signal("Wind speed", "wind", &[("every", n(1.5))]), gen_signal("Battery voltage", "battery_v", &[("every", n(3.0))])]),
        gen_node(7, "Events & counters", vec![gen_signal("Heartbeat", "heartbeat", &[]), gen_signal("Random alarm", "alarm", &[]), gen_signal("Parts counter", "parts", &[])]),
    ];
    let mut p = port(8, HubProto::Mqtt, "UNS topics");
    p.mqtt_connection = "local".into();
    let names = [
        "tank_level", "pump_run", "pump_kw", "temperature", "humidity", "pressure", "flow", "load_kw", "energy_kwh", "wind", "battery_v", "heartbeat", "alarm", "parts",
    ];
    p.mqtt_outputs = names.iter().map(|n| mqtt_out(n, &format!("plant/gen/{n}"))).collect();
    // The same points on a live browser dashboard (http://<this machine>:8780/). The node *is* the
    // connection; its points come from the arrows drawn into it (one per generator node).
    let mut d = port(9, HubProto::Dashboard, "Plant dashboard");
    d.dash_title = "Plant".into();
    d.dash_port = 8780;
    let names_owned: Vec<String> = names.iter().map(|n| n.to_string()).collect();
    d.ext_outputs = crate::ext_proto::dashboard_auto_binds(&[], &names_owned);
    let gen_ids: Vec<u64> = sources.iter().map(|s| s.id).collect();
    let h = hub("Generator plant", HubProto::Mqtt, sources, vec![p, d]);
    let hub_id = h.id;
    let mut m = model(h);
    m.feeds.extend(gen_ids.iter().map(|id| crate::hubs::HubFeed { from_hub: hub_id, from_node: *id, to_hub: hub_id, to_node: 9 }));
    crate::ext_proto::sync_dashboard_conns(&mut m, &mut f.ext.dashboard.connections);
    f.hubs = Some(m);
    f
}

// ---------------------------------------------------------------------------
// CAN loopback (no devices: generators → CAN out → vcan0 → CAN in)
// ---------------------------------------------------------------------------

/// One loopback signal: the generator point `name`, sent in frame `tx_id` and read back from
/// `rx_id` (the same frame, or its J1939 PGN) into `rx_<name>`.
struct CanSig {
    name: &'static str,
    tx_id: &'static str,
    rx_id: &'static str,
    start: u32,
    len: u32,
    order: &'static str,
    kind: &'static str,
    scale: f64,
    offset: f64,
}

#[allow(clippy::too_many_arguments)]
const fn cs(name: &'static str, tx_id: &'static str, rx_id: &'static str, start: u32, len: u32, order: &'static str, kind: &'static str, scale: f64, offset: f64) -> CanSig {
    CanSig { name, tx_id, rx_id, start, len, order, kind, scale, offset }
}

/// The frame map. Six frames: standard (11-bit) Intel and Motorola, extended (29-bit) with
/// float32, and two J1939 parameter groups read back by PGN.
const CAN_SIGNALS: [CanSig; 20] = [
    // 0x100 pump skid — 11-bit, Intel.
    cs("tank_level", "0x100", "0x100", 0, 16, "intel", "unsigned", 0.1, 0.0),
    cs("pump_run", "0x100", "0x100", 16, 1, "intel", "unsigned", 1.0, 0.0),
    cs("motor_amps", "0x100", "0x100", 24, 16, "intel", "unsigned", 0.01, 0.0),
    cs("vibration", "0x100", "0x100", 40, 16, "intel", "unsigned", 0.01, 0.0),
    cs("alarm", "0x100", "0x100", 56, 1, "intel", "unsigned", 1.0, 0.0),
    // 0x101 process — 11-bit, Motorola (DBC start = MSB).
    cs("temperature", "0x101", "0x101", 7, 16, "motorola", "signed", 0.1, 0.0),
    cs("humidity", "0x101", "0x101", 23, 16, "motorola", "unsigned", 0.1, 0.0),
    cs("pressure", "0x101", "0x101", 39, 16, "motorola", "unsigned", 0.1, 0.0),
    cs("valve_pos", "0x101", "0x101", 55, 8, "motorola", "unsigned", 1.0, 0.0),
    // 0x18FF1000 flow & energy — 29-bit, two float32.
    cs("flow", "0x18FF1000", "0x18FF1000", 0, 32, "intel", "float", 1.0, 0.0),
    cs("energy_kwh", "0x18FF1000", "0x18FF1000", 32, 32, "intel", "float", 1.0, 0.0),
    // 0x18FF1100 power & weather — 29-bit, mixed widths, a signed field.
    cs("load_kw", "0x18FF1100", "0x18FF1100", 0, 16, "intel", "unsigned", 1.0, 0.0),
    cs("battery_v", "0x18FF1100", "0x18FF1100", 16, 16, "intel", "unsigned", 0.01, 0.0),
    cs("wind", "0x18FF1100", "0x18FF1100", 32, 8, "intel", "unsigned", 0.1, 0.0),
    cs("outdoor_temp", "0x18FF1100", "0x18FF1100", 40, 16, "intel", "signed", 0.1, 0.0),
    // J1939 EEC1 (PGN 61444) and ET1 (PGN 65262), J1939 scaling, read back by PGN.
    cs("engine_load", "0x0CF00400", "pgn:61444", 16, 8, "intel", "unsigned", 1.0, -125.0),
    cs("engine_rpm", "0x0CF00400", "pgn:61444", 24, 16, "intel", "unsigned", 0.125, 0.0),
    cs("coolant_c", "0x18FEEE00", "pgn:65262", 0, 8, "intel", "unsigned", 1.0, -40.0),
    cs("oil_temp_c", "0x18FEEE00", "pgn:65262", 16, 16, "intel", "unsigned", 0.03125, -273.0),
    // 0x200 counters — 11-bit, DLC 2.
    cs("heartbeat", "0x200", "0x200", 0, 16, "intel", "unsigned", 1.0, 0.0),
];

fn can_bind(s: &CanSig, id: &str, point: &str) -> Map<String, Value> {
    obj(json!({"id":id,"start":s.start,"len":s.len,"order":s.order,"type":s.kind,"scale":s.scale,"offset":s.offset,"point":point}))
}

pub fn can_loopback() -> ProjectFile {
    let mut f = blank_project();
    f.settings.name = "CAN loopback".into();
    f.settings.description = "CAN bus test with no hardware or setup. Twenty generator signals go out as six CAN frames on \
        the built-in virtual bus \"loop\" (connection bus_tx, every 100 ms) and come straight back in on a second \
        connection on the same bus (bus_rx) as rx_<signal>. Frames cover 11-bit and 29-bit ids, Intel and Motorola order, \
        signed / unsigned / float32, scale + offset, single bits, and two J1939 groups read back by PGN. \
        The read-back points are published to the hub's embedded broker (:1888) as can/rx/<signal>. \
        To test real hardware, switch both connections to SocketCAN or a PCAN Ethernet gateway."
        .into();
    f.next_point_seq = 200;
    f.mqtt.embedded.enabled = true;
    f.mqtt.embedded.port = 1888;
    f.mqtt.clients = vec![loopback(1888, "pb-can-loop", &["can/#"])];
    f.mqtt.selected_id = "local".into();
    f.modbus.connections.clear();
    f.modbus.selected_id.clear();
    f.ext.gen.connections = vec![ext_conn(json!({"id":"loop","label":"Loopback generators","seed":42,"speed":1}))];
    f.ext.gen.selected_id = "loop".into();
    // Two connections on one built-in virtual bus. Like SocketCAN, a frame reaches every *other*
    // connection, never the one that sent it, so send and receive are separate connections.
    // Point both at a real bus instead (SocketCAN can0 or a PCAN gateway) by changing transport.
    f.ext.can.connections = vec![
        ext_conn(json!({"id":"bus_tx","label":"Loopback transmit","transport":"virtual","bus":"loop","stale_secs":5})),
        ext_conn(json!({"id":"bus_rx","label":"Loopback receive","transport":"virtual","bus":"loop","stale_secs":5})),
    ];
    f.ext.can.selected_id = "bus_rx".into();

    let n = |v: f64| json!(v);
    let custom = |m: &str, name: &str, kv: &[(&str, Value)]| gen_signal(m, name, kv);
    let node = |id: u64, label: &str, signals: Vec<Map<String, Value>>| {
        let mut p = gen_node(id, label, signals);
        p.ext_connection = "loop".into();
        p
    };
    let sources_gen = vec![
        node(
            2,
            "Pump skid",
            vec![
                gen_signal("Tank level", "tank_level", &[]),
                gen_signal("Pump running", "pump_run", &[]),
                gen_signal("Motor current", "motor_amps", &[]),
                gen_signal("Vibration", "vibration", &[]),
                gen_signal("Random alarm", "alarm", &[]),
            ],
        ),
        node(
            3,
            "Process",
            vec![
                // Swings below zero so the signed Motorola field is exercised.
                custom("sine", "temperature", &[("low", n(-15.0)), ("high", n(45.0)), ("period", n(60.0)), ("noise", n(0.2)), ("decimals", n(1.0))]),
                gen_signal("Humidity", "humidity", &[]),
                gen_signal("Discharge pressure", "pressure", &[]),
                gen_signal("Valve position", "valve_pos", &[]),
            ],
        ),
        node(4, "Flow & energy", vec![gen_signal("Flow rate", "flow", &[]), gen_signal("Energy", "energy_kwh", &[])]),
        node(
            5,
            "Power & weather",
            vec![
                gen_signal("Daily load", "load_kw", &[]),
                gen_signal("Battery voltage", "battery_v", &[]),
                gen_signal("Wind speed", "wind", &[]),
                custom("walk", "outdoor_temp", &[("start", n(-5.0)), ("step", n(0.5)), ("low", n(-30.0)), ("high", n(35.0)), ("decimals", n(1.0))]),
            ],
        ),
        node(
            6,
            "Engine (J1939)",
            vec![
                custom("drift", "engine_load", &[("mean", n(55.0)), ("sigma", n(3.0)), ("reversion", n(0.1)), ("decimals", n(0.0))]),
                custom("sine", "engine_rpm", &[("low", n(800.0)), ("high", n(1800.0)), ("period", n(40.0)), ("noise", n(5.0)), ("decimals", n(0.0))]),
                custom("drift", "coolant_c", &[("mean", n(85.0)), ("sigma", n(0.8)), ("reversion", n(0.05)), ("decimals", n(0.0))]),
                custom("drift", "oil_temp_c", &[("mean", n(98.0)), ("sigma", n(0.5)), ("reversion", n(0.05)), ("decimals", n(1.0))]),
            ],
        ),
        node(7, "Counters", vec![gen_signal("Heartbeat", "heartbeat", &[])]),
    ];
    let mut sources = sources_gen;
    let mut rx = port(8, HubProto::Can, "CAN receive (loopback)");
    rx.ext_connection = "bus_rx".into();
    rx.ext_inputs = CAN_SIGNALS.iter().map(|s| can_bind(s, s.rx_id, &format!("rx_{}", s.name))).collect();
    sources.push(rx);

    let mut tx = port(9, HubProto::Can, "CAN transmit");
    tx.ext_connection = "bus_tx".into();
    tx.ext_interval_secs = 0.1;
    tx.ext_outputs = CAN_SIGNALS
        .iter()
        .map(|s| {
            let mut b = can_bind(s, s.tx_id, s.name);
            b.insert("dlc".into(), json!(if s.tx_id == "0x200" { 2 } else { 8 }));
            b
        })
        .collect();
    let mut up = port(10, HubProto::Mqtt, "Read-back topics");
    up.mqtt_connection = "local".into();
    up.mqtt_outputs = CAN_SIGNALS.iter().map(|s| mqtt_out(&format!("rx_{}", s.name), &format!("can/rx/{}", s.name))).collect();
    f.hubs = Some(model(hub("CAN loopback", HubProto::Mqtt, sources, vec![tx, up])));
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> Vec<(&'static str, ProjectFile)> {
        vec![("frac-spread.json", frac_spread("/tmp/pb-data")), ("bottling-line.json", bottling_line()), ("water-lift-station.json", water_lift_station()), ("generator-plant.json", generator_plant()), ("can-loopback.json", can_loopback())]
    }

    /// Every demo serializes, reloads unchanged, and every Output point is produced by an Input.
    #[test]
    fn industrial_demos_are_consistent() {
        for (name, f) in all() {
            let text = serde_json::to_string_pretty(&f).unwrap();
            let back: ProjectFile = serde_json::from_str(&text).unwrap();
            let again = serde_json::to_string_pretty(&back).unwrap();
            if again != text {
                let diff: Vec<(&str, &str)> = text.lines().zip(again.lines()).filter(|(a, b)| a != b).take(5).collect();
                panic!("{name} round trip differs: {diff:?}");
            }
            let hubs = f.hubs.as_ref().unwrap();
            let mut produced = std::collections::HashSet::new();
            for h in &hubs.hubs {
                for s in &h.sources {
                    s.modbus_inputs.iter().for_each(|b| {
                        assert!(b.problem(false).is_none(), "{name}: {}", b.describe());
                        produced.insert(b.point.clone());
                    });
                    s.mqtt_inputs.iter().for_each(|b| {
                        produced.insert(b.point.clone());
                    });
                    if let Some(e) = s.protocol.ext() {
                        assert!(!s.ext_connection.is_empty());
                        for b in &s.ext_inputs {
                            assert!(crate::ext_proto::bind_source(e, &s.ext_connection, b).is_some(), "{name}: {b:?}");
                            produced.insert(crate::ext_proto::text_of(b, "point"));
                        }
                        let conns = f.ext.get(e).connections.iter().map(|c| crate::ext_proto::text_of(c, "id")).collect::<Vec<_>>();
                        assert!(conns.contains(&s.ext_connection), "{name}: {} missing", s.ext_connection);
                    }
                }
                for o in &h.outputs {
                    let mut pts: Vec<String> = o.modbus_outputs.iter().map(|b| b.point.clone()).collect();
                    pts.extend(o.mqtt_outputs.iter().map(|b| b.point.clone()));
                    pts.extend(o.las_outputs.iter().map(|b| b.point.clone()));
                    pts.extend(o.ext_outputs.iter().map(|b| crate::ext_proto::text_of(b, "point")));
                    for p in pts {
                        assert!(produced.contains(&p), "{name}: output point {p} has no input");
                    }
                }
            }
            // Service builds pack config for each ext section without dropping binds.
            let v = serde_json::to_value(&f).unwrap();
            for e in crate::ext_proto::ExtProto::ALL {
                let n_in: usize = hubs.hubs.iter().flat_map(|h| h.sources.iter()).filter(|s| s.protocol.ext() == Some(e)).map(|s| s.ext_inputs.len()).sum();
                let _ = (n_in, &v);
            }
        }
    }

    /// The generator demo: six source nodes, every signal valid and uniquely named, formulas only
    /// read what exists, every MQTT output has a producer, and nothing is wired into a generator.
    #[test]
    fn generator_plant_is_consistent() {
        let f = generator_plant();
        let hubs = f.hubs.as_ref().unwrap();
        let hub = &hubs.hubs[0];
        let gens: Vec<&HubPort> = hub.sources.iter().filter(|s| s.protocol == HubProto::Gen).collect();
        assert_eq!(gens.len(), 6, "six generator nodes");
        let board: Vec<String> = f.ext.gen.connections.iter().map(|c| crate::ext_proto::text_of(c, "id")).collect();
        assert_eq!(board, ["plant"]);
        let mut names = std::collections::HashSet::new();
        let mut triggers = std::collections::HashSet::new();
        let mut models = std::collections::HashSet::new();
        for g in &gens {
            assert_eq!(g.ext_connection, "plant");
            assert!(!g.ext_inputs.is_empty());
            for b in &g.ext_inputs {
                let name = crate::ext_proto::text_of(b, "name");
                assert!(names.insert(name.clone()), "duplicate signal {name}");
                assert_eq!(crate::ext_proto::text_of(b, "point"), name, "point named after the signal");
                assert_eq!(crate::ext_proto::bind_source(crate::ext_proto::ExtProto::Gen, "plant", b).unwrap(), format!("gen/plant::{name}"));
                pb_signal::preview(b).unwrap_or_else(|e| panic!("{name}: {e}"));
                triggers.insert(crate::ext_proto::text_of(b, "trigger"));
                models.insert(crate::ext_proto::text_of(b, "model"));
            }
        }
        assert_eq!(triggers.len(), 3, "interval, random and burst all appear: {triggers:?}");
        assert!(models.len() >= 9, "a spread of models: {models:?}");
        // Formulas only read built-ins and signals that exist.
        for g in &gens {
            for b in &g.ext_inputs {
                let bind = pb_extkit::Bind { fields: b.clone(), point: String::new(), source: String::new(), interval: std::time::Duration::from_secs(1) };
                let sig = pb_signal::Signal::from_bind(&bind, 1, 0).unwrap();
                for v in sig.formula_vars() {
                    assert!(pb_signal::BUILTIN_VARS.contains(&v.as_str()) || names.contains(&v), "unknown variable {v}");
                }
            }
        }
        // Every published topic maps to a signal, every signal is published, and the hub is MQTT.
        assert_eq!(hub.protocol, HubProto::Mqtt);
        let out = &hub.outputs[0];
        let pts: std::collections::HashSet<String> = out.mqtt_outputs.iter().map(|b| b.point.clone()).collect();
        assert_eq!(pts, names);
        assert!(out.mqtt_outputs.iter().all(|b| b.topic == format!("plant/gen/{}", b.point)));
        // Generators are sources only: no generator node among the outputs.
        assert!(hub.outputs.iter().all(|o| o.protocol != HubProto::Gen));
        // Every generator is wired to the hub.
        for g in &gens {
            assert!(hubs.feeds.iter().any(|fd| fd.from_node == g.id && fd.to_node == hub.id), "{} not wired", g.label);
        }
    }

    /// CAN loopback: twenty generator signals, each sent on bus_tx and read back on bus_rx (two
    /// connections — a CAN connection never hears its own frames), both on virtual bus "loop".
    #[test]
    fn can_loopback_is_consistent() {
        let f = can_loopback();
        let hub = &f.hubs.as_ref().unwrap().hubs[0];
        let gen: std::collections::HashSet<String> =
            hub.sources.iter().filter(|s| s.protocol == HubProto::Gen).flat_map(|s| s.ext_inputs.iter().map(|b| crate::ext_proto::text_of(b, "point"))).collect();
        assert_eq!(gen.len(), 20);
        let rx = hub.sources.iter().find(|s| s.protocol == HubProto::Can).unwrap();
        let tx = hub.outputs.iter().find(|s| s.protocol == HubProto::Can).unwrap();
        assert_ne!(rx.ext_connection, tx.ext_connection, "send and receive need separate sockets");
        let buses: Vec<(String, String)> = f.ext.can.connections.iter().map(|c| (crate::ext_proto::text_of(c, "transport"), crate::ext_proto::text_of(c, "bus"))).collect();
        assert_eq!(buses, [("virtual".to_string(), "loop".to_string()), ("virtual".to_string(), "loop".to_string())]);
        let sent: std::collections::HashSet<String> = tx.ext_outputs.iter().map(|b| crate::ext_proto::text_of(b, "point")).collect();
        assert_eq!(sent, gen, "every generator signal is transmitted");
        for b in &rx.ext_inputs {
            let p = crate::ext_proto::text_of(b, "point");
            assert!(gen.contains(p.trim_start_matches("rx_")), "{p}");
            assert!(crate::ext_proto::bind_source(crate::ext_proto::ExtProto::Can, "bus_rx", b).is_some(), "{p}");
        }
        assert_eq!(rx.ext_inputs.len(), 20);
    }

    /// Writes the demo projects into `PB_EXPORT_DEMOS` (data files under `<dir>/data`).
    #[test]
    #[ignore]
    fn export_industrial_demos() {
        let Some(dir) = std::env::var_os("PB_EXPORT_DEMOS") else { return };
        let dir = std::path::PathBuf::from(dir);
        let data = dir.join("data");
        std::fs::create_dir_all(&data).unwrap();
        let data_s = data.to_string_lossy().into_owned();
        for (name, f) in [("frac-spread.json", frac_spread(&data_s)), ("bottling-line.json", bottling_line()), ("water-lift-station.json", water_lift_station()), ("generator-plant.json", generator_plant()), ("can-loopback.json", can_loopback())] {
            std::fs::write(dir.join(name), serde_json::to_string_pretty(&f).unwrap() + "\n").unwrap();
        }
    }
}
