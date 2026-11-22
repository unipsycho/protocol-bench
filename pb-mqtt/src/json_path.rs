//! Dotted / JSONPath-ish field extraction for MQTT JSON payloads.
//! Pattern aligned with TopicBench `mqtt/value_path.rs` (no UI).
//!
//! Point ids are always `mqtt/<client_name>/<topic>` (and `…::path` for JSON fields).


/// Point id for a whole topic on one client: `mqtt/<client>/<topic>`.
pub fn point_id_for_client_topic(client: &str, topic: &str) -> String {
    format!("mqtt/{client}/{topic}")
}

/// Point id for a JSON field: `mqtt/<client>/<topic>::<path>`.
pub fn point_id_for_client_field(client: &str, topic: &str, path: &str) -> String {
    let path = pb_json::normalize_path(path);
    format!("mqtt/{client}/{topic}::{path}")
}

/// Split `mqtt/<client>/<topic>[::path]` into (client, topic, optional path).
/// Legacy `mqtt/<topic>[::path]` (no known client segment) returns `None` for client.
pub fn split_point_id<'a>(
    name: &'a str,
    known_clients: &[String],
) -> (Option<&'a str>, &'a str, Option<&'a str>) {
    let rest = name.strip_prefix("mqtt/").unwrap_or(name);
    let (body, path) = match rest.split_once("::") {
        Some((b, p)) if !p.is_empty() => (b, Some(p)),
        _ => (rest, None),
    };
    if let Some((first, topic)) = body.split_once('/') {
        if !topic.is_empty() && known_clients.iter().any(|c| c == first) {
            return (Some(first), topic, path);
        }
    }
    // Legacy or unknown: entire body is the topic.
    (None, body, path)
}

/// Rewrite legacy `mqtt/<topic>[::path]` → `mqtt/<client>/<topic>[::path]`.
/// Already-scoped ids (`mqtt/<known_client>/…`) are left unchanged.
pub fn migrate_point_id(name: &str, known_clients: &[String], default_client: &str) -> String {
    let name = name.trim();
    if name.is_empty() || !name.starts_with("mqtt/") {
        return name.to_string();
    }
    let (client, topic, path) = split_point_id(name, known_clients);
    if client.is_some() {
        return name.to_string();
    }
    if topic.is_empty() {
        return name.to_string();
    }
    let client = if default_client.is_empty() {
        "default"
    } else {
        default_client
    };
    match path {
        Some(p) => point_id_for_client_field(client, topic, p),
        None => point_id_for_client_topic(client, topic),
    }
}

/// MQTT-style topic filter match (`+` single level, `#` multi-level).
pub fn topic_matches(filter: &str, topic: &str) -> bool {
    let filter = filter.trim();
    let topic = topic.trim();
    if filter.is_empty() || filter == "#" {
        return true;
    }
    let f: Vec<&str> = filter.split('/').collect();
    let t: Vec<&str> = topic.split('/').collect();
    let mut fi = 0;
    let mut ti = 0;
    while fi < f.len() {
        if f[fi] == "#" {
            return fi + 1 == f.len();
        }
        if ti >= t.len() {
            return false;
        }
        if f[fi] != "+" && f[fi] != t[ti] {
            return false;
        }
        fi += 1;
        ti += 1;
    }
    ti == t.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topic_filter_wildcards() {
        assert!(topic_matches("a/+/c", "a/b/c"));
        assert!(topic_matches("a/#", "a/b/c"));
        assert!(topic_matches("#", "any/topic"));
        assert!(topic_matches("+", "one"));
        assert!(!topic_matches("+", "one/two"));
        assert!(topic_matches("pb/plus/+/x", "pb/plus/mid/x"));
        assert!(!topic_matches("pb/plus/+/x", "pb/plus/mid/y"));
        assert!(topic_matches("pb/wc/#", "pb/wc/deep/leaf"));
    }

    #[test]
    fn client_scoped_ids() {
        assert_eq!(
            point_id_for_client_topic("local", "sensors/1"),
            "mqtt/local/sensors/1"
        );
        assert_eq!(
            point_id_for_client_field("local", "sensors/1", "temp"),
            "mqtt/local/sensors/1::temp"
        );
    }

    #[test]
    fn migrates_legacy_topic_ids() {
        let clients = vec!["local".into()];
        assert_eq!(
            migrate_point_id("mqtt/pb/in", &clients, "local"),
            "mqtt/local/pb/in"
        );
        assert_eq!(
            migrate_point_id("mqtt/pb/in::temp", &clients, "local"),
            "mqtt/local/pb/in::temp"
        );
        // Already scoped — unchanged.
        assert_eq!(
            migrate_point_id("mqtt/local/pb/in", &clients, "local"),
            "mqtt/local/pb/in"
        );
    }
}
