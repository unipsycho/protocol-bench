//! Row parsing: CSV lines, JSON-lines paths, and typing of text cells.

use serde_json::Value as Json;

/// Value as it goes to the host point bus.
#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
}

/// Type a text cell: `true`/`false` → bool, whole numbers → int, decimals → float, else text.
pub fn type_text(raw: &str) -> Cell {
    let t = raw.trim();
    match t.to_ascii_lowercase().as_str() {
        "true" => return Cell::Bool(true),
        "false" => return Cell::Bool(false),
        _ => {}
    }
    if let Ok(n) = t.parse::<i64>() {
        return Cell::Int(n);
    }
    if !t.is_empty() {
        if let Ok(x) = t.parse::<f64>() {
            if x.is_finite() {
                return Cell::Float(x);
            }
        }
    }
    Cell::Text(raw.trim().to_string())
}

/// Split one CSV line. Handles `"quoted, fields"` and `""` escapes; a quote never spans lines
/// (multi-line fields are not supported — one physical line is one row).
pub fn split_csv_line(line: &str, delim: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if in_q {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    cur.push('"');
                    chars.next();
                } else {
                    in_q = false;
                }
            } else {
                cur.push(c);
            }
        } else if c == '"' && cur.is_empty() {
            in_q = true;
        } else if c == delim {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    out.push(cur);
    out
}

/// Resolve a CSV column spec to an index: header name (exact, then case-insensitive), else a
/// 0-based number.
pub fn column_index(header: Option<&[String]>, spec: &str) -> Option<usize> {
    let spec = spec.trim();
    if let Some(h) = header {
        if let Some(i) = h.iter().position(|c| c.trim() == spec) {
            return Some(i);
        }
        if let Some(i) = h.iter().position(|c| c.trim().eq_ignore_ascii_case(spec)) {
            return Some(i);
        }
    }
    spec.parse::<usize>().ok()
}

/// Follow a dotted path with optional `[n]` indices: `data.temps[1].value` (syntax: see `pb_json`).
pub use pb_json::get_path as json_path;

/// Leaf paths of a JSON row with sample values, for the column picker: `data.temps[0] = 21.5`.
/// Arrays contribute their first few elements; the walk stops after `limit` leaves.
pub fn json_leaves(v: &Json, limit: usize) -> Vec<(String, String)> {
    fn walk(v: &Json, path: String, out: &mut Vec<(String, String)>, limit: usize) {
        if out.len() >= limit {
            return;
        }
        match v {
            Json::Object(m) if !m.is_empty() => {
                for (k, x) in m {
                    walk(x, if path.is_empty() { k.clone() } else { format!("{path}.{k}") }, out, limit);
                }
            }
            Json::Array(a) if !a.is_empty() => {
                for (i, x) in a.iter().take(3).enumerate() {
                    walk(x, format!("{path}[{i}]"), out, limit);
                }
            }
            Json::String(s) => out.push((path, s.clone())),
            Json::Null => out.push((path, String::new())),
            other => out.push((path, other.to_string())),
        }
    }
    let mut out = Vec::new();
    walk(v, String::new(), &mut out, limit);
    out
}

/// CSV columns for the picker: `(name, sample)` from the header (or numbered when the file has
/// no header row) and the latest data row.
pub fn csv_columns(header: Option<&[String]>, row: &[String]) -> Vec<(String, String)> {
    let n = header.map(|h| h.len()).unwrap_or(0).max(row.len());
    (0..n)
        .map(|i| {
            let name = header.and_then(|h| h.get(i)).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| i.to_string());
            (name, row.get(i).cloned().unwrap_or_default())
        })
        .collect()
}

/// JSON value → point cell. Objects / arrays become compact JSON text.
pub fn json_cell(v: &Json) -> Cell {
    match v {
        Json::Bool(b) => Cell::Bool(*b),
        Json::Number(n) => match n.as_i64() {
            Some(i) => Cell::Int(i),
            None => Cell::Float(n.as_f64().unwrap_or(0.0)),
        },
        Json::String(s) => type_text(s),
        Json::Null => Cell::Text(String::new()),
        other => Cell::Text(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_quotes_and_delimiters() {
        assert_eq!(split_csv_line("a,b,c", ','), vec!["a", "b", "c"]);
        assert_eq!(split_csv_line(r#"1,"x, y",3"#, ','), vec!["1", "x, y", "3"]);
        assert_eq!(split_csv_line(r#""say ""hi""",2"#, ','), vec![r#"say "hi""#, "2"]);
        assert_eq!(split_csv_line("a;b;;d", ';'), vec!["a", "b", "", "d"]);
        assert_eq!(split_csv_line("", ','), vec![""]);
        assert_eq!(split_csv_line("a\tb", '\t'), vec!["a", "b"]);
        // A quote in the middle of an unquoted field is literal.
        assert_eq!(split_csv_line("5\" pipe,2", ','), vec!["5\" pipe", "2"]);
    }

    #[test]
    fn cells_are_typed() {
        assert_eq!(type_text(" 42 "), Cell::Int(42));
        assert_eq!(type_text("-7"), Cell::Int(-7));
        assert_eq!(type_text("3.25"), Cell::Float(3.25));
        assert_eq!(type_text("1e3"), Cell::Float(1000.0));
        assert_eq!(type_text("TRUE"), Cell::Bool(true));
        assert_eq!(type_text("false"), Cell::Bool(false));
        assert_eq!(type_text("N/A"), Cell::Text("N/A".into()));
        assert_eq!(type_text("nan"), Cell::Text("nan".into()));
        assert_eq!(type_text(""), Cell::Text(String::new()));
    }

    #[test]
    fn column_by_name_case_insensitive_or_index() {
        let h: Vec<String> = ["Time", "Temp", "Flow"].iter().map(|s| s.to_string()).collect();
        assert_eq!(column_index(Some(&h), "Temp"), Some(1));
        assert_eq!(column_index(Some(&h), "flow"), Some(2));
        assert_eq!(column_index(Some(&h), "2"), Some(2));
        assert_eq!(column_index(Some(&h), "nope"), None);
        assert_eq!(column_index(None, "3"), Some(3));
        assert_eq!(column_index(None, "Temp"), None);
    }

    #[test]
    fn picker_columns_from_csv_and_json_rows() {
        let h: Vec<String> = ["time", "temp", ""].iter().map(|s| s.to_string()).collect();
        let row: Vec<String> = ["10:00", "21.5", "x", "extra"].iter().map(|s| s.to_string()).collect();
        assert_eq!(
            csv_columns(Some(&h), &row),
            vec![("time".into(), "10:00".into()), ("temp".into(), "21.5".into()), ("2".into(), "x".into()), ("3".into(), "extra".into())]
        );
        assert_eq!(csv_columns(None, &row)[1], ("1".to_string(), "21.5".to_string()));
        let v: Json = serde_json::from_str(r#"{"a":{"b":[1,2,3,4]},"ok":true,"s":"txt","n":null,"e":{}}"#).unwrap();
        let l = json_leaves(&v, 50);
        let names: Vec<&str> = l.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"a.b[0]") && names.contains(&"a.b[2]") && !names.contains(&"a.b[3]"));
        assert!(l.contains(&("ok".to_string(), "true".to_string())));
        assert!(l.contains(&("s".to_string(), "txt".to_string())));
        assert_eq!(json_leaves(&v, 2).len(), 2, "limit respected");
    }

    #[test]
    fn json_paths_with_indices() {
        let v: Json = serde_json::from_str(r#"{"a":{"b":[10,{"c":"7.5"}]},"ok":true,"n":null}"#).unwrap();
        assert_eq!(json_path(&v, "a.b[0]"), Some(&Json::from(10)));
        assert_eq!(json_cell(json_path(&v, "a.b[1].c").unwrap()), Cell::Float(7.5));
        assert_eq!(json_cell(json_path(&v, "ok").unwrap()), Cell::Bool(true));
        assert_eq!(json_cell(json_path(&v, "n").unwrap()), Cell::Text(String::new()));
        assert!(json_path(&v, "a.x").is_none());
        assert!(json_path(&v, "a.b[9]").is_none());
        assert!(json_path(&v, "a.b[x]").is_none());
        assert_eq!(json_path(&v, "$"), Some(&v));
        assert_eq!(json_cell(json_path(&v, "a.b[1]").unwrap()), Cell::Text(r#"{"c":"7.5"}"#.into()));
    }
}
