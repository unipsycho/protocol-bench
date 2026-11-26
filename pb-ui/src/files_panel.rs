//! File follow panel — growing CSV / JSON-lines sources (first-build parsers per the PRD).
//!
//! The panel owns **sources** (path, parser, poll, start position). Hub `Files` Input nodes own
//! column / JSON-path → `file_N` binds. Byte offset across restart and rotation is owned by the
//! pack (read-only here once live).

use egui::{Layout, RichText, Sense, UiBuilder};
use pb_engine::{EngineId, HostSnapshot};
use serde::{Deserialize, Serialize};

use crate::connection_list::{self, RowAction};
use crate::modbus_panel::{combo_row, drag_row, section, text_field};
use crate::theme::{apply_form_spacing, font_ui, form_inset, space, ThemePalette};

// ---------------------------------------------------------------------------
// Live state (from the pack's `detail_json`)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LiveSrc {
    pub id: String,
    pub state: String,
    pub connected: bool,
    pub error: String,
    pub rows: u64,
    pub bad_rows: u64,
    pub offset: u64,
    /// `(name, sample)` from the pack, for the column picker.
    pub columns: Vec<(String, String)>,
    /// CSV header layout the pack found (`3 preamble lines · names on line 4 · …`).
    pub header: String,
    /// Preamble lines above the header.
    pub preamble: Vec<String>,
}

pub fn parse_live(detail_json: &str) -> Vec<LiveSrc> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(detail_json) else {
        return Vec::new();
    };
    let st = |c: &serde_json::Value, k: &str| c.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let n = |c: &serde_json::Value, k: &str| c.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
    v.get("sources")
        .and_then(|c| c.as_array())
        .into_iter()
        .flatten()
        .map(|c| LiveSrc {
            id: st(c, "id"),
            state: st(c, "state"),
            connected: c.get("connected").and_then(|x| x.as_bool()).unwrap_or(false),
            error: st(c, "error"),
            rows: n(c, "rows"),
            bad_rows: n(c, "bad_rows"),
            offset: n(c, "offset"),
            columns: c
                .get("columns")
                .and_then(|x| x.as_array())
                .map(|a| a.iter().map(|x| (st(x, "name"), st(x, "sample"))).collect())
                .unwrap_or_default(),
            header: st(c, "header"),
            preamble: c
                .get("preamble")
                .and_then(|x| x.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                .unwrap_or_default(),
        })
        .collect()
}

/// Live sources from the host snapshot, plus whether the pack is loaded at all.
pub fn live_from_host(host: &HostSnapshot) -> (Vec<LiveSrc>, bool) {
    match host.slot(EngineId::FileFollow) {
        Some(slot) => (parse_live(&slot.detail_json), slot.pack_loaded),
        None => (Vec::new(), false),
    }
}

/// Coloured dot + state, and the last error underneath.
pub fn status_line(ui: &mut egui::Ui, live: Option<&LiveSrc>, pack_loaded: bool, p: &ThemePalette) {
    let (color, text) = if !pack_loaded {
        (p.warn, "File follow pack not loaded — copy libpb_files into the service modules/ folder".to_string())
    } else {
        match live {
            None => (p.muted, "Idle — no hub Files Input binds a column from this source".to_string()),
            Some(l) if l.connected => (p.live, l.state.clone()),
            Some(l) if l.state == "error" => (p.err, "Error".to_string()),
            Some(l) => (p.warn, l.state.clone()),
        }
    };
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = space::ITEM_X;
        ui.label(RichText::new("●").font(font_ui(12.0)).color(color));
        ui.label(RichText::new(text).font(font_ui(12.0)).color(p.text));
    });
    if let Some(l) = live {
        if !l.error.is_empty() {
            ui.label(RichText::new(&l.error).font(font_ui(11.5)).color(p.warn));
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileParser {
    Csv,
    /// One JSON object per line.
    Jsonl,
}

impl FileParser {
    pub fn label(self) -> &'static str {
        match self {
            Self::Csv => "CSV",
            Self::Jsonl => "JSON lines",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileStart {
    /// Read existing rows once, then follow.
    Beginning,
    /// Skip existing rows; only new appends become points.
    End,
}

impl FileStart {
    pub fn label(self) -> &'static str {
        match self {
            Self::Beginning => "Beginning of file",
            Self::End => "End of file (new rows only)",
        }
    }
}

/// CSV header handling. Same JSON as the pack's `header_row`: `"auto"`, `false`, or a 1-based
/// line number for the column names. Legacy `true` (old checkbox) reads as Auto.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HeaderMode {
    /// Smart detection: skip file / test info lines, find the names row and where data starts.
    #[default]
    Auto,
    None,
    /// Column names on this 1-based line; data from the next.
    Line(u32),
}

impl HeaderMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto-detect",
            Self::None => "None (numbered columns)",
            Self::Line(_) => "Names on line…",
        }
    }
}

impl Serialize for HeaderMode {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Auto => s.serialize_str("auto"),
            Self::None => s.serialize_bool(false),
            Self::Line(n) => s.serialize_u32(*n),
        }
    }
}

impl<'de> Deserialize<'de> for HeaderMode {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            B(bool),
            N(u64),
            S(String),
        }
        Ok(match Raw::deserialize(d)? {
            Raw::B(true) => Self::Auto,
            Raw::B(false) | Raw::N(0) => Self::None,
            Raw::N(n) => Self::Line(n.min(u32::MAX as u64) as u32),
            Raw::S(s) => match s.trim().to_ascii_lowercase().as_str() {
                "none" | "false" | "" => Self::None,
                "first" => Self::Line(1),
                other => match other.parse::<u32>() {
                    Ok(0) => Self::None,
                    Ok(n) => Self::Line(n),
                    Err(_) => Self::Auto,
                },
            },
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileSource {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub path: String,
    pub parser: FileParser,
    /// CSV: header handling (smart detection by default).
    #[serde(default)]
    pub header_row: HeaderMode,
    /// CSV field delimiter (single character).
    #[serde(default = "default_delim")]
    pub delimiter: String,
    #[serde(default = "default_poll_ms")]
    pub poll_ms: u32,
    #[serde(default = "default_start")]
    pub start_at: FileStart,
    /// Reopen from byte 0 when the file shrinks or is replaced (log rotation).
    #[serde(default = "yes")]
    pub follow_rotation: bool,
}

fn yes() -> bool {
    true
}
fn default_delim() -> String {
    ",".into()
}
fn default_poll_ms() -> u32 {
    500
}
fn default_start() -> FileStart {
    FileStart::End
}

impl FileSource {
    pub fn default_csv() -> Self {
        Self {
            id: "meter".into(),
            label: "Meter log".into(),
            path: String::new(),
            parser: FileParser::Csv,
            header_row: HeaderMode::Auto,
            delimiter: default_delim(),
            poll_ms: default_poll_ms(),
            start_at: default_start(),
            follow_rotation: true,
        }
    }

    pub fn display_name(&self) -> &str {
        if self.label.trim().is_empty() {
            &self.id
        } else {
            &self.label
        }
    }

    pub fn summary(&self) -> String {
        let path = if self.path.trim().is_empty() {
            "(no path)"
        } else {
            self.path.trim()
        };
        format!("{}  ·  {path}", self.parser.label())
    }
}

#[derive(Debug, Clone)]
pub struct FilesPanelState {
    pub sources: Vec<FileSource>,
    pub selected: Option<usize>,
    pub pending_delete: Option<(usize, String)>,
}

impl Default for FilesPanelState {
    fn default() -> Self {
        Self {
            sources: vec![FileSource::default_csv()],
            selected: None,
            pending_delete: None,
        }
    }
}

impl FilesPanelState {
    pub fn choices(&self) -> Vec<(String, String)> {
        self.sources
            .iter()
            .map(|s| (s.id.clone(), s.display_name().to_string()))
            .collect()
    }

    fn ensure(&mut self) {
        if self.sources.is_empty() {
            self.sources.push(FileSource::default_csv());
            self.selected = None;
        } else if self.selected.is_some_and(|i| i >= self.sources.len()) {
            self.selected = None;
        }
    }

    fn unique_id(&self, base: &str) -> String {
        connection_list::unique_id(base, |id| self.sources.iter().any(|c| c.id == id))
    }
}

pub fn paint(ui: &mut egui::Ui, p: &ThemePalette, state: &mut FilesPanelState, host: &HostSnapshot) -> bool {
    state.ensure();
    let mut dirty = false;
    let full = ui.available_rect_before_wrap();
    let inner = form_inset(full);
    ui.allocate_new_ui(
        UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::top_down(egui::Align::Min)),
        |ui| dirty = paint_body(ui, p, state, host),
    );
    ui.allocate_rect(full, Sense::hover());
    dirty
}

fn paint_body(ui: &mut egui::Ui, p: &ThemePalette, state: &mut FilesPanelState, host: &HostSnapshot) -> bool {
    apply_form_spacing(ui);
    let (live, pack_loaded) = live_from_host(host);
    let mut dirty = false;
    ui.label(
        RichText::new("Growing CSV / JSON-lines files followed into points. Bind columns on a hub Files Input.")
            .font(font_ui(12.5))
            .color(p.muted),
    );
    ui.add_space(space::AFTER_BLURB);

    if connection_list::heading_with_add(ui, "Sources", "+ Add", p.live, p) {
        let mut s = FileSource::default_csv();
        s.id = state.unique_id("file");
        s.label = s.id.clone();
        state.sources.push(s);
        state.selected = Some(state.sources.len() - 1);
        dirty = true;
    }
    ui.add_space(space::AFTER_HEADING);

    let mut select = None;
    let show_delete = state.sources.len() > 1;
    for (i, s) in state.sources.iter().enumerate() {
        let label = s.display_name().to_string();
        let status = match live.iter().find(|l| l.id == s.id) {
            Some(l) => l.state.clone(),
            None if !pack_loaded => "pack not loaded".to_string(),
            None => "idle".to_string(),
        };
        let subtitle = format!("{}  ·  {status}", s.summary());
        match connection_list::selectable_row(
            ui,
            &label,
            &subtitle,
            state.selected == Some(i),
            show_delete,
            p,
        ) {
            Some(RowAction::Select) => select = Some(i),
            Some(RowAction::Delete) => state.pending_delete = Some((i, label)),
            None => {}
        }
        ui.add_space(2.0);
    }
    if let Some(i) = select {
        state.selected = if state.selected == Some(i) { None } else { Some(i) };
    }
    if let Some(i) = connection_list::confirm_delete(ui.ctx(), &mut state.pending_delete, p) {
        if i < state.sources.len() {
            let was = state.selected;
            state.sources.remove(i);
            state.selected = match was {
                Some(s) if s == i => None,
                Some(s) if s > i => Some(s - 1),
                other => other,
            };
            state.ensure();
            dirty = true;
        }
    }

    ui.add_space(space::AFTER_FIELD + 2.0);
    let Some(idx) = state.selected else {
        ui.label(
            RichText::new("Select a source to edit.")
                .font(font_ui(12.5))
                .color(p.muted),
        );
        return dirty;
    };
    let others: Vec<String> = state
        .sources
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != idx)
        .map(|(_, s)| s.id.clone())
        .collect();
    egui::ScrollArea::vertical()
        .id_salt("pb_files_cfg")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let s = &mut state.sources[idx];
            section(ui, "STATUS", p);
            let mine = live.iter().find(|l| l.id == s.id);
            status_line(ui, mine, pack_loaded, p);
            if let Some(l) = mine {
                let mut bits = vec![format!("{} rows", l.rows), format!("offset {} B", l.offset)];
                if l.bad_rows > 0 {
                    bits.push(format!("{} bad", l.bad_rows));
                }
                ui.label(RichText::new(bits.join("  ·  ")).font(font_ui(11.0)).color(p.muted));
            }
            ui.add_space(space::SECTION);
            section(ui, "SOURCE", p);
            let before = s.id.clone();
            dirty |= text_field(ui, "Id", &mut s.id, p);
            if s.id.trim().is_empty() || others.contains(&s.id) {
                s.id = before;
            }
            dirty |= text_field(ui, "Label", &mut s.label, p);
            dirty |= text_field(ui, "Path", &mut s.path, p);

            combo_row(ui, "Parser", p, |ui| {
                let mut v = s.parser;
                egui::ComboBox::from_id_salt("pb_files_parser")
                    .selected_text(v.label())
                    .width(160.0)
                    .show_ui(ui, |ui| {
                        for x in [FileParser::Csv, FileParser::Jsonl] {
                            ui.selectable_value(&mut v, x, x.label());
                        }
                    });
                if v != s.parser {
                    s.parser = v;
                    dirty = true;
                }
            });
            if s.parser == FileParser::Csv {
                dirty |= text_field(ui, "Delimiter", &mut s.delimiter, p);
                if s.delimiter.chars().count() > 1 {
                    s.delimiter = s.delimiter.chars().take(1).collect();
                }
                combo_row(ui, "Header", p, |ui| {
                    let mut v = s.header_row;
                    egui::ComboBox::from_id_salt("pb_files_header")
                        .selected_text(v.label())
                        .width(180.0)
                        .show_ui(ui, |ui| {
                            let line = match s.header_row {
                                HeaderMode::Line(n) => n,
                                _ => 1,
                            };
                            for x in [HeaderMode::Auto, HeaderMode::Line(line), HeaderMode::None] {
                                ui.selectable_value(&mut v, x, x.label());
                            }
                        });
                    if let HeaderMode::Line(mut n) = v {
                        if ui.add(egui::DragValue::new(&mut n).range(1..=10_000).speed(0.2)).changed() {
                            v = HeaderMode::Line(n);
                        }
                    }
                    if v != s.header_row {
                        s.header_row = v;
                        dirty = true;
                    }
                });
                let hint = match s.header_row {
                    HeaderMode::Auto => "Scans the top of the file until the column count settles: that block is the data, \
                                         its first row the column names, and lines above it (test / file info) are skipped.",
                    HeaderMode::Line(_) => "Lines above the names are skipped; data starts on the next line. Blank lines are not counted.",
                    HeaderMode::None => "Every line is data; bind columns by number (0, 1, 2…).",
                };
                ui.label(RichText::new(hint).font(font_ui(11.0)).color(p.muted));
                if let Some(l) = mine.filter(|l| !l.header.is_empty()) {
                    ui.label(RichText::new(format!("Detected: {}", l.header)).font(font_ui(11.5)).color(p.text));
                    if !l.preamble.is_empty() {
                        egui::CollapsingHeader::new(
                            RichText::new(format!("Preamble ({} lines)", l.preamble.len())).font(font_ui(11.5)).color(p.muted),
                        )
                        .id_salt(("pb_files_preamble", &s.id))
                        .show(ui, |ui| {
                            for line in &l.preamble {
                                ui.label(RichText::new(line).font(egui::FontId::monospace(11.0)).color(p.muted));
                            }
                        });
                    }
                }
            }

            ui.add_space(space::SECTION);
            section(ui, "FOLLOW", p);
            dirty |= drag_row(ui, "Poll (ms)", p, |ui| {
                ui.add(egui::DragValue::new(&mut s.poll_ms).range(50..=60_000).speed(10.0))
            });
            combo_row(ui, "Start at", p, |ui| {
                let mut v = s.start_at;
                egui::ComboBox::from_id_salt("pb_files_start")
                    .selected_text(v.label())
                    .width(220.0)
                    .show_ui(ui, |ui| {
                        for x in [FileStart::End, FileStart::Beginning] {
                            ui.selectable_value(&mut v, x, x.label());
                        }
                    });
                if v != s.start_at {
                    s.start_at = v;
                    dirty = true;
                }
            });
            combo_row(ui, "Rotation", p, |ui| {
                dirty |= ui.checkbox(&mut s.follow_rotation, "Reopen when the file is replaced or truncated").changed();
            });
            ui.add_space(space::AFTER_FIELD);
            ui.label(
                RichText::new(
                    "The service keeps the byte offset across restart and rotation, so seen rows are never re-read. \
                     The path is read on the machine running the service.",
                )
                .font(font_ui(11.0))
                .color(p.muted),
            );
        });
    dirty
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picker_columns_parse() {
        let j = r#"{"sources":[{"id":"m","columns":[{"name":"temp","sample":"21.5"},{"name":"run","sample":"true"}]}]}"#;
        assert_eq!(parse_live(j)[0].columns, vec![("temp".to_string(), "21.5".to_string()), ("run".to_string(), "true".to_string())]);
    }

    #[test]
    fn live_sources_parse_from_pack_json() {
        let j = r#"{"sources":[{"id":"m","path":"/x.csv","state":"following","connected":true,"error":"","rows":12,"bad_rows":1,"offset":345}]}"#;
        let l = parse_live(j);
        assert_eq!(l.len(), 1);
        assert_eq!((l[0].rows, l[0].bad_rows, l[0].offset), (12, 1, 345));
        assert!(l[0].columns.is_empty());
        assert!(l[0].connected);
        assert!(parse_live("").is_empty());
    }

    #[test]
    fn header_mode_json_and_live_detection() {
        let p = |j: &str| serde_json::from_str::<HeaderMode>(j).unwrap();
        assert_eq!(p("true"), HeaderMode::Auto, "legacy checkbox on");
        assert_eq!(p("false"), HeaderMode::None);
        assert_eq!(p("\"auto\""), HeaderMode::Auto);
        assert_eq!(p("5"), HeaderMode::Line(5));
        for m in [HeaderMode::Auto, HeaderMode::None, HeaderMode::Line(3)] {
            assert_eq!(p(&serde_json::to_string(&m).unwrap()), m);
        }
        let old: FileSource = serde_json::from_str(r#"{"id":"m","parser":"csv","header_row":true}"#).unwrap();
        assert_eq!(old.header_row, HeaderMode::Auto);
        let l = parse_live(r#"{"sources":[{"id":"m","header":"2 preamble lines · names on line 3","preamble":["Rig 4","Op: Sam"]}]}"#);
        assert_eq!(l[0].preamble, vec!["Rig 4", "Op: Sam"]);
        assert!(l[0].header.contains("names on line 3"));
    }
}
