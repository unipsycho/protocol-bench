//! One JSON path syntax for every pack that digs a field out of a JSON document.
//!
//! Accepted forms (all equivalent ways to write the same lookup):
//!
//! | Path | Meaning |
//! | --- | --- |
//! | `` / `$` / `.` | the whole document |
//! | `room.temp` | object key chain |
//! | `$.room.temp`, `.room.temp` | leading `$.` / `.` is ignored |
//! | `tags[0].id`, `tags.0.id` | array index, bracket or dotted |
//! | `a[0][1]` | nested arrays |
//!
//! A numeric segment on an *object* falls back to the key of that name (`{"0": …}`), so
//! `readings.0` works on both arrays and objects. Missing keys, out-of-range indexes and walking
//! into a scalar all return `None`; nothing here panics on odd input.

use serde_json::Value;

/// Rewrite a path into plain dotted form: strip `$` / `$.`, turn `foo[0].bar` into `foo.0.bar`.
pub fn normalize_path(path: &str) -> String {
    let p = path.trim();
    let p = p.strip_prefix("$.").unwrap_or(p);
    let p = if p == "$" { "" } else { p.strip_prefix('$').unwrap_or(p) };
    let p = p.strip_prefix('.').unwrap_or(p);
    let mut out = String::with_capacity(p.len());
    let mut chars = p.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '[' {
            let mut idx = String::new();
            for d in chars.by_ref() {
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

/// Walk `path` through `root`. `None` when any step is missing.
pub fn get_path<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    if path.matches('[').count() != path.matches(']').count() {
        return None; // unterminated bracket: refuse rather than quietly read the wrong field
    }
    let norm = normalize_path(path);
    let mut cur = root;
    for seg in norm.split('.').map(str::trim).filter(|s| !s.is_empty()) {
        cur = match seg.parse::<usize>() {
            Ok(i) => cur.get(i).or_else(|| cur.get(seg))?,
            Err(_) => cur.get(seg)?,
        };
    }
    Some(cur)
}

/// Parse `payload` as JSON and render the value at `path` as text: strings unquoted, numbers /
/// booleans as written, `null` as `"null"`, objects and arrays as compact JSON.
pub fn extract_text(payload: &str, path: &str) -> Option<String> {
    let root: Value = serde_json::from_str(payload.trim()).ok()?;
    Some(value_text(get_path(&root, path)?))
}

/// Text form used for points: see [`extract_text`].
pub fn value_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "null".into(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn doc() -> Value {
        json!({"room": {"temp": 21.5}, "tags": [{"id": "a"}, {"id": "b"}], "grid": [[1, 2], [3, 4]], "map": {"0": "zero"}})
    }

    #[test]
    fn every_spelling_of_a_path_agrees() {
        let d = doc();
        for p in ["room.temp", "$.room.temp", ".room.temp", " room . temp ", "$room.temp"] {
            assert_eq!(get_path(&d, p), Some(&json!(21.5)), "{p}");
        }
        assert_eq!(get_path(&d, "tags[1].id"), Some(&json!("b")));
        assert_eq!(get_path(&d, "tags.1.id"), Some(&json!("b")));
        assert_eq!(get_path(&d, "grid[1][0]"), Some(&json!(3)));
        assert_eq!(get_path(&d, "map.0"), Some(&json!("zero")), "numeric segment falls back to an object key");
    }

    #[test]
    fn whole_document_paths() {
        let d = doc();
        for p in ["", "$", "  ", "."] {
            assert_eq!(get_path(&d, p), Some(&d), "{p:?}");
        }
    }

    #[test]
    fn missing_things_are_none_not_panics() {
        let d = doc();
        for p in ["nope", "room.temp.x", "tags[9]", "tags[x]", "tags[", "room[0]"] {
            assert_eq!(get_path(&d, p), None, "{p}");
        }
    }

    #[test]
    fn text_rendering() {
        let p = r#"{"s":"hi","n":1,"b":true,"z":null,"o":{"a":1}}"#;
        assert_eq!(extract_text(p, "s").as_deref(), Some("hi"));
        assert_eq!(extract_text(p, "n").as_deref(), Some("1"));
        assert_eq!(extract_text(p, "b").as_deref(), Some("true"));
        assert_eq!(extract_text(p, "z").as_deref(), Some("null"));
        assert_eq!(extract_text(p, "o").as_deref(), Some(r#"{"a":1}"#));
        assert_eq!(extract_text("not json", "s"), None);
    }
}
