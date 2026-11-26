//! WITS level 0 encode / decode (pure, unit-tested).

/// One data line: item code (`0108`) and its raw value text.
#[derive(Debug, Clone, PartialEq)]
pub struct WitsItem {
    pub code: String,
    pub value: String,
}

/// Normalize an item code: digits only, left-padded to 4 (`108` → `0108`). `None` if it is not
/// a 4-digit record + item code.
pub fn normalize_code(code: &str) -> Option<String> {
    let t = code.trim();
    if t.is_empty() || t.len() > 4 || !t.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(format!("{:0>4}", t))
}

/// Parse one data line `RRIIvalue` (spaces between code and value allowed).
pub fn parse_line(line: &str) -> Option<WitsItem> {
    let t = line.trim();
    if t.len() < 5 || !t.as_bytes()[..4].iter().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let value = t[4..].trim();
    if value.is_empty() {
        return None;
    }
    Some(WitsItem { code: t[..4].to_string(), value: value.to_string() })
}

/// Incremental block parser: feed lines, get complete blocks.
#[derive(Debug, Default)]
pub struct Parser {
    in_block: bool,
    items: Vec<WitsItem>,
}

impl Parser {
    /// Feed one line. Returns a finished block on `!!`, or a one-item block for a data line
    /// that arrives outside `&& … !!`.
    pub fn feed(&mut self, line: &str) -> Option<Vec<WitsItem>> {
        let t = line.trim();
        if t.starts_with("&&") {
            self.in_block = true;
            self.items.clear();
            return None;
        }
        if t.starts_with("!!") {
            self.in_block = false;
            return (!self.items.is_empty()).then(|| std::mem::take(&mut self.items));
        }
        let item = parse_line(t)?;
        if self.in_block {
            self.items.push(item);
            // A sender that never closes its block must not grow memory without bound.
            if self.items.len() > 2000 {
                self.in_block = false;
                return Some(std::mem::take(&mut self.items));
            }
            None
        } else {
            Some(vec![item])
        }
    }
}

/// Encode a block (`&&`, one line per item, `!!`, CRLF line ends).
pub fn encode_block(items: &[(String, String)]) -> String {
    let mut out = String::from("&&\r\n");
    for (code, value) in items {
        out.push_str(code);
        out.push_str(value.trim());
        out.push_str("\r\n");
    }
    out.push_str("!!\r\n");
    out
}

/// Format a value for the wire: numbers with `decimals` places when given, text as-is.
pub fn format_value(raw: &str, decimals: Option<u32>) -> String {
    let t = raw.trim();
    match (decimals, t.parse::<f64>()) {
        (Some(d), Ok(x)) => format!("{x:.*}", d as usize),
        _ => match t.to_ascii_lowercase().as_str() {
            "true" => "1".into(),
            "false" => "0".into(),
            _ => t.to_string(),
        },
    }
}

/// Is `value` one of the configured null markers (numeric compare, so `-9999.00` matches `-9999`)?
pub fn is_null(value: &str, nulls: &[f64]) -> bool {
    value.trim().parse::<f64>().map(|x| nulls.iter().any(|n| (x - n).abs() < 1e-9)).unwrap_or(false)
}

/// Standard WITS record 1 ("general time-based") item names, for labels in the UI.
/// Frac vans and service companies usually agree their own item list; any 4-digit code works.
pub fn item_name(code: &str) -> &'static str {
    match code {
        "0101" => "Well identifier",
        "0102" => "Sidetrack / hole section",
        "0103" => "Record identifier",
        "0104" => "Sequence identifier",
        "0105" => "Date",
        "0106" => "Time",
        "0107" => "Activity code",
        "0108" => "Depth bit (meas)",
        "0109" => "Depth bit (vert)",
        "0110" => "Depth hole (meas)",
        "0111" => "Depth hole (vert)",
        "0112" => "Block position",
        "0113" => "Rate of penetration (avg)",
        "0114" => "Hookload (avg)",
        "0115" => "Hookload (max)",
        "0116" => "Weight on bit (avg)",
        "0117" => "Weight on bit (max)",
        "0118" => "Rotary torque (avg)",
        "0119" => "Rotary torque (max)",
        "0120" => "Rotary speed (avg)",
        "0121" => "Standpipe pressure (avg)",
        "0122" => "Casing (choke) pressure",
        "0123" => "Pump stroke rate 1",
        "0124" => "Pump stroke rate 2",
        "0125" => "Pump stroke rate 3",
        "0126" => "Tank volume (active)",
        "0127" => "Tank volume change",
        "0128" => "Mud flow out %",
        "0129" => "Mud flow out (avg)",
        "0130" => "Mud flow in (avg)",
        "0131" => "Mud density out (avg)",
        "0132" => "Mud density in (avg)",
        "0133" => "Mud temperature out",
        "0134" => "Mud temperature in",
        "0135" => "Mud conductivity out",
        "0136" => "Mud conductivity in",
        "0137" => "Pump stroke count (cum)",
        "0138" => "Lag strokes",
        "0139" => "Depth returns (meas)",
        "0140" => "Gas (avg)",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_and_codes() {
        assert_eq!(parse_line("01081234.5"), Some(WitsItem { code: "0108".into(), value: "1234.5".into() }));
        assert_eq!(parse_line("0121 3500"), Some(WitsItem { code: "0121".into(), value: "3500".into() }));
        assert_eq!(parse_line("0108"), None);
        assert_eq!(parse_line("ab121"), None);
        assert_eq!(normalize_code("108"), Some("0108".into()));
        assert_eq!(normalize_code("12345"), None);
        assert_eq!(normalize_code("1a"), None);
    }

    #[test]
    fn blocks_parse_and_bare_lines_pass_through() {
        let mut p = Parser::default();
        assert_eq!(p.feed("&&"), None);
        assert_eq!(p.feed("01081000"), None);
        assert_eq!(p.feed("garbage"), None);
        assert_eq!(p.feed("0113 25.5"), None);
        let b = p.feed("!!").unwrap();
        assert_eq!(b.len(), 2);
        assert_eq!(b[1].value, "25.5");
        assert_eq!(p.feed("!!"), None, "empty block is nothing");
        assert_eq!(p.feed("01217").unwrap()[0].code, "0121");
    }

    #[test]
    fn encode_round_trips() {
        let text = encode_block(&[("0108".into(), "100.5".into()), ("0121".into(), " 3400 ".into())]);
        assert_eq!(text, "&&\r\n0108100.5\r\n01213400\r\n!!\r\n");
        let mut p = Parser::default();
        let mut got = None;
        for l in text.lines() {
            if let Some(b) = p.feed(l) {
                got = Some(b);
            }
        }
        let got = got.unwrap();
        assert_eq!(got[0], WitsItem { code: "0108".into(), value: "100.5".into() });
        assert_eq!(got[1].value, "3400");
    }

    #[test]
    fn values_format_and_nulls() {
        assert_eq!(format_value("3.14159", Some(2)), "3.14");
        assert_eq!(format_value("true", None), "1");
        assert_eq!(format_value(" PUMPING ", Some(1)), "PUMPING");
        assert!(is_null("-9999.00", &[-9999.0, -999.25]));
        assert!(is_null("-999.25", &[-9999.0, -999.25]));
        assert!(!is_null("12", &[-9999.0]));
        assert!(!is_null("n/a", &[-9999.0]));
        assert_eq!(item_name("0108"), "Depth bit (meas)");
        assert_eq!(item_name("9901"), "");
    }
}
