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
    // Widths count characters, not bytes: "21.5°C" must not split inside "°" (that panicked).
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut at = 0;
    for w in widths {
        let end = (at + w).min(chars.len());
        out.push(chars[at..end].iter().collect::<String>().trim().to_string());
        at = end;
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

/// Exactly `max` characters: cut or space-padded (`format!` pads by chars).
fn truncate(value: &str, max: usize) -> String {
    let taken: String = value.chars().take(max).collect();
    format!("{taken:<width$}", width = max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsers_split_as_documented() {
        assert_eq!(sample_fields("a\tb\t", "tsv", "", ""), vec!["a", "b", ""]);
        assert_eq!(sample_fields("  1.5   ok  x ", "whitespace", "", ""), vec!["1.5", "ok", "x"]);
        assert_eq!(sample_fields("whole line\r\nnext", "whole_line", "", ""), vec!["whole line"]);
        assert_eq!(sample_fields("\"a,b\",c", "csv", "", ""), vec!["a,b", "c"], "quoted delimiter");
        assert!(sample_fields("", "csv", "", "").is_empty());
    }

    #[test]
    fn fixed_widths_count_characters_not_bytes() {
        // Regression: split_at on a byte index panicked inside a multi-byte character.
        assert_eq!(sample_fields("21.5°C  OK", "fixed", "6,2,4", ""), vec!["21.5°C", "", "OK"]);
        assert_eq!(sample_fields("µS/cm 12", "fixed", "5,3", ""), vec!["µS/cm", "12"]);
        // Short line: missing fields are empty, not a panic.
        assert_eq!(sample_fields("ab", "fixed", "1,1,5", ""), vec!["a", "b", ""]);
        // Bad / empty width list falls back to the whole line.
        assert_eq!(sample_fields("abc", "fixed", "x,0", ""), vec!["abc"]);
    }

    #[test]
    fn regex_groups_whole_match_and_misses() {
        assert_eq!(sample_fields("T=21.5 P=3", "regex", "", ""), vec!["T=21.5 P=3"], "empty pattern keeps the line");
        assert_eq!(sample_fields("T=21.5 P=3", "regex", "", r"T=(\S+) P=(\S+)"), vec!["21.5", "3"]);
        assert_eq!(sample_fields("T=21.5", "regex", "", r"T=\S+"), vec!["T=21.5"]);
        assert!(sample_fields("nothing", "regex", "", r"T=(\d+)").is_empty());
        assert_eq!(sample_fields("x", "regex", "", "(("), vec!["x"], "invalid pattern keeps the line");
    }

    #[test]
    fn join_and_trailers() {
        let v = |x: &[&str]| x.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(join_fields("csv", &v(&["1", "", "3"])), "1,,3");
        assert_eq!(join_fields("tsv", &v(&["1", "2"])), "1\t2");
        assert_eq!(join_fields("whitespace", &v(&["1", "", "3"])), "1 3");
        assert_eq!(join_fields("whole_line", &v(&["only", "x"])), "only");
        assert_eq!(join_fields("fixed", &v(&["12345678901", "°C"])), "12345678°C      ", "8 chars each");
        assert_eq!(trailer_bytes("crlf", ""), b"\r\n");
        assert_eq!(trailer_bytes("custom", "\x03"), b"\x03");
        assert!(trailer_bytes("none", "").is_empty());
        assert_eq!(trailer_bytes("anything", ""), b"\n");
    }

    #[test]
    fn csv_split() {
        assert_eq!(
            sample_fields("12.5,OK,tank", "csv", "", ""),
            vec!["12.5", "OK", "tank"]
        );
    }
}
