//! Sparkplug B worker (host and edge roles).

use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

use pb_abi::PB_LOG_INFO;
use pb_extkit::{now_ms, ConnExt, LinkCtx, Val};
use pb_proto::sparkplug::{self as spb, dt, MValue, Metric, Payload};
use rumqttc::{Client, Event, LastWill, MqttOptions, Packet, QoS, RecvTimeoutError};
use serde_json::{Map, Value};

pub struct PackState(pub pb_extkit::ExtPack);

fn is_edge(c: &Map<String, Value>) -> bool {
    c.s("role", "host").eq_ignore_ascii_case("edge")
}

fn endpoint(c: &Map<String, Value>) -> String {
    let role = if is_edge(c) { "edge" } else { "host" };
    format!("{role} {}:{} · {}", c.s("host", "127.0.0.1"), c.n("port", 1883.0) as u16, c.s("group_id", "+"))
}

fn check(c: &Map<String, Value>) -> Result<(), String> {
    if is_edge(c) && (c.s("group_id", "").is_empty() || c.s("edge_node_id", "").is_empty()) {
        return Err("an edge node needs a group id and an edge node id".into());
    }
    for k in ["group_id", "edge_node_id"] {
        if c.s(k, "").contains(['/', '+', '#']) {
            return Err(format!("{k} may not contain / + #"));
        }
    }
    Ok(())
}

fn mvalue_to_val(v: &MValue) -> Option<Val> {
    Some(match v {
        MValue::Int(i) => Val::Int(*i),
        MValue::UInt(u) => i64::try_from(*u).map(Val::Int).unwrap_or(Val::Float(*u as f64)),
        MValue::Float(f) => Val::Float(format!("{f}").parse().unwrap_or(*f as f64)),
        MValue::Double(d) => Val::Float(*d),
        MValue::Bool(b) => Val::Bool(*b),
        MValue::Text(t) => Val::Text(t.clone()),
        MValue::Null => return None,
    })
}

fn options(ctx: &LinkCtx, default_id: &str) -> MqttOptions {
    let c = &ctx.conn;
    let id = c.s("client_id", default_id);
    let mut o = MqttOptions::new(id, c.s("host", "127.0.0.1"), c.n("port", 1883.0) as u16);
    o.set_keep_alive(Duration::from_secs(10));
    o.set_clean_session(true);
    let user = c.s("username", "");
    if !user.is_empty() {
        o.set_credentials(user, c.s("password", ""));
    }
    o
}

pub fn run(ctx: LinkCtx) {
    if is_edge(&ctx.conn) {
        run_edge(ctx)
    } else {
        run_host(ctx)
    }
}

/// Wait for the CONNACK (or an error) so "connected" means connected.
fn await_connack(conn: &mut rumqttc::Connection, ctx: &LinkCtx) -> Result<(), String> {
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end && !ctx.stopped() {
        match conn.recv_timeout(Duration::from_millis(100)) {
            Ok(Ok(Event::Incoming(Packet::ConnAck(_)))) => return Ok(()),
            Ok(Ok(_)) => {}
            Ok(Err(e)) => return Err(e.to_string()),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Err("disconnected".into()),
        }
    }
    Err("no CONNACK from broker".into())
}

// ---------------------------------------------------------------------------
// Host role
// ---------------------------------------------------------------------------

fn run_host(mut ctx: LinkCtx) {
    let group = ctx.conn.s("group_id", "+");
    let edge_filter = ctx.conn.s("edge_node_id", "");
    let id = format!("pb-spb-host-{}", ctx.id);
    let mut backoff = Duration::from_millis(500);
    while !ctx.stopped() {
        ctx.status(|s| s.state = "connecting".into());
        let (client, mut conn) = Client::new(options(&ctx, &id), 256);
        if let Err(e) = await_connack(&mut conn, &ctx) {
            ctx.down(format!("{}: {e}", endpoint(&ctx.conn)));
            if !pb_extkit::sleep_stop(&ctx.stop, backoff) {
                break;
            }
            backoff = (backoff * 2).min(Duration::from_secs(10));
            continue;
        }
        backoff = Duration::from_millis(500);
        let filter = if edge_filter.is_empty() { format!("spBv1.0/{group}/#") } else { format!("spBv1.0/{group}/+/{edge_filter}/#") };
        let _ = client.subscribe(filter.clone(), QoS::AtMostOnce);
        ctx.up(format!("host · {filter}"));
        ctx.log(PB_LOG_INFO, &format!("Sparkplug host subscribed {filter}"));
        // (edge, device) → alias → (name, datatype)
        let mut aliases: HashMap<(String, String), HashMap<u64, (String, u32)>> = HashMap::new();
        // edge → (group, metric datatypes by (device, name)) for commands
        let mut known: HashMap<(String, String, String), u32> = HashMap::new();
        let mut last_edge_group: HashMap<String, String> = HashMap::new();
        let err = loop {
            if ctx.stopped() {
                let _ = client.disconnect();
                break None;
            }
            match conn.recv_timeout(Duration::from_millis(50)) {
                Ok(Ok(Event::Incoming(Packet::Publish(p)))) => {
                    let Some(t) = spb::parse_topic(&p.topic) else { continue };
                    if !edge_filter.is_empty() && t.edge != edge_filter {
                        continue;
                    }
                    last_edge_group.insert(t.edge.clone(), t.group.clone());
                    on_host_message(&ctx, &t, &p.payload, &mut aliases, &mut known);
                }
                Ok(Ok(_)) => {}
                Ok(Err(e)) => break Some(e.to_string()),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break Some("disconnected".into()),
            }
            // Outputs: commands to edge nodes.
            for (point, value) in ctx.drain_writes() {
                for b in ctx.outputs_for(&point) {
                    let edge = if b.text("edge").is_empty() { edge_filter.clone() } else { b.text("edge") };
                    if edge.is_empty() {
                        continue;
                    }
                    let device = b.text("device");
                    let metric = b.text("metric");
                    let g = last_edge_group.get(&edge).cloned().unwrap_or_else(|| group.clone());
                    if g == "+" {
                        continue;
                    }
                    let datatype = known.get(&(edge.clone(), device.clone(), metric.clone())).copied();
                    let (dtp, v) = match datatype {
                        Some(d) => (d, spb::text_as(d, &value)),
                        None => spb::from_text(&value),
                    };
                    let payload = Payload { timestamp: Some(now_ms()), seq: None, metrics: vec![Metric::new(&metric, dtp, v)] };
                    let kind = if device.is_empty() { "NCMD" } else { "DCMD" };
                    if client.try_publish(spb::topic(&g, kind, &edge, &device), QoS::AtMostOnce, false, spb::encode(&payload)).is_ok() {
                        ctx.status(|s| s.tx += 1);
                    }
                }
            }
        };
        match err {
            None => break,
            Some(e) => {
                ctx.down(format!("{}: {e}", endpoint(&ctx.conn)));
                if !pb_extkit::sleep_stop(&ctx.stop, backoff) {
                    break;
                }
            }
        }
    }
}

fn input_matches(b: &pb_extkit::Bind, device: &str, metric: &str) -> bool {
    b.text("metric").trim() == metric && b.text("device").trim() == device
}

fn on_host_message(
    ctx: &LinkCtx,
    t: &spb::Topic,
    payload: &[u8],
    aliases: &mut HashMap<(String, String), HashMap<u64, (String, u32)>>,
    known: &mut HashMap<(String, String, String), u32>,
) {
    let key = (t.edge.clone(), t.device.clone());
    match t.kind.as_str() {
        "NDEATH" | "DDEATH" => {
            for b in &ctx.inputs {
                let dev = b.text("device");
                if t.kind == "NDEATH" || dev.trim() == t.device {
                    ctx.publish_bad(b);
                }
            }
            ctx.status(|s| s.see(&format!("{}/{}", t.edge, t.device), Map::new(), format!("{} {} died", t.edge, t.device), "DEATH"));
            return;
        }
        "NBIRTH" | "DBIRTH" | "NDATA" | "DDATA" => {}
        _ => return,
    }
    let Ok(p) = spb::decode(payload) else {
        ctx.status(|s| {
            s.errors += 1;
            s.error = format!("bad payload on {}", t.kind);
        });
        return;
    };
    ctx.status(|s| s.rx += 1);
    let birth = t.kind.ends_with("BIRTH");
    if birth {
        aliases.insert(key.clone(), HashMap::new());
    }
    for m in &p.metrics {
        let (name, datatype) = match (&m.name, m.alias) {
            (Some(n), alias) => {
                if let (true, Some(a)) = (birth, alias) {
                    aliases.entry(key.clone()).or_default().insert(a, (n.clone(), m.datatype));
                }
                (n.clone(), m.datatype)
            }
            (None, Some(a)) => match aliases.get(&key).and_then(|m| m.get(&a)) {
                Some((n, d)) => (n.clone(), *d),
                None => continue,
            },
            _ => continue,
        };
        if name == "bdSeq" {
            continue;
        }
        known.insert((t.edge.clone(), t.device.clone(), name.clone()), datatype);
        let val = mvalue_to_val(&m.value);
        ctx.status(|s| {
            let mut leaf = Map::new();
            leaf.insert("device".into(), Value::from(t.device.clone()));
            leaf.insert("metric".into(), Value::from(name.clone()));
            let label = if t.device.is_empty() { format!("{} · {name}", t.edge) } else { format!("{}/{} · {name}", t.edge, t.device) };
            s.see(&format!("{}/{}/{name}", t.edge, t.device), leaf, label, val.as_ref().map(|v| v.to_string()).unwrap_or_else(|| "null".into()));
        });
        for b in ctx.inputs.iter().filter(|b| input_matches(b, &t.device, &name)) {
            match &val {
                Some(v) => ctx.publish(b, &scaled(b, v)),
                None => ctx.publish_bad(b),
            }
        }
    }
}

fn scaled(b: &pb_extkit::Bind, v: &Val) -> Val {
    let (s, o) = b.scaling();
    match v {
        Val::Int(i) if s != 1.0 || o != 0.0 => Val::Float(*i as f64 * s + o),
        Val::Float(f) => Val::Float(f * s + o),
        other => other.clone(),
    }
}

// ---------------------------------------------------------------------------
// Edge role
// ---------------------------------------------------------------------------

fn run_edge(mut ctx: LinkCtx) {
    let group = ctx.conn.s("group_id", "");
    let edge = ctx.conn.s("edge_node_id", "");
    let id = format!("pb-spb-edge-{group}-{edge}");
    let every = ctx.output_interval(Duration::from_millis(500));
    let mut bd_seq: u64 = 0;
    let mut backoff = Duration::from_millis(500);
    while !ctx.stopped() {
        ctx.status(|s| s.state = "connecting".into());
        let mut opts = options(&ctx, &id);
        let death = Payload { timestamp: Some(now_ms()), seq: None, metrics: vec![Metric::new("bdSeq", dt::UINT64, MValue::UInt(bd_seq))] };
        opts.set_last_will(LastWill::new(spb::topic(&group, "NDEATH", &edge, ""), spb::encode(&death), QoS::AtLeastOnce, false));
        let (client, mut conn) = Client::new(opts, 256);
        if let Err(e) = await_connack(&mut conn, &ctx) {
            ctx.down(format!("{}: {e}", endpoint(&ctx.conn)));
            if !pb_extkit::sleep_stop(&ctx.stop, backoff) {
                break;
            }
            backoff = (backoff * 2).min(Duration::from_secs(10));
            continue;
        }
        backoff = Duration::from_millis(500);
        let _ = client.subscribe(spb::topic(&group, "NCMD", &edge, ""), QoS::AtLeastOnce);
        let _ = client.subscribe(format!("spBv1.0/{group}/DCMD/{edge}/+"), QoS::AtLeastOnce);
        // Birth: one alias per (device, metric); datatypes fixed by the first known value.
        let mut seq: u64 = 0;
        let mut next_seq = || {
            let s = seq;
            seq = (seq + 1) % 256;
            s
        };
        let mut types: BTreeMap<(String, String), (u64, u32)> = BTreeMap::new();
        for (i, b) in ctx.outputs.iter().enumerate() {
            let (dtp, _) = ctx.latest.get(&b.point).map(|v| spb::from_text(v)).unwrap_or((dt::DOUBLE, MValue::Null));
            let dtp = match b.text("datatype").to_ascii_lowercase().as_str() {
                "float" => dt::FLOAT,
                "double" => dt::DOUBLE,
                "int32" => dt::INT32,
                "int64" => dt::INT64,
                "boolean" | "bool" => dt::BOOLEAN,
                "string" => dt::STRING,
                _ => dtp,
            };
            types.entry((b.text("device"), b.text("metric"))).or_insert((i as u64 + 1, dtp));
        }
        let devices: Vec<String> = {
            let mut d: Vec<String> = types.keys().map(|(d, _)| d.clone()).filter(|d| !d.is_empty()).collect();
            d.dedup();
            d
        };
        let metric_now = |ctx: &LinkCtx, device: &str, name: &str, alias: u64, dtp: u32, with_name: bool| -> Metric {
            let value = ctx
                .outputs
                .iter()
                .find(|b| b.text("device") == device && b.text("metric") == name)
                .and_then(|b| ctx.latest.get(&b.point))
                .map(|v| spb::text_as(dtp, v))
                .unwrap_or(MValue::Null);
            Metric { name: with_name.then(|| name.to_string()), alias: Some(alias), timestamp: Some(now_ms()), datatype: dtp, value }
        };
        let mut nbirth = vec![Metric::new("bdSeq", dt::UINT64, MValue::UInt(bd_seq))];
        nbirth.push(Metric::new("Node Control/Rebirth", dt::BOOLEAN, MValue::Bool(false)));
        for ((d, n), (a, t)) in &types {
            if d.is_empty() {
                nbirth.push(metric_now(&ctx, d, n, *a, *t, true));
            }
        }
        let publish = |client: &Client, kind: &str, device: &str, metrics: Vec<Metric>, seq: u64| {
            let p = Payload { timestamp: Some(now_ms()), seq: Some(seq), metrics };
            client.try_publish(spb::topic(&group, kind, &edge, device), QoS::AtMostOnce, false, spb::encode(&p)).is_ok()
        };
        let s0 = next_seq();
        publish(&client, "NBIRTH", "", nbirth, s0);
        for dev in &devices {
            let ms: Vec<Metric> = types.iter().filter(|((d, _), _)| d == dev).map(|((d, n), (a, t))| metric_now(&ctx, d, n, *a, *t, true)).collect();
            let s = next_seq();
            publish(&client, "DBIRTH", dev, ms, s);
        }
        ctx.up(format!("edge {group}/{edge} · born (bdSeq {bd_seq})"));
        ctx.log(PB_LOG_INFO, &format!("Sparkplug edge {group}/{edge} NBIRTH"));
        let mut pending: Vec<String> = Vec::new();
        let mut next_send = Instant::now();
        let err = loop {
            if ctx.stopped() {
                // Clean shutdown: publish our own NDEATH (the will only fires on a broken link).
                let death = Payload { timestamp: Some(now_ms()), seq: None, metrics: vec![Metric::new("bdSeq", dt::UINT64, MValue::UInt(bd_seq))] };
                let _ = client.try_publish(spb::topic(&group, "NDEATH", &edge, ""), QoS::AtMostOnce, false, spb::encode(&death));
                let _ = conn.recv_timeout(Duration::from_millis(100));
                let _ = client.disconnect();
                break None;
            }
            match conn.recv_timeout(Duration::from_millis(30)) {
                Ok(Ok(Event::Incoming(Packet::Publish(p)))) => {
                    if let Some(t) = spb::parse_topic(&p.topic) {
                        if let Ok(pl) = spb::decode(&p.payload) {
                            ctx.status(|s| s.rx += 1);
                            let mut rebirth = false;
                            for m in &pl.metrics {
                                let name = m.name.clone().or_else(|| {
                                    m.alias.and_then(|a| types.iter().find(|(_, (al, _))| *al == a).map(|((_, n), _)| n.clone()))
                                });
                                let Some(name) = name else { continue };
                                if name == "Node Control/Rebirth" && m.value == MValue::Bool(true) {
                                    rebirth = true;
                                    continue;
                                }
                                let val = mvalue_to_val(&m.value);
                                ctx.status(|s| {
                                    let mut leaf = Map::new();
                                    leaf.insert("device".into(), Value::from(t.device.clone()));
                                    leaf.insert("metric".into(), Value::from(name.clone()));
                                    s.see(&format!("cmd/{}/{name}", t.device), leaf, format!("{} {name}", t.kind), val.as_ref().map(|v| v.to_string()).unwrap_or_default());
                                });
                                if let Some(v) = &val {
                                    for b in ctx.inputs.iter().filter(|b| input_matches(b, &t.device, &name)) {
                                        ctx.publish(b, &scaled(b, v));
                                    }
                                }
                            }
                            if rebirth {
                                break Some("rebirth requested".into());
                            }
                        }
                    }
                }
                Ok(Ok(_)) => {}
                Ok(Err(e)) => break Some(e.to_string()),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break Some("disconnected".into()),
            }
            for (p, _) in ctx.drain_writes() {
                if !pending.contains(&p) {
                    pending.push(p);
                }
            }
            if !pending.is_empty() && Instant::now() >= next_send {
                next_send = Instant::now() + every;
                // Group changed metrics per device: NDATA for node metrics, DDATA per device.
                let mut by_dev: BTreeMap<String, Vec<Metric>> = BTreeMap::new();
                for p in pending.drain(..) {
                    for b in ctx.outputs.iter().filter(|b| b.point == p) {
                        let key = (b.text("device"), b.text("metric"));
                        if let Some((a, t)) = types.get(&key) {
                            by_dev.entry(key.0.clone()).or_default().push(metric_now(&ctx, &key.0, &key.1, *a, *t, false));
                        }
                    }
                }
                for (dev, ms) in by_dev {
                    let s = next_seq();
                    if publish(&client, if dev.is_empty() { "NDATA" } else { "DDATA" }, &dev, ms, s) {
                        ctx.status(|st| st.tx += 1);
                    }
                }
            }
        };
        bd_seq = (bd_seq + 1) % 256;
        match err {
            None => break,
            Some(e) if e == "rebirth requested" => continue,
            Some(e) => {
                ctx.down(format!("{}: {e}", endpoint(&ctx.conn)));
                if !pb_extkit::sleep_stop(&ctx.stop, backoff) {
                    break;
                }
            }
        }
    }
}

pb_extkit::ext_pack_impl!(PackState, b"sparkplug\0", b"Sparkplug B\0", "sparkplug", run, endpoint, check);
pb_abi::export_pack!(PackState);

#[cfg(test)]
mod tests {
    use super::*;
    use pb_extkit::{testhost, ExtPack};
    use rumqttd::protocol::v4::V4;
    use rumqttd::{ConnectionSettings, LinkType, Router, RouterConfig, Server, ServerSettings};

    fn broker() -> u16 {
        let port = testhost::free_port();
        let listen: std::net::SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        let settings = ServerSettings {
            name: "t".into(),
            listen,
            tls: None,
            next_connection_delay_ms: 1,
            connections: ConnectionSettings {
                connection_timeout_ms: 60_000,
                max_payload_size: 256 * 1024,
                max_inflight_count: 100,
                auth: None,
                external_auth: None,
                dynamic_filters: true,
            },
        };
        let router = Router::new(
            0,
            RouterConfig {
                max_connections: 64,
                max_outgoing_packet_count: 200,
                max_segment_size: 1024 * 1024,
                max_segment_count: 4,
                custom_segment: None,
                initialized_filters: None,
                shared_subscriptions_strategy: Default::default(),
            },
        );
        let tx = router.spawn();
        let mut server = Server::new(settings, tx, V4);
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            rt.block_on(async move {
                let _ = server.start(LinkType::Remote).await;
            });
        });
        for _ in 0..50 {
            if std::net::TcpStream::connect(listen).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        port
    }

    #[test]
    fn edge_births_and_data_reach_a_host_and_commands_come_back() {
        let port = broker();
        let (hlog, hhost) = testhost::host();
        let mut host = ExtPack::new(hhost, "sparkplug", run, endpoint, check);
        host.configure(
            &serde_json::json!({
                "connections":[{"id":"scada","role":"host","host":"127.0.0.1","port":port,"group_id":"Plant"}],
                "inputs":[{"connection":"scada","binds":[
                    {"device":"","metric":"Line speed","point":"speed","source":"sparkplug/scada::Line speed"},
                    {"device":"Robot1","metric":"Torque","point":"torque","source":"sparkplug/scada::Robot1:Torque"}]}],
                "outputs":[{"connection":"scada","binds":[{"edge":"Line2","device":"","metric":"Speed setpoint","point":"sp"}]}]
            })
            .to_string(),
        )
        .unwrap();
        testhost::wait(hlog, "host up", |_| host.is_connected());
        std::thread::sleep(Duration::from_millis(200));
        let (elog, ehost) = testhost::host();
        let mut edge = ExtPack::new(ehost, "sparkplug", run, endpoint, check);
        edge.write_point("speed_pv", "12.5");
        edge.configure(
            &serde_json::json!({
                "connections":[{"id":"line","role":"edge","host":"127.0.0.1","port":port,"group_id":"Plant","edge_node_id":"Line2"}],
                "inputs":[{"connection":"line","binds":[{"device":"","metric":"Speed setpoint","point":"line_sp","source":"sparkplug/line::Speed setpoint"}]}],
                "outputs":[{"connection":"line","interval_secs":0.05,"binds":[
                    {"device":"","metric":"Line speed","point":"speed_pv"},
                    {"device":"Robot1","metric":"Torque","point":"robot_tq"}]}]
            })
            .to_string(),
        )
        .unwrap();
        // NBIRTH carries the current value.
        testhost::wait(hlog, "speed from birth", |l| l.iter().any(|r| r.name == "speed" && r.value == Val::Float(12.5)));
        // DDATA (by alias) for the device metric.
        edge.write_point("robot_tq", "41");
        testhost::wait(hlog, "torque", |l| l.iter().any(|r| r.name == "torque" && r.value == Val::Float(41.0)));
        // Host command → NCMD → edge Input.
        host.write_point("sp", "15");
        testhost::wait(elog, "setpoint", |l| l.iter().any(|r| r.name == "line_sp" && r.value == Val::Int(15)));
        // Edge leaves → NDEATH → host points bad.
        edge.shutdown();
        testhost::wait(hlog, "death", |l| l.iter().any(|r| r.name == "speed" && r.quality == pb_abi::PB_QUALITY_BAD));
        let d: Value = serde_json::from_str(&host.detail_json()).unwrap();
        assert!(d["connections"][0]["seen"].as_array().unwrap().iter().any(|x| x["leaf"]["metric"] == "Torque"));
    }

    #[test]
    fn edge_needs_ids() {
        let c = serde_json::json!({"role":"edge","group_id":"G"});
        assert!(check(c.as_object().unwrap()).is_err());
        let c = serde_json::json!({"role":"host","group_id":"a/b"});
        assert!(check(c.as_object().unwrap()).is_err());
    }
}
