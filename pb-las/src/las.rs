//! LAS (Log ASCII Standard) 1.2 / 2.0 parsing and writing. Pure functions.
//!
//! A LAS file is `~V` version, `~W` well, `~C` curves, `~P` parameters, `~O` other and `~A` ASCII
//! data sections. Header lines look like `MNEM.UNIT   value : description`; `#` starts a comment.
//! Only unwrapped data (`WRAP. NO`, one row per line) is supported.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct Curve {
    pub mnemonic: String,
    pub unit: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    pub version: String,
    /// `WRAP. YES` — multi-line rows, not supported.
    pub wrapped: bool,
    /// The file's NULL value (`-999.25` when absent).
    pub null: f64,
    pub curves: Vec<Curve>,
    /// `~W` entries by mnemonic (STRT, STOP, STEP, WELL …).
    pub well: BTreeMap<String, String>,
}

impl Default for Header {
    fn default() -> Self {
        Self { version: "2.0".into(), wrapped: false, null: -999.25, curves: Vec::new(), well: BTreeMap::new() }
    }
}

impl Header {
    /// Column of a curve by mnemonic (case-insensitive).
    pub fn column(&self, mnemonic: &str) -> Option<usize> {
        let m = mnemonic.trim();
        self.curves.iter().position(|c| c.mnemonic.eq_ignore_ascii_case(m))
    }

    /// True when `v` equals the file's NULL value (within float tolerance).
    pub fn is_null(&self, v: f64) -> bool {
        (v - self.null).abs() < 1e-9 || v.is_nan()
    }
}

/// One `MNEM.UNIT value : description` line.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub mnemonic: String,
    pub unit: String,
    pub value: String,
    pub description: String,
}

/// Split a header line `MNEM.UNIT  value : description`; `None` for comments and blank lines.
pub fn parse_field(line: &str) -> Option<Field> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') || line.starts_with('~') {
        return None;
    }
    let dot = line.find('.')?;
    let mnemonic = line[..dot].trim().to_string();
    if mnemonic.is_empty() {
        return None;
    }
    let rest = &line[dot + 1..];
    // Unit: characters right after the dot up to the first whitespace.
    let unit_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let unit = rest[..unit_end].to_string();
    let rest = &rest[unit_end..];
    // The description follows the LAST colon (values such as times may contain colons).
    let (value, description) = match rest.rfind(':') {
        Some(i) => (rest[..i].trim(), rest[i + 1..].trim()),
        None => (rest.trim(), ""),
    };
    Some(Field { mnemonic, unit, value: value.to_string(), description: description.to_string() })
}

/// Incremental header parser: feed lines until [`HeaderParser::in_data`] turns true.
#[derive(Debug, Default)]
pub struct HeaderParser {
    section: Option<char>,
    header: Header,
    data: bool,
}

impl HeaderParser {
    /// Parser positioned at the start of a file.
    pub fn new() -> Self {
        Self::default()
    }

    /// True once the `~A` (data) section has started.
    pub fn in_data(&self) -> bool {
        self.data
    }

    /// Header parsed so far.
    pub fn header(&self) -> &Header {
        &self.header
    }

    /// Consume one header line. Returns an error for unsupported files (wrapped data).
    pub fn feed(&mut self, line: &str) -> Result<(), String> {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            return Ok(());
        }
        if let Some(rest) = t.strip_prefix('~') {
            let sec = rest.chars().next().map(|c| c.to_ascii_uppercase());
            self.section = sec;
            if sec == Some('A') {
                self.data = true;
                if self.header.wrapped {
                    return Err("WRAP YES (multi-line rows) is not supported".into());
                }
                if self.header.curves.is_empty() {
                    return Err("no ~Curve section before ~A".into());
                }
            }
            return Ok(());
        }
        let Some(f) = parse_field(t) else { return Ok(()) };
        match self.section {
            Some('V') => {
                if f.mnemonic.eq_ignore_ascii_case("VERS") {
                    self.header.version = f.value.clone();
                } else if f.mnemonic.eq_ignore_ascii_case("WRAP") {
                    self.header.wrapped = f.value.trim().eq_ignore_ascii_case("YES");
                }
            }
            Some('W') => {
                if f.mnemonic.eq_ignore_ascii_case("NULL") {
                    if let Ok(v) = f.value.trim().parse::<f64>() {
                        self.header.null = v;
                    }
                }
                self.header.well.insert(f.mnemonic.to_ascii_uppercase(), f.value);
            }
            Some('C') => self.header.curves.push(Curve { mnemonic: f.mnemonic, unit: f.unit, description: f.description }),
            _ => {}
        }
        Ok(())
    }
}

/// Parse a data row of whitespace-separated numbers. `None` marks the NULL value.
pub fn parse_row(line: &str, header: &Header) -> Result<Vec<Option<f64>>, String> {
    let cells: Vec<&str> = line.split_whitespace().collect();
    if cells.len() != header.curves.len() {
        return Err(format!("row has {} values, expected {} curves", cells.len(), header.curves.len()));
    }
    cells
        .iter()
        .map(|c| match c.parse::<f64>() {
            Ok(v) if header.is_null(v) => Ok(None),
            Ok(v) => Ok(Some(v)),
            Err(_) => Err(format!("\"{c}\" is not a number")),
        })
        .collect()
}

/// Read just the header of an existing file (up to and including `~A`).
pub fn read_header(path: &Path) -> Result<Header, String> {
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut p = HeaderParser::new();
    for line in BufReader::new(f).lines().take(20_000) {
        let line = line.map_err(|e| e.to_string())?;
        p.feed(&line)?;
        if p.in_data() {
            return Ok(p.header().clone());
        }
    }
    Err("no ~A section found".into())
}

pub const DEFAULT_NULL: f64 = -999.25;

/// A fresh LAS 2.0 header for a growing log.
pub fn render_header(well: &str, index: &Curve, curves: &[Curve], start: f64) -> String {
    let mut s = String::new();
    s.push_str("~Version Information\n VERS.   2.0 : CWLS LOG ASCII STANDARD - VERSION 2.0\n WRAP.   NO  : ONE LINE PER DEPTH STEP\n");
    s.push_str("~Well Information\n");
    s.push_str(&format!(" STRT.{:<6} {:<14}: START\n", index.unit, start));
    s.push_str(&format!(" STOP.{:<6} {:<14}: STOP (not updated while logging)\n", index.unit, start));
    s.push_str(&format!(" STEP.{:<6} {:<14}: STEP\n", index.unit, 0));
    s.push_str(&format!(" NULL.       {:<14}: NULL VALUE\n", DEFAULT_NULL));
    s.push_str(&format!(" WELL.       {:<14}: WELL\n", well));
    s.push_str("~Curve Information\n");
    for (i, c) in std::iter::once(index).chain(curves.iter()).enumerate() {
        let desc = if c.description.is_empty() { format!("{} {}", i + 1, c.mnemonic) } else { c.description.clone() };
        s.push_str(&format!(" {:<6}.{:<6} : {desc}\n", c.mnemonic, c.unit));
    }
    s.push_str("~Parameter Information\n~ASCII Log Data\n");
    s
}

/// One data row for the writer: index, then each value; `None` is written as the default NULL (-999.25).
pub fn render_row(index: f64, values: &[Option<f64>]) -> String {
    let mut s = fmt_num(index);
    for v in values {
        s.push(' ');
        s.push_str(&fmt_num(v.unwrap_or(DEFAULT_NULL)));
    }
    s.push('\n');
    s
}

fn fmt_num(v: f64) -> String {
    let t = format!("{v:.4}");
    let t = t.trim_end_matches('0').trim_end_matches('.');
    if t.is_empty() || t == "-" || t == "-0" { "0".into() } else { t.to_string() }
}

#[cfg(test)]
pub(crate) const SAMPLE: &str = "\
# a comment
~Version Information
 VERS.   2.0 : CWLS LOG ASCII STANDARD - VERSION 2.0
 WRAP.   NO  : ONE LINE PER DEPTH STEP
~Well Information
 STRT.M   1670.0000 : START DEPTH
 STOP.M   1669.7500 : STOP DEPTH
 STEP.M   -0.1250   : STEP
 NULL.    -999.25   : NULL VALUE
 WELL.    ANY ++ 12-34-56 : WELL
 DATE.    2026:11:20 10:30:00 : LOG DATE
~Curve Information
 DEPT.M                 : 1  DEPTH
 DT  .US/M  60 520 32 00 : 2  SONIC TRANSIT TIME
 GR  .GAPI              : 3  GAMMA RAY
~Parameter Information
 MUD .   GEL  : MUD TYPE
~ASCII Log Data
1670.000   123.450   110.200
1669.875   123.450   -999.25
";

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_all(text: &str) -> (Header, Vec<String>) {
        let mut p = HeaderParser::new();
        let mut data = Vec::new();
        for l in text.lines() {
            if p.in_data() {
                data.push(l.to_string());
            } else {
                p.feed(l).unwrap();
            }
        }
        (p.header().clone(), data)
    }

    #[test]
    fn field_lines_split_name_unit_value_description() {
        let f = parse_field(" DT  .US/M  60 520 32 00 : 2  SONIC").unwrap();
        assert_eq!((f.mnemonic.as_str(), f.unit.as_str(), f.value.as_str(), f.description.as_str()), ("DT", "US/M", "60 520 32 00", "2  SONIC"));
        let f = parse_field(" DATE.    2026:11:20 10:30:00 : LOG DATE").unwrap();
        assert_eq!(f.value, "2026:11:20 10:30:00", "the description starts after the LAST colon");
        assert_eq!(f.description, "LOG DATE");
        let f = parse_field(" NULL.   -999.25").unwrap();
        assert_eq!((f.value.as_str(), f.description.as_str()), ("-999.25", ""));
        assert!(parse_field("# comment").is_none());
        assert!(parse_field("~Curve").is_none());
        assert!(parse_field("no dot here").is_none());
    }

    #[test]
    fn header_sections_curves_null_and_well() {
        let (h, data) = parse_all(SAMPLE);
        assert_eq!(h.version, "2.0");
        assert!(!h.wrapped);
        assert_eq!(h.null, -999.25);
        assert_eq!(h.curves.iter().map(|c| c.mnemonic.as_str()).collect::<Vec<_>>(), vec!["DEPT", "DT", "GR"]);
        assert_eq!(h.curves[1].unit, "US/M");
        assert_eq!(h.well["WELL"], "ANY ++ 12-34-56");
        assert_eq!(h.column("gr"), Some(2));
        assert_eq!(h.column("nope"), None);
        assert_eq!(data.len(), 2);
    }

    #[test]
    fn rows_parse_with_nulls_and_errors() {
        let (h, data) = parse_all(SAMPLE);
        assert_eq!(parse_row(&data[0], &h).unwrap(), vec![Some(1670.0), Some(123.45), Some(110.2)]);
        assert_eq!(parse_row(&data[1], &h).unwrap(), vec![Some(1669.875), Some(123.45), None]);
        assert!(parse_row("1 2", &h).unwrap_err().contains("expected 3"));
        assert!(parse_row("1 x 3", &h).unwrap_err().contains("not a number"));
    }

    #[test]
    fn unsupported_and_incomplete_files_are_rejected() {
        let mut p = HeaderParser::new();
        for l in ["~V", " WRAP. YES : multi-line", "~C", " DEPT.M : d"] {
            p.feed(l).unwrap();
        }
        assert!(p.feed("~A").unwrap_err().contains("WRAP YES"));
        let mut p = HeaderParser::new();
        assert!(p.feed("~A").unwrap_err().contains("no ~Curve"));
        // v1.2 style headers and lowercase section letters work.
        let (h, _) = parse_all("~v\n VERS. 1.2 : x\n~c\n DEPT.FT : d\n GR.API : g\n~a\n1 2\n");
        assert_eq!((h.version.as_str(), h.curves.len()), ("1.2", 2));
    }

    #[test]
    fn written_files_read_back() {
        let idx = Curve { mnemonic: "DEPT".into(), unit: "M".into(), description: String::new() };
        let curves = vec![
            Curve { mnemonic: "GR".into(), unit: "GAPI".into(), description: String::new() },
            Curve { mnemonic: "ROP".into(), unit: "M/H".into(), description: "rate".into() },
        ];
        let mut text = render_header("Well-A", &idx, &curves, 100.0);
        text.push_str(&render_row(100.0, &[Some(55.5), None]));
        text.push_str(&render_row(100.5, &[Some(56.0), Some(12.25)]));
        let (h, data) = parse_all(&text);
        assert_eq!(h.curves.iter().map(|c| c.mnemonic.as_str()).collect::<Vec<_>>(), vec!["DEPT", "GR", "ROP"]);
        assert_eq!(h.well["WELL"], "Well-A");
        assert_eq!(h.null, DEFAULT_NULL);
        assert_eq!(parse_row(&data[0], &h).unwrap(), vec![Some(100.0), Some(55.5), None]);
        assert_eq!(parse_row(&data[1], &h).unwrap(), vec![Some(100.5), Some(56.0), Some(12.25)]);
        assert_eq!(fmt_num(1.5), "1.5");
        assert_eq!(fmt_num(2.0), "2");
        assert_eq!(fmt_num(-0.00001), "0");
    }

    #[test]
    fn read_header_from_disk_stops_at_ascii_section() {
        let dir = pb_tail::testutil::dir("lasheader");
        let p = dir.join("w.las");
        std::fs::write(&p, SAMPLE).unwrap();
        let h = read_header(&p).unwrap();
        assert_eq!(h.curves.len(), 3);
        std::fs::write(&p, "~V\n VERS. 2.0 : x\n").unwrap();
        assert!(read_header(&p).is_err());
    }
}
