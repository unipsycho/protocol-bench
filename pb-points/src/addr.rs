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

#[cfg(test)]
mod tests {
    use super::*;

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
