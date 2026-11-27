//! Point source ("wire") addresses — the one place their formats are defined.
//!
//! The UI builds these strings when it lists Input binds, and the service builds the *same* strings
//! when it configures a pack; a pack tags every point it publishes with its address, and the host
//! point store matches on it. If the two sides ever disagree a bind silently never gets a value,
//! so both call these functions.
//!
//! | Protocol | Format |
//! | --- | --- |
//! | MQTT | `mqtt/<conn>/<topic>[::<json path>]` |
//! | HTTP | `http/<conn>/<url path>[::<json path>]` |
//! | Streams | `stream/<endpoint>[::<field>]` |
//! | Modbus | `modbus/<conn>[/u<unit>]::<area>:<addr>:<type>[:<order>]` (order only when not `abcd`) |
//! | File follow | `file/<source>[::<column>]` |
//! | SNMP | `snmp/<target>[::<oid>]` |
//! | LAS | `las/<endpoint>[::<curve>]` |
//! | WITS | `wits/<conn>::<item>` (4-digit record + item, `0108`) |
//! | WebSocket | `websocket/<conn>::<json path>` |
//! | Sparkplug B | `sparkplug/<conn>::<device>:<metric>` (device empty for node metrics) |
//! | DNP3 | `dnp3/<conn>::<kind>:<index>` (`analog:3`, `binary:0`, `counter:1`, …) |
//! | CAN | `can/<conn>::<id>:<start>:<len>` (frame id as written, start bit, bit length) |
//!
//! An empty connection drops that segment (`mqtt/<topic>`); an empty leaf drops the `::` part.
//! Every input is trimmed.

/// `proto[/instance][::leaf]`, all parts trimmed, empty parts omitted.
fn join(proto: &str, instance: &str, leaf: &str) -> String {
    let (instance, leaf) = (instance.trim(), leaf.trim());
    let mut out = proto.to_string();
    if !instance.is_empty() {
        out.push('/');
        out.push_str(instance);
    }
    if !leaf.is_empty() {
        out.push_str("::");
        out.push_str(leaf);
    }
    out
}

/// `proto[/conn][/path][::leaf]` for protocols whose path is part of the identity (MQTT, HTTP).
fn join_path(proto: &str, conn: &str, path: &str, leaf: &str) -> String {
    let (conn, path) = (conn.trim(), path.trim().trim_start_matches('/'));
    let instance = match (conn.is_empty(), path.is_empty()) {
        (true, true) => String::new(),
        (true, false) => path.to_string(),
        (false, true) => conn.to_string(),
        (false, false) => format!("{conn}/{path}"),
    };
    join(proto, &instance, leaf)
}

/// `mqtt/<conn>/<topic>[::<json path>]`.
pub fn mqtt_bind_source(conn: &str, topic: &str, json_path: &str) -> String {
    join_path("mqtt", conn, topic, json_path)
}

/// `http/<conn>/<url path>[::<json path>]`.
pub fn http_bind_source(conn: &str, url_path: &str, json_path: &str) -> String {
    join_path("http", conn, url_path, json_path)
}

/// `stream/<endpoint>[::<field>]`.
pub fn stream_bind_source(endpoint: &str, field: &str) -> String {
    join("stream", endpoint, field)
}

/// `file/<source>[::<column>]`.
pub fn file_bind_source(source: &str, column: &str) -> String {
    join("file", source, column)
}

/// `snmp/<target>[::<oid>]`.
pub fn snmp_bind_source(target: &str, oid: &str) -> String {
    join("snmp", target, oid)
}

/// `las/<endpoint>[::<curve>]`.
pub fn las_bind_source(endpoint: &str, curve: &str) -> String {
    join("las", endpoint, curve)
}

/// `unit == 0` means "the connection's default unit id" and is omitted from the address.
pub fn modbus_bind_source(conn: &str, unit: u64, area: &str, address: u64, dtype: &str, order: &str) -> String {
    let conn = conn.trim();
    let mut instance = conn.to_string();
    if unit != 0 {
        instance.push_str(&format!("/u{unit}"));
    }
    let mut leaf = format!("{area}:{address}:{dtype}");
    if !order.trim().is_empty() && !order.eq_ignore_ascii_case("abcd") {
        leaf.push(':');
        leaf.push_str(&order.to_ascii_lowercase());
    }
    // A unit with no connection still needs its own segment: `modbus/u3::…`.
    join("modbus", instance.trim_start_matches('/'), &leaf)
}

/// A protocol whose binds the UI and service handle generically ("ext" protocols). The UI
/// adds field schemas on top; the service only needs the section key, the point-id prefix and
/// which bind fields make up the wire address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtSpec {
    /// Project section key, pack id, address prefix, and (case-insensitively) hub port protocol.
    pub id: &'static str,
    /// Short point id prefix: `wits` → `wits_1`.
    pub prefix: &'static str,
    /// Bind fields joined with `:` to form the address leaf. Empty values are skipped.
    pub leaf_keys: &'static [&'static str],
    /// Fields that must be non-empty for a bind to count. Empty list = a bind with every leaf
    /// field empty is valid (WebSocket: the whole message).
    pub required: &'static [&'static str],
}

pub const EXT_PROTOCOLS: &[ExtSpec] = &[
    ExtSpec { id: "wits", prefix: "wits", leaf_keys: &["item"], required: &["item"] },
    ExtSpec { id: "websocket", prefix: "ws", leaf_keys: &["path"], required: &[] },
    ExtSpec { id: "sparkplug", prefix: "spb", leaf_keys: &["device", "metric"], required: &["metric"] },
    ExtSpec { id: "dnp3", prefix: "dnp3", leaf_keys: &["kind", "index"], required: &["kind", "index"] },
    ExtSpec { id: "can", prefix: "can", leaf_keys: &["id", "start", "len"], required: &["id", "start", "len"] },
    ExtSpec { id: "gen", prefix: "gen", leaf_keys: &["name"], required: &["name"] },
    // Output-only for now; Input binds (control widgets) arrive with write-back.
    ExtSpec { id: "dashboard", prefix: "dash", leaf_keys: &[], required: &[] },
];

/// Spec for a section key / port protocol name (case-insensitive).
pub fn ext_spec(id: &str) -> Option<&'static ExtSpec> {
    EXT_PROTOCOLS.iter().find(|s| s.id.eq_ignore_ascii_case(id.trim()))
}

/// Address leaf for an ext bind: the `leaf_keys` values (via `get`), trimmed, empties skipped,
/// joined with `:`. `None` when a required field is empty (the bind is blank).
pub fn ext_leaf(spec: &ExtSpec, get: impl Fn(&str) -> String) -> Option<String> {
    let parts: Vec<String> = spec
        .leaf_keys
        .iter()
        .map(|k| get(k).trim().to_string())
        .filter(|v| !v.is_empty())
        .collect();
    let ok = spec.required.iter().all(|k| !get(k).trim().is_empty());
    ok.then(|| parts.join(":"))
}

/// `<id>/<conn>::<leaf>`.
pub fn ext_bind_source(spec: &ExtSpec, conn: &str, leaf: &str) -> String {
    join(spec.id, conn, leaf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ext_protocol_addresses() {
        let wits = ext_spec("WITS").unwrap();
        let leaf = ext_leaf(wits, |k| if k == "item" { " 0108 ".into() } else { String::new() }).unwrap();
        assert_eq!(ext_bind_source(wits, "van", &leaf), "wits/van::0108");
        let spb = ext_spec("Sparkplug").unwrap();
        let get = |d: &'static str, m: &'static str| move |k: &str| if k == "device" { d.to_string() } else if k == "metric" { m.to_string() } else { String::new() };
        assert_eq!(ext_leaf(spb, get("Pump1", "Pressure")).unwrap(), "Pump1:Pressure");
        assert_eq!(ext_leaf(spb, get("", "Node/Uptime")).unwrap(), "Node/Uptime");
        assert_eq!(ext_leaf(spb, get("Pump1", "")), None);
        let dnp = ext_spec("dnp3").unwrap();
        let leaf = ext_leaf(dnp, |k| if k == "kind" { "analog".into() } else { "3".into() }).unwrap();
        assert_eq!(ext_bind_source(dnp, "rtu", &leaf), "dnp3/rtu::analog:3");
        assert!(ext_spec("nope").is_none());
        let ws = ext_spec("websocket").unwrap();
        assert_eq!(ext_leaf(ws, |_| String::new()).as_deref(), Some(""));
        assert_eq!(ext_bind_source(ws, "feed", ""), "websocket/feed");
        let can = ext_spec("can").unwrap();
        assert_eq!(ext_leaf(can, |k| if k == "id" { String::new() } else { "8".into() }), None);
        // Prefixes are unique so point ids never collide across protocols.
        let mut seen = std::collections::HashSet::new();
        for s in EXT_PROTOCOLS {
            assert!(seen.insert(s.prefix));
        }
    }

    #[test]
    fn mqtt_and_http_paths() {
        assert_eq!(mqtt_bind_source("local", "a/b", ""), "mqtt/local/a/b");
        assert_eq!(mqtt_bind_source("local", " a/b ", " temp "), "mqtt/local/a/b::temp");
        assert_eq!(mqtt_bind_source("", "a/b", ""), "mqtt/a/b");
        assert_eq!(http_bind_source("api", "/v1/tank", "level"), "http/api/v1/tank::level");
        assert_eq!(http_bind_source("", "", ""), "http");
        assert_eq!(http_bind_source("api", "", ""), "http/api");
        assert_eq!(http_bind_source("", "v1", "x"), "http/v1::x");
    }

    #[test]
    fn simple_instance_leaf_protocols() {
        assert_eq!(stream_bind_source("com1", "2"), "stream/com1::2");
        assert_eq!(stream_bind_source("", ""), "stream");
        assert_eq!(file_bind_source("meter", "temp"), "file/meter::temp");
        assert_eq!(file_bind_source("", "temp"), "file::temp");
        assert_eq!(file_bind_source("meter", ""), "file/meter");
        assert_eq!(snmp_bind_source("sw", " 1.3.6.1.2.1.1.5.0 "), "snmp/sw::1.3.6.1.2.1.1.5.0");
        assert_eq!(snmp_bind_source("", "1.3"), "snmp::1.3");
        assert_eq!(las_bind_source("well-a", " GR "), "las/well-a::GR");
        assert_eq!(las_bind_source("well-a", ""), "las/well-a");
    }

    #[test]
    fn modbus_addresses() {
        assert_eq!(modbus_bind_source("plc", 0, "holding", 100, "f32", "abcd"), "modbus/plc::holding:100:f32");
        assert_eq!(modbus_bind_source("plc", 0, "holding", 100, "f32", "CDAB"), "modbus/plc::holding:100:f32:cdab");
        assert_eq!(modbus_bind_source("plc", 7, "coil", 3, "bool", "abcd"), "modbus/plc/u7::coil:3:bool");
        assert_eq!(modbus_bind_source("", 0, "input", 0, "u16", "abcd"), "modbus::input:0:u16");
        assert_eq!(modbus_bind_source("", 3, "input", 0, "u16", ""), "modbus/u3::input:0:u16");
    }
}
