//! CSV header handling: none, a fixed line, or **smart detection** of a preamble + header block.
//!
//! Instrument / test-rig CSVs often start with lines *about* the file (test name, operator,
//! serial number, …), then one or more header lines (column names, sometimes a units line), then
//! the data. Auto detection looks at the top of the file until the column count settles: the
//! data is the stable run of rows at the bottom of the sample, the column-name row is the first
//! row of that run (the header block), and anything above it is preamble.
//!
//! Line numbers here count **non-blank lines** — the tailer drops empty lines, so these indices
//! line up with the rows it delivers.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::parse::{split_csv_line, type_text, Cell};

/// How many lines Auto looks at before it stops waiting for more evidence.
pub const SCAN_LINES: usize = 200;
/// Data rows Auto wants to see after the header before it commits (unless the file stays idle).
pub const CONFIDENT_ROWS: usize = 2;
/// How many trailing rows decide the data's column count and column types.
const TAIL_SAMPLE: usize = 12;

/// Header handling for a CSV source.
///
/// JSON form (`header_row`): `"auto"`, `false` / `"none"`, or a 1-based line number for the
/// column-name row. Legacy `true` (the old checkbox) maps to `"auto"`, which reads a plain
/// one-line header exactly the same way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HeaderMode {
    /// Find the preamble / header block / data start from the file itself.
    #[default]
    Auto,
    /// No header: every line is data, columns are numbered from 0.
    None,
    /// Column names are on this 1-based line; data starts on the next one.
    Line(u32),
}

impl Serialize for HeaderMode {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Auto => s.serialize_str("auto"),
            Self::None => s.serialize_bool(false),
            Self::Line(n) => s.serialize_u32(*n),
        }
    }
}

impl<'de> Deserialize<'de> for HeaderMode {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            B(bool),
            N(u64),
            S(String),
        }
        Ok(match Raw::deserialize(d)? {
            Raw::B(true) => Self::Auto,
            Raw::B(false) => Self::None,
            Raw::N(0) => Self::None,
            Raw::N(n) => Self::Line(n.min(u32::MAX as u64) as u32),
            Raw::S(s) => match s.trim().to_ascii_lowercase().as_str() {
                "none" | "no" | "off" | "false" | "" => Self::None,
                "first" => Self::Line(1),
                "auto" | "true" => Self::Auto,
                other => match other.parse::<u32>() {
                    Ok(0) => Self::None,
                    Ok(n) => Self::Line(n),
                    Err(_) => return Err(serde::de::Error::custom(format!("header_row: unknown mode \"{s}\""))),
                },
            },
        })
    }
}

/// Where the header and data sit in a file. Indices are 0-based over non-blank lines.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HeaderLayout {
    /// Column names (`None` = no header; columns are numbered).
    pub names: Option<Vec<String>>,
    /// Line holding the column names.
    pub names_line: Option<usize>,
    /// Further header lines after the names (units, descriptions) — kept for display only.
    pub extra_header: Vec<Vec<String>>,
    /// Lines above the header block (file / test info), as raw text.
    pub preamble: Vec<String>,
    /// First data line. Lines before it are never published.
    pub data_start: usize,
    /// Columns in a data row.
    pub columns: usize,
    /// Data rows seen in the sample (evidence for Auto).
    pub data_rows: usize,
}

impl HeaderLayout {
    /// One-line description for the UI: `3 preamble lines · names on line 4 · data from line 6`.
    pub fn describe(&self) -> String {
        let mut bits = Vec::new();
        if !self.preamble.is_empty() {
            let n = self.preamble.len();
            bits.push(format!("{n} preamble line{}", if n == 1 { "" } else { "s" }));
        }
        match self.names_line {
            Some(l) => bits.push(format!("names on line {}", l + 1)),
            None => bits.push("no header".into()),
        }
        if !self.extra_header.is_empty() {
            let n = self.extra_header.len();
            bits.push(format!("+{n} header line{}", if n == 1 { "" } else { "s" }));
        }
        if self.columns > 0 {
            bits.push(format!("{} columns", self.columns));
        }
        if self.data_rows > 0 {
            bits.push(format!("data from line {}", self.data_start + 1));
        } else {
            bits.push("no data rows yet".into());
        }
        bits.join(" · ")
    }
}

fn is_blank(line: &str) -> bool {
    line.trim().is_empty()
}

fn is_numeric(cell: &str) -> bool {
    !cell.trim().is_empty() && !matches!(type_text(cell), Cell::Text(_))
}

/// A value rather than a label: a number, or text that starts like one (dates, times, `+5`).
fn is_value_like(cell: &str) -> bool {
    let t = cell.trim().trim_start_matches('"');
    is_numeric(t) || t.starts_with(|c: char| c.is_ascii_digit() || c == '+' || c == '-' || c == '.')
}

/// Could this row be a header line (names / units)? No numbers, and at most a quarter of its
/// cells value-like — `Date,2026-11-25,Start,08:00:00` is preamble even at the data's width.
fn header_like(r: &[String]) -> bool {
    if r.iter().any(|c| is_numeric(c)) {
        return false;
    }
    let filled: Vec<&String> = r.iter().filter(|c| !c.trim().is_empty()).collect();
    !filled.is_empty() && filled.iter().filter(|c| is_value_like(c)).count() * 4 <= filled.len()
}

fn split(line: &str, delim: char) -> Vec<String> {
    split_csv_line(line.strip_prefix('\u{feff}').unwrap_or(line), delim)
}

/// Lay out `lines` (the top of the file, non-blank lines in order) for `mode`.
pub fn layout(lines: &[String], delim: char, mode: HeaderMode) -> HeaderLayout {
    match mode {
        HeaderMode::None => {
            let columns = lines.iter().find(|l| !is_blank(l)).map(|l| split(l, delim).len()).unwrap_or(0);
            HeaderLayout { columns, data_rows: lines.iter().filter(|l| !is_blank(l)).count(), ..Default::default() }
        }
        HeaderMode::Line(n) => {
            let at = (n.max(1) - 1) as usize;
            let names = lines.get(at).map(|l| split(l, delim));
            HeaderLayout {
                columns: names.as_ref().map(|n| n.len()).unwrap_or(0),
                names_line: names.as_ref().map(|_| at),
                names,
                preamble: lines.iter().take(at).cloned().collect(),
                data_start: at + 1,
                data_rows: lines.len().saturating_sub(at + 1),
                ..Default::default()
            }
        }
        HeaderMode::Auto => detect(lines, delim),
    }
}

/// Smart detection. See the module docs for the idea; the steps:
///
/// 1. The data's column count is the most common count among the last rows of the sample.
/// 2. The *stable run* is the block of rows at the bottom with exactly that count.
/// 3. Columns that are mostly numeric in the trailing rows define what a data row looks like.
/// 4. In the stable run, rows before the first data-looking row are the header block; its last
///    stretch of label-only rows holds the names (rows above it with numbers, dates or
///    times in them are `key,value` preamble that happened to have the same width).
/// 5. If the run starts straight with data, a text-only row just above it whose width is within
///    one of the data's (header with a trailing / missing delimiter) is taken as the names.
pub fn detect(lines: &[String], delim: char) -> HeaderLayout {
    let rows: Vec<(usize, Vec<String>)> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| !is_blank(l))
        .map(|(i, l)| (i, split(l, delim)))
        .collect();
    if rows.is_empty() {
        return HeaderLayout::default();
    }

    // 1. Column count of the data.
    let tail = &rows[rows.len().saturating_sub(TAIL_SAMPLE)..];
    let mut counts: Vec<(usize, usize)> = Vec::new();
    for (_, r) in tail {
        match counts.iter_mut().find(|(n, _)| *n == r.len()) {
            Some(c) => c.1 += 1,
            None => counts.push((r.len(), 1)),
        }
    }
    // Most frequent; ties go to the wider row (data is rarely narrower than its preamble), then
    // to the count of the very last row.
    let last_n = rows.last().map(|(_, r)| r.len()).unwrap_or(0);
    let n = counts
        .iter()
        .max_by_key(|(n, c)| (*c, *n == last_n, *n))
        .map(|(n, _)| *n)
        .unwrap_or(last_n);

    // 2. Stable run at the bottom.
    let mut s = rows.len();
    while s > 0 && rows[s - 1].1.len() == n {
        s -= 1;
    }
    if s == rows.len() {
        // The last row is an outlier (e.g. a short trailing line): run ends one earlier.
        let end = rows.iter().rposition(|(_, r)| r.len() == n).unwrap_or(0);
        s = end;
        while s > 0 && rows[s - 1].1.len() == n {
            s -= 1;
        }
    }
    let run_end = rows.iter().rposition(|(_, r)| r.len() == n).map(|i| i + 1).unwrap_or(rows.len());
    let run = &rows[s..run_end];

    // 3. Numeric columns, judged on the bottom of the run.
    // Only the bottom half of the run: a header block (or same-width preamble) sits at its top.
    let take = run.len().div_ceil(2).clamp(1, TAIL_SAMPLE);
    let sample = &run[run.len() - take.min(run.len())..];
    let numeric_cols: Vec<usize> = (0..n)
        .filter(|&j| {
            let hits = sample.iter().filter(|(_, r)| r.get(j).is_some_and(|c| is_numeric(c))).count();
            hits * 2 > sample.len()
        })
        .collect();
    // A data row has a number (or nothing) in every numeric column — text there means header.
    let looks_like_data = |r: &[String]| -> bool {
        if numeric_cols.is_empty() {
            return false;
        }
        let mut hits = 0;
        for &j in &numeric_cols {
            match r.get(j).map(|c| c.trim()) {
                Some("") => {}
                Some(c) if is_numeric(c) => hits += 1,
                _ => return false,
            }
        }
        hits > 0
    };
    let no_numbers = |r: &[String]| header_like(r);

    // 4. Header block inside the run.
    let first_data = if numeric_cols.is_empty() {
        // All-text data: the first row of the run names the columns (the usual CSV), unless the
        // run is a single row with nothing above it… which is still most likely a header.
        1.min(run.len())
    } else {
        run.iter().position(|(_, r)| looks_like_data(r)).unwrap_or(run.len())
    };
    let mut names_at: Option<usize> = None; // index into `run`
    if first_data > 0 {
        // Last all-text stretch before the data.
        let mut h = first_data;
        while h > 0 && no_numbers(&run[h - 1].1) {
            h -= 1;
        }
        names_at = Some(if h < first_data { h } else { first_data - 1 });
    }

    let mut out = HeaderLayout { columns: n, ..Default::default() };
    let data_rows = run.len() - first_data;
    out.data_rows = data_rows;
    out.data_start = run.get(first_data).map(|(i, _)| *i).unwrap_or_else(|| run.last().map(|(i, _)| i + 1).unwrap_or(0));

    let names_row_idx: Option<usize> = match names_at {
        Some(k) => Some(s + k),
        None if s > 0 => {
            // 5. Header just above the run with a slightly different width.
            let (_, prev) = &rows[s - 1];
            (no_numbers(prev) && prev.len().abs_diff(n) <= 1 && prev.iter().filter(|c| !c.trim().is_empty()).count() >= n.min(2))
                .then_some(s - 1)
        }
        None => None,
    };

    match names_row_idx {
        Some(ri) => {
            let (line, names) = &rows[ri];
            out.names_line = Some(*line);
            let mut names: Vec<String> = names.iter().map(|c| c.trim().to_string()).collect();
            // Header lines between the names and the data (units etc.).
            let extra: Vec<Vec<String>> = rows[ri + 1..]
                .iter()
                .take_while(|(i, _)| *i < out.data_start)
                .map(|(_, r)| r.clone())
                .collect();
            // A blank name borrows from the next header line (two-row headers that split a label).
            for (j, nm) in names.iter_mut().enumerate() {
                if nm.is_empty() {
                    if let Some(v) = extra.iter().filter_map(|r| r.get(j)).map(|c| c.trim()).find(|c| !c.is_empty()) {
                        *nm = v.to_string();
                    }
                }
            }
            out.extra_header = extra;
            out.names = Some(names);
            out.preamble = rows[..ri].iter().map(|(i, _)| lines[*i].clone()).collect();
        }
        None => {
            out.preamble = rows.iter().take_while(|(i, _)| *i < out.data_start).map(|(i, _)| lines[*i].clone()).collect();
        }
    }
    out
}

/// Has Auto seen enough to commit? `idle` = the file did not grow on the last poll.
pub fn auto_settled(l: &HeaderLayout, scanned: usize, idle: bool) -> bool {
    l.data_rows >= CONFIDENT_ROWS || scanned >= SCAN_LINES || (idle && l.data_rows >= 1)
}

/// First `max` complete non-blank lines of the file (`\r` stripped), as the tailer delivers them.
pub fn head_lines(path: &Path, max: usize) -> Vec<String> {
    let Ok(mut f) = File::open(path) else { return Vec::new() };
    let mut buf = Vec::new();
    if f.by_ref().take(1024 * 1024).read_to_end(&mut buf).is_err() {
        return Vec::new();
    }
    let Some(end) = buf.iter().rposition(|b| *b == b'\n') else { return Vec::new() };
    buf[..end]
        .split(|b| *b == b'\n')
        .map(|raw| raw.strip_suffix(b"\r").unwrap_or(raw))
        .filter(|raw| !raw.is_empty())
        .take(max)
        .map(|raw| String::from_utf8_lossy(raw).into_owned())
        .collect()
}

/// How many non-blank lines precede byte `offset` (only for offsets within the first 4 MiB;
/// beyond that the header is long gone and the answer is not needed).
pub fn lines_before(path: &Path, offset: u64) -> Option<usize> {
    if offset > 4 * 1024 * 1024 {
        return None;
    }
    let mut f = File::open(path).ok()?;
    let mut buf = vec![0u8; offset as usize];
    f.read_exact(&mut buf).ok()?;
    let mut n = 0;
    for raw in buf.split(|b| *b == b'\n') {
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        if !raw.is_empty() {
            n += 1;
        }
    }
    // `buf` ends on a line boundary, so the final split piece is empty and was not counted.
    Some(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ls(s: &str) -> Vec<String> {
        s.lines().filter(|l| !l.is_empty()).map(|l| l.to_string()).collect()
    }

    #[test]
    fn plain_header() {
        let l = detect(&ls("time,temp,flow\n10:00,20.5,3\n10:01,20.6,4\n"), ',');
        assert_eq!(l.names.unwrap(), vec!["time", "temp", "flow"]);
        assert_eq!((l.names_line, l.data_start, l.data_rows, l.columns), (Some(0), 1, 2, 3));
        assert!(l.preamble.is_empty());
    }

    #[test]
    fn no_header_all_numeric() {
        let l = detect(&ls("1,2,3\n4,5,6\n7,8,9\n"), ',');
        assert_eq!((l.names.clone(), l.data_start, l.data_rows), (None, 0, 3));
        assert!(l.describe().contains("no header"), "{}", l.describe());
    }

    #[test]
    fn preamble_then_header_then_data() {
        let f = "Test Report\nOperator: Mike\nSerial,SN-1234\nDate,2026-11-25,Start,08:00:00\n\
                 Time,Pressure,Temp,Flow,Valve\n0.0,101.3,20.1,0,OPEN\n0.5,101.4,20.2,1.5,OPEN\n1.0,101.6,20.2,1.6,CLOSED\n";
        let l = detect(&ls(f), ',');
        assert_eq!(l.names.as_deref().unwrap(), ["Time", "Pressure", "Temp", "Flow", "Valve"]);
        assert_eq!(l.names_line, Some(4));
        assert_eq!(l.data_start, 5);
        assert_eq!(l.preamble.len(), 4);
        assert_eq!(l.preamble[0], "Test Report");
        assert_eq!(l.columns, 5);
        assert_eq!(l.describe(), "4 preamble lines · names on line 5 · 5 columns · data from line 6");
    }

    #[test]
    fn same_width_date_line_and_blank_line_above_header() {
        let f = "Pump Test Report,Rig 4\nOperator: Sam\nSerial,SN-77\nDate,2026-11-25,Start,08:00:00\n\n\
                 Time,Pressure,Temp,Flow\ns,kPa,degC,L/min\n0,101.3,20.1,0\n1,101.4,20.3,1.5\n2,101.6,20.4,1.6\n";
        let l = detect(&ls(f), ',');
        assert_eq!(l.names.as_deref().unwrap(), ["Time", "Pressure", "Temp", "Flow"]);
        assert_eq!(l.preamble.len(), 4);
        assert_eq!(l.describe(), "4 preamble lines · names on line 5 · +1 header line · 4 columns · data from line 7");
    }

    #[test]
    fn units_line_under_names() {
        let f = "Rig 7 run 12\nTime,Pressure,Temp\ns,kPa,degC\n0,101.3,20.1\n1,101.4,20.3\n2,101.4,20.4\n";
        let l = detect(&ls(f), ',');
        assert_eq!(l.names.as_deref().unwrap(), ["Time", "Pressure", "Temp"]);
        assert_eq!(l.extra_header, vec![vec!["s".to_string(), "kPa".into(), "degC".into()]]);
        assert_eq!((l.names_line, l.data_start), (Some(1), 3));
        assert_eq!(l.preamble, vec!["Rig 7 run 12"]);
    }

    #[test]
    fn same_width_key_value_preamble_with_numbers_is_not_the_header() {
        let f = "Station,12\nGain,1.5\ntime,level\n1,0.3\n2,0.4\n3,0.5\n";
        let l = detect(&ls(f), ',');
        assert_eq!(l.names.as_deref().unwrap(), ["time", "level"]);
        assert_eq!(l.preamble, vec!["Station,12", "Gain,1.5"]);
        assert_eq!(l.data_start, 3);
    }

    #[test]
    fn header_with_trailing_delimiter_difference() {
        let f = "a,b,c,\n1,2,3\n4,5,6\n";
        let l = detect(&ls(f), ',');
        assert_eq!(l.names.as_deref().unwrap(), ["a", "b", "c", ""]);
        assert_eq!(l.data_start, 1);
    }

    #[test]
    fn all_text_data_takes_first_row_as_names() {
        let l = detect(&ls("state,mode\nRUN,AUTO\nSTOP,MAN\n"), ',');
        assert_eq!(l.names.as_deref().unwrap(), ["state", "mode"]);
        assert_eq!(l.data_start, 1);
    }

    #[test]
    fn header_only_and_preamble_only_are_unsettled() {
        let l = detect(&ls("Info line\ntime,temp\n"), ',');
        assert_eq!(l.names.as_deref().unwrap(), ["time", "temp"]);
        assert_eq!(l.data_rows, 0);
        assert!(!auto_settled(&l, 2, true));
        let one = detect(&ls("Info line\ntime,temp\n1,2\n"), ',');
        assert_eq!(one.names.as_deref().unwrap(), ["time", "temp"]);
        assert!(!auto_settled(&one, 3, false));
        assert!(auto_settled(&one, 3, true));
    }

    #[test]
    fn bom_tabs_and_blank_lines() {
        let lines = vec!["\u{feff}Run info".to_string(), "  ".into(), "t\tv".into(), "1\t2".into(), "2\t3".into()];
        let l = detect(&lines, '\t');
        assert_eq!(l.names.as_deref().unwrap(), ["t", "v"]);
        assert_eq!((l.names_line, l.data_start), (Some(2), 3));
    }

    #[test]
    fn fixed_line_and_none_modes() {
        let lines = ls("junk\nA,B\n1,2\n");
        let l = layout(&lines, ',', HeaderMode::Line(2));
        assert_eq!((l.names.as_deref().map(|v| v.to_vec()), l.data_start), (Some(vec!["A".to_string(), "B".into()]), 2));
        assert_eq!(l.preamble, vec!["junk"]);
        let n = layout(&lines, ',', HeaderMode::None);
        assert_eq!((n.names, n.data_start), (None, 0));
    }

    #[test]
    fn mode_json_forms() {
        let p = |s: &str| serde_json::from_str::<HeaderMode>(s).unwrap();
        assert_eq!(p("true"), HeaderMode::Auto);
        assert_eq!(p("false"), HeaderMode::None);
        assert_eq!(p("\"auto\""), HeaderMode::Auto);
        assert_eq!(p("\"none\""), HeaderMode::None);
        assert_eq!(p("3"), HeaderMode::Line(3));
        assert_eq!(p("\"first\""), HeaderMode::Line(1));
        assert_eq!(p("0"), HeaderMode::None);
        assert!(serde_json::from_str::<HeaderMode>("\"bogus\"").is_err());
        for m in [HeaderMode::Auto, HeaderMode::None, HeaderMode::Line(4)] {
            assert_eq!(p(&serde_json::to_string(&m).unwrap()), m);
        }
    }
}
