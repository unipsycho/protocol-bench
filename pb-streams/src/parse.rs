//! Split a framed text line into fields (UI/pack shared semantics).

pub fn sample_fields(sample: &str, parser: &str, widths: &str, regex: &str) -> Vec<String> {
    let line = sample.lines().next().unwrap_or(sample).trim_end_matches('\r');
    if line.is_empty() {
        return Vec::new();
    }
    match parser {
        "csv" => split_csv_like(line, ','),
        "tsv" => split_csv_like(line, '\t'),
        "whitespace" => line.split_whitespace().map(|s| s.to_string()).collect(),
        "fixed" => split_fixed(line, widths),
        "regex" => split_regex(line, regex),
        _ => vec![line.to_string()], // whole_line
    }
}

fn split_csv_like(line: &str, delim: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    for ch in line.chars() {
        if ch == '"' {
            in_q = !in_q;
            continue;
        }
        if ch == delim && !in_q {
            out.push(std::mem::take(&mut cur));
            continue;
        }
        cur.push(ch);
    }
    out.push(cur);
    out
}

fn split_fixed(line: &str, widths: &str) -> Vec<String> {
    let widths: Vec<usize> = widths
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .filter(|w| *w > 0)
        .collect();
    if widths.is_empty() {
        return vec![line.to_string()];
    }
    let mut out = Vec::new();
    let mut rest = line;
    for w in widths {
        if rest.is_empty() {
            out.push(String::new());
            continue;
        }
        let (take, next) = rest.split_at(w.min(rest.len()));
        out.push(take.trim().to_string());
        rest = next;
    }
    out
}

fn split_regex(line: &str, pattern: &str) -> Vec<String> {
    let pat = pattern.trim();
    if pat.is_empty() {
        return vec![line.to_string()];
    }
    let Ok(re) = regex::Regex::new(pat) else {
        return vec![line.to_string()];
    };
    let Some(caps) = re.captures(line) else {
        return Vec::new();
    };
    // Prefer named/numbered capture groups (skip whole match at 0).
    if caps.len() > 1 {
        return (1..caps.len())
            .map(|i| caps.get(i).map(|m| m.as_str().to_string()).unwrap_or_default())
            .collect();
    }
    vec![caps.get(0).map(|m| m.as_str().to_string()).unwrap_or_default()]
}

/// Join emit fields for an outbound line.
pub fn join_fields(layout: &str, values: &[String]) -> String {
    match layout {
        "tsv" => values.join("\t"),
        "whitespace" => values
            .iter()
            .filter(|v| !v.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join(" "),
        "whole_line" => values.first().cloned().unwrap_or_default(),
        "fixed" => values
            .iter()
            .map(|v| format!("{:<8}", truncate(v, 8)))
            .collect(),
        _ => values.join(","), // csv
    }
}

pub fn trailer_bytes(eol: &str, custom: &str) -> Vec<u8> {
    match eol {
        "crlf" => b"\r\n".to_vec(),
        "cr" => b"\r".to_vec(),
        "none" => Vec::new(),
        "custom" => custom.as_bytes().to_vec(),
        _ => b"\n".to_vec(),
    }
}

fn truncate(value: &str, max: usize) -> String {
    let mut chars = value.chars();
    let taken: String = chars.by_ref().take(max).collect();
    if taken.len() >= max {
        taken
    } else {
        format!("{taken:<width$}", width = max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_split() {
        assert_eq!(
            sample_fields("12.5,OK,tank", "csv", "", ""),
            vec!["12.5", "OK", "tank"]
        );
    }
}
