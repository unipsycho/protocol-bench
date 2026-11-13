//! Dotted / JSONPath-ish field extraction for MQTT JSON payloads.
//! Pattern aligned with TopicBench `mqtt/value_path.rs` (no UI).

use serde_json::Value;

/// Point id for a whole topic payload: `mqtt/<topic>`.
pub fn point_id_for_topic(topic: &str) -> String {
    format!("mqtt/{topic}")
}

/// Point id for a JSON field on a topic: `mqtt/<topic>::<path>`.
pub fn point_id_for_field(topic: &str, path: &str) -> String {
    let path = normalize_field_path(path);
    format!("mqtt/{topic}::{path}")
}

/// Topic (+ optional `::path`) from a point id.
pub fn split_point_id(name: &str) -> (&str, Option<&str>) {
    let rest = name.strip_prefix("mqtt/").unwrap_or(name);
    match rest.split_once("::") {
        Some((topic, path)) if !path.is_empty() => (topic, Some(path)),
        _ => (rest, None),
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
        assert_eq!(
            extract_path_value(payload, "$.room.temp").as_deref(),
            Some("21.5")
        );
    }

    #[test]
    fn topic_filter_wildcards() {
        assert!(topic_matches("a/+/c", "a/b/c"));
        assert!(!topic_matches("a/+/c", "a/b/c/d"));
        assert!(topic_matches("a/#", "a/b/c"));
        assert!(topic_matches("sensors/room1", "sensors/room1"));
        assert!(!topic_matches("sensors/room1", "sensors/room2"));
    }

    #[test]
    fn field_point_id() {
        assert_eq!(
            point_id_for_field("sensors/1", "temp"),
            "mqtt/sensors/1::temp"
        );
    }
}
