//! Dotted / JSONPath-ish field extraction for MQTT JSON payloads.
//! Pattern aligned with TopicBench `mqtt/value_path.rs` (no UI).
//!
//! Point ids are always `mqtt/<client_name>/<topic>` (and `…::path` for JSON fields).

use serde_json::Value;

/// Point id for a whole topic on one client: `mqtt/<client>/<topic>`.
pub fn point_id_for_client_topic(client: &str, topic: &str) -> String {
    format!("mqtt/{client}/{topic}")
}

/// Point id for a JSON field: `mqtt/<client>/<topic>::<path>`.
pub fn point_id_for_client_field(client: &str, topic: &str, path: &str) -> String {
    let path = normalize_field_path(path);
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

/// Normalize a field path: strip `$` / `$.`, turn `foo[0].bar` into `foo.0.bar`.
pub fn normalize_field_path(path: &str) -> String {
    let p = path.trim();
    let p = p.strip_prefix("$.").unwrap_or(p);
    let p = if p == "$" {
        ""
    } else {
        p.strip_prefix('$').unwrap_or(p)
    };
    let mut out = String::with_capacity(p.len());
    let mut chars = p.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '[' {
            let mut idx = String::new();
            while let Some(&d) = chars.peek() {
                chars.next();
                if d == ']' {
                    break;
                }
                idx.push(d);
            }
            let idx = idx.trim();
            if idx.is_empty() {
                continue;
            }
            if !out.is_empty() && !out.ends_with('.') {
                out.push('.');
            }
            out.push_str(idx);
        } else {
            out.push(c);
        }
    }
    out
}

/// Walk `dotted.path` (objects + numeric array segments) on a JSON value.
pub fn get_at_path<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    let norm = normalize_field_path(path);
    if norm.is_empty() {
        return Some(root);
    }
    let mut cur = root;
    for seg in norm.split('.').filter(|s| !s.is_empty()) {
        cur = if let Ok(i) = seg.parse::<usize>() {
            cur.get(i).or_else(|| cur.get(seg))?
        } else {
            cur.get(seg)?
        };
    }
    Some(cur)
}

/// Extract a scalar (or compact JSON) at `path` from a UTF-8 JSON payload.
pub fn extract_path_value(payload: &str, path: &str) -> Option<String> {
    let root: Value = serde_json::from_str(payload.trim()).ok()?;
    let cur = get_at_path(&root, path)?;
    Some(match cur {
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        Value::Null => "null".into(),
        other => other.to_string(),
    })
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
    fn extracts_nested_and_array() {
        let payload = r#"{"room":{"temp":21.5},"tags":[{"id":"a"}]}"#;
        assert_eq!(
            extract_path_value(payload, "room.temp").as_deref(),
            Some("21.5")
        );
        assert_eq!(
            extract_path_value(payload, "tags[0].id").as_deref(),
            Some("a")
        );
    }

    #[test]
    fn topic_filter_wildcards() {
        assert!(topic_matches("a/+/c", "a/b/c"));
        assert!(topic_matches("a/#", "a/b/c"));
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
