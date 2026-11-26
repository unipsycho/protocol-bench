//! Help panel — searchable, tagged catalog of help articles with a formatted reader.
//!
//! Layout: search field + category pills on top; topic list on the left and the reader on the
//! right (stacked list → reader with a Back button when the pane is narrow). Content lives in
//! [`crate::help_catalog`]. Panel state is kept in egui memory so the shell needs no new fields.

use std::sync::OnceLock;

use egui::text::{LayoutJob, TextFormat};
use egui::{
    Align, Color32, CursorIcon, FontId, Key, Layout, Rect, RichText, Rounding, ScrollArea, Sense,
    Shape, Stroke, Ui, UiBuilder, Vec2,
};

use crate::help_catalog::{find, Article, Category, ARTICLES};
use crate::theme::{font_mono, font_semi, font_ui, form_inset, mix, tint, ThemePalette};

/// Below this width the list and reader stack instead of sitting side by side.
const WIDE_MIN: f32 = 620.0;

/// `selected` value for the Help home (landing) page.
const HOME: &str = "";

/// Common tasks linked from the Help home.
const COMMON: &[&str] = &["wiring", "output-nodes", "mqtt", "modbus", "point-quality", "troubleshooting"];

/// Example searches offered on the Help home.
const EXAMPLE_SEARCHES: &[&str] = &["modbus byte order", "mqtt broker", "#setup", "file rotation", "opc ua catalog"];

#[derive(Clone)]
struct HelpState {
    query: String,
    category: Option<Category>,
    selected: &'static str,
    /// Narrow layout: true while the reader is showing instead of the list.
    reading: bool,
}

impl Default for HelpState {
    fn default() -> Self {
        // Land on the Help home; in a narrow pane the home shows instead of the list.
        Self { query: String::new(), category: None, selected: HOME, reading: true }
    }
}

/// Things a click inside the reader asks the panel to do after drawing.
enum Nav {
    Open(&'static str),
    Tag(&'static str),
    Category(Category),
    Search(&'static str),
    Home,
    Back,
}

pub fn paint(ui: &mut Ui, p: &ThemePalette) {
    let full = ui.available_rect_before_wrap();
    let inner = form_inset(full);
    let state_id = ui.id().with("pb_help_state");
    let mut st: HelpState = ui.data_mut(|d| d.get_temp(state_id)).unwrap_or_default();

    ui.allocate_new_ui(
        UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)),
        |ui| {
            let wide = inner.width() >= WIDE_MIN;
            let results = search(&st.query, st.category);

            // ── search row ──
            let mut moved_by_key = false;
            if wide || !st.reading {
                moved_by_key = search_row(ui, &mut st, &results, p);
                ui.add_space(6.0);
                category_row(ui, &mut st, p);
                ui.add_space(8.0);
            }

            let mut nav: Option<Nav> = None;
            let body = ui.available_rect_before_wrap();
            if wide {
                let list_w = (body.width() * 0.34).clamp(220.0, 340.0);
                let list_rect = Rect::from_min_size(body.min, Vec2::new(list_w, body.height()));
                let read_rect = Rect::from_min_max(
                    egui::pos2(body.min.x + list_w + 12.0, body.min.y),
                    body.max,
                );
                ui.painter().vline(
                    body.min.x + list_w + 6.0,
                    body.y_range(),
                    Stroke::new(1.0, tint(p.line, 160)),
                );
                ui.allocate_new_ui(UiBuilder::new().max_rect(list_rect), |ui| {
                    if let Some(id) = topic_list(ui, &st, &results, moved_by_key, p) {
                        nav = Some(Nav::Open(id));
                    }
                });
                ui.allocate_new_ui(UiBuilder::new().max_rect(read_rect), |ui| {
                    if let Some(n) = reader(ui, &st, false, p) {
                        nav = Some(n);
                    }
                });
            } else if st.reading {
                if let Some(n) = reader(ui, &st, true, p) {
                    nav = Some(n);
                }
            } else if let Some(id) = topic_list(ui, &st, &results, moved_by_key, p) {
                nav = Some(Nav::Open(id));
            }

            match nav {
                Some(Nav::Open(id)) => {
                    st.selected = id;
                    st.reading = true;
                }
                Some(Nav::Tag(tag)) => {
                    st.query = format!("#{tag}");
                    st.category = None;
                    st.reading = false;
                }
                Some(Nav::Category(c)) => {
                    st.category = Some(c);
                    st.query.clear();
                    st.selected = ARTICLES.iter().find(|a| a.category == c).map_or(HOME, |a| a.id);
                    // Wide: filtered list + its first article. Narrow: the filtered list.
                    st.reading = false;
                }
                Some(Nav::Search(q)) => {
                    st.query = q.to_string();
                    st.category = None;
                    st.selected = search(q, None).first().map_or(HOME, |h| h.article.id);
                    st.reading = false;
                }
                Some(Nav::Home) => {
                    st.selected = HOME;
                    st.query.clear();
                    st.category = None;
                    st.reading = true;
                }
                Some(Nav::Back) => st.reading = false,
                None => {}
            }
        },
    );
    ui.data_mut(|d| d.insert_temp(state_id, st));
    ui.allocate_rect(full, Sense::hover());
}

// ───────────────────────────── search ─────────────────────────────

struct Hit {
    article: &'static Article,
    score: u32,
    snippet: Option<String>,
}

/// Lower-cased plain text of every article body (markup stripped), built once.
fn plain_bodies() -> &'static [String] {
    static CACHE: OnceLock<Vec<String>> = OnceLock::new();
    CACHE.get_or_init(|| ARTICLES.iter().map(|a| strip_markup(a.body)).collect())
}

fn strip_markup(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    for line in body.lines() {
        let t = line.trim();
        if t.starts_with("```") || t.starts_with("|---") || t.starts_with("|:--") {
            continue;
        }
        let t = t.trim_start_matches(['#', '>', '!', '-']).trim_start_matches("i>");
        for ch in t.chars() {
            if !matches!(ch, '*' | '`' | '|') {
                out.push(ch);
            } else if ch == '|' {
                out.push(' ');
            }
        }
        out.push(' ');
    }
    // Collapse runs of whitespace so snippets read cleanly.
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn terms(query: &str) -> Vec<String> {
    query.split_whitespace().map(|t| t.to_ascii_lowercase()).collect()
}

/// Terms to highlight in the reader (tag filters excluded).
fn highlight_terms(query: &str) -> Vec<String> {
    terms(query).into_iter().filter(|t| !t.starts_with('#') && t.len() >= 2).collect()
}

fn search(query: &str, category: Option<Category>) -> Vec<Hit> {
    let terms = terms(query);
    let bodies = plain_bodies();
    let mut hits: Vec<Hit> = Vec::new();
    'articles: for (i, a) in ARTICLES.iter().enumerate() {
        if category.is_some_and(|c| c != a.category) {
            continue;
        }
        let title = a.title.to_ascii_lowercase();
        let summary = a.summary.to_ascii_lowercase();
        let cat = a.category.label().to_ascii_lowercase();
        let body = bodies[i].to_ascii_lowercase();
        let mut score = 0u32;
        let mut snippet_term: Option<&str> = None;
        for t in &terms {
            if let Some(tag) = t.strip_prefix('#') {
                let hit = a.tags.iter().any(|g| g.to_ascii_lowercase().contains(tag))
                    || cat.contains(tag);
                if !hit {
                    continue 'articles;
                }
                score += 8;
                continue;
            }
            let mut s = 0;
            if title.contains(t.as_str()) {
                s += if title.split(|c: char| !c.is_alphanumeric()).any(|w| w == t) { 14 } else { 10 };
            }
            if a.tags.iter().any(|g| g.eq_ignore_ascii_case(t)) {
                s += 9;
            } else if a.tags.iter().any(|g| g.to_ascii_lowercase().contains(t.as_str())) {
                s += 5;
            }
            if summary.contains(t.as_str()) {
                s += 4;
            }
            if cat.contains(t.as_str()) {
                s += 2;
            }
            let body_n = body.matches(t.as_str()).count() as u32;
            s += body_n.min(6);
            if s == 0 {
                continue 'articles;
            }
            if body_n > 0 && snippet_term.is_none() && !title.contains(t.as_str()) {
                snippet_term = Some(t);
            }
            score += s;
        }
        let snippet = snippet_term.and_then(|t| snippet(&bodies[i], t));
        hits.push(Hit { article: a, score, snippet });
    }
    if !terms.is_empty() {
        hits.sort_by(|a, b| b.score.cmp(&a.score));
    }
    hits
}

/// About 110 characters of body text around the first match of `term`.
fn snippet(plain: &str, term: &str) -> Option<String> {
    let lower = plain.to_ascii_lowercase();
    let at = lower.find(term)?;
    let mut start = at.saturating_sub(36);
    while !plain.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (at + term.len() + 74).min(plain.len());
    while !plain.is_char_boundary(end) {
        end += 1;
    }
    let mut s = String::new();
    if start > 0 {
        s.push('…');
    }
    s.push_str(plain[start..end].trim());
    if end < plain.len() {
        s.push('…');
    }
    Some(s)
}

// ───────────────────────────── top rows ─────────────────────────────

/// Search field. Returns true when the arrow keys moved the selection this frame.
fn search_row(ui: &mut Ui, st: &mut HelpState, results: &[Hit], p: &ThemePalette) -> bool {
    let mut moved = false;
    ui.horizontal(|ui| {
        let clear_w = if st.query.is_empty() { 0.0 } else { 28.0 };
        let w = (ui.available_width() - clear_w - 84.0).max(120.0);
        ui.spacing_mut().item_spacing.x = 10.0;
        let edit = egui::TextEdit::singleline(&mut st.query)
            .id_salt("pb_help_search")
            .hint_text(crate::theme::hint(ui, "Search help — try “modbus byte order” or #setup"))
            .font(font_ui(13.0))
            .margin(Vec2::new(10.0, 6.0))
            .desired_width(w);
        let resp = ui.add(edit);
        if resp.changed() {
            st.reading = false;
            if let Some(first) = results.first() {
                if !results.iter().any(|h| h.article.id == st.selected) {
                    st.selected = first.article.id;
                }
            }
        }
        if resp.has_focus() || resp.lost_focus() {
            let (down, up, enter, esc) = ui.input(|i| {
                (
                    i.key_pressed(Key::ArrowDown),
                    i.key_pressed(Key::ArrowUp),
                    i.key_pressed(Key::Enter),
                    i.key_pressed(Key::Escape),
                )
            });
            if down || up {
                let pos = results.iter().position(|h| h.article.id == st.selected);
                let next = match (pos, down) {
                    (None, _) => 0,
                    (Some(i), true) => (i + 1).min(results.len().saturating_sub(1)),
                    (Some(i), false) => i.saturating_sub(1),
                };
                if let Some(h) = results.get(next) {
                    st.selected = h.article.id;
                    moved = true;
                }
            }
            if enter && resp.lost_focus() {
                if !results.iter().any(|h| h.article.id == st.selected) {
                    if let Some(h) = results.first() {
                        st.selected = h.article.id;
                    }
                }
                st.reading = true;
            }
            if esc {
                st.query.clear();
            }
        }
        if !st.query.is_empty() {
            let x = ui
                .add(egui::Button::new(RichText::new("×").color(p.muted)).frame(false))
                .on_hover_text("Clear search");
            if x.clicked() {
                st.query.clear();
            }
        }
        let n = results.len();
        ui.label(
            RichText::new(if n == 1 { "1 topic".to_string() } else { format!("{n} topics") })
                .font(font_ui(11.5))
                .color(p.muted),
        );
    });
    moved
}

fn category_row(ui: &mut Ui, st: &mut HelpState, p: &ThemePalette) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
        if pill(ui, "All", None, st.category.is_none(), Some(ARTICLES.len()), p).clicked() {
            st.category = None;
        }
        for c in Category::ALL {
            let n = ARTICLES.iter().filter(|a| a.category == c).count();
            let on = st.category == Some(c);
            if pill(ui, c.label(), Some(c.color()), on, Some(n), p).clicked() {
                st.category = if on { None } else { Some(c) };
                st.reading = false;
            }
        }
    });
}

/// Rounded pill with an optional colour dot and count.
fn pill(
    ui: &mut Ui,
    label: &str,
    dot: Option<Color32>,
    selected: bool,
    count: Option<usize>,
    p: &ThemePalette,
) -> egui::Response {
    let font = font_ui(11.5);
    let text_c = if selected { p.text } else { mix(p.muted, p.text, 0.35) };
    let label_g = ui.fonts(|f| f.layout_no_wrap(label.to_string(), font.clone(), text_c));
    let count_g = count.map(|n| ui.fonts(|f| f.layout_no_wrap(n.to_string(), font_ui(10.0), p.muted)));
    let dot_w = if dot.is_some() { 12.0 } else { 0.0 };
    let count_w = count_g.as_ref().map_or(0.0, |g| g.size().x + 7.0);
    let size = Vec2::new(10.0 + dot_w + label_g.size().x + count_w + 10.0, 22.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let accent = dot.unwrap_or(p.accent);
    let fill = if selected {
        tint(accent, 52)
    } else if resp.hovered() {
        tint(p.control, 255)
    } else {
        tint(p.control, 150)
    };
    let stroke = if selected { Stroke::new(1.0, tint(accent, 200)) } else { Stroke::new(1.0, tint(p.line, 140)) };
    ui.painter().rect(rect, Rounding::same(11.0), fill, stroke);
    let mut x = rect.min.x + 10.0;
    if let Some(c) = dot {
        ui.painter().circle_filled(egui::pos2(x + 3.5, rect.center().y), 3.5, c);
        x += dot_w;
    }
    let ly = rect.center().y - label_g.size().y / 2.0;
    let lw = label_g.size().x;
    ui.painter().galley(egui::pos2(x, ly), label_g, text_c);
    if let Some(g) = count_g {
        let gy = rect.center().y - g.size().y / 2.0;
        ui.painter().galley(egui::pos2(x + lw + 6.0, gy), g, p.muted);
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

// ───────────────────────────── topic list ─────────────────────────────

fn topic_list(
    ui: &mut Ui,
    st: &HelpState,
    results: &[Hit],
    scroll_to_selected: bool,
    p: &ThemePalette,
) -> Option<&'static str> {
    let mut clicked = None;
    let searching = !st.query.trim().is_empty();
    let terms = highlight_terms(&st.query);
    ScrollArea::vertical()
        .id_salt("pb_help_list")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            if results.is_empty() {
                ui.add_space(18.0);
                ui.label(RichText::new("No topics match.").font(font_semi(13.5)).color(p.text));
                ui.add_space(4.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(
                            "Try fewer words, a protocol name (mqtt, modbus, opc ua), or a tag like #setup.",
                        )
                        .font(font_ui(12.0))
                        .color(p.muted),
                    )
                    .wrap(),
                );
                return;
            }
            if !searching {
                let home = st.selected == HOME;
                let label = RichText::new("Help home")
                    .font(font_semi(12.5))
                    .color(if home { p.text } else { p.accent });
                let r = ui
                    .add(egui::Button::new(label).frame(home).fill(tint(p.accent, 40)))
                    .on_hover_text("Welcome page with the Getting started path")
                    .on_hover_cursor(CursorIcon::PointingHand);
                if r.clicked() {
                    clicked = Some(HOME);
                }
                ui.add_space(4.0);
            }
            let mut last_cat: Option<Category> = None;
            for hit in results {
                if !searching && last_cat != Some(hit.article.category) {
                    last_cat = Some(hit.article.category);
                    ui.add_space(if ui.cursor().min.y > ui.min_rect().min.y + 4.0 { 10.0 } else { 2.0 });
                    group_heading(ui, hit.article.category, p);
                }
                let selected = hit.article.id == st.selected;
                let resp = topic_row(ui, hit, selected, &terms, p);
                if selected && scroll_to_selected {
                    resp.scroll_to_me(None);
                }
                if resp.clicked() {
                    clicked = Some(hit.article.id);
                }
            }
            ui.add_space(12.0);
        });
    clicked
}

fn group_heading(ui: &mut Ui, cat: Category, p: &ThemePalette) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(Vec2::new(8.0, 14.0), Sense::hover());
        ui.painter().rect_filled(
            Rect::from_center_size(r.center(), Vec2::new(4.0, 12.0)),
            Rounding::same(2.0),
            cat.color(),
        );
        ui.label(
            RichText::new(cat.label().to_uppercase())
                .font(font_semi(10.5))
                .color(mix(cat.color(), p.text, if p.is_light { 0.55 } else { 0.25 })),
        );
    });
    ui.add_space(2.0);
}

fn topic_row(
    ui: &mut Ui,
    hit: &Hit,
    selected: bool,
    terms: &[String],
    p: &ThemePalette,
) -> egui::Response {
    let a = hit.article;
    let color = a.color();
    let w = ui.available_width();
    let text_w = (w - 22.0).max(60.0);

    let title = ui.fonts(|f| {
        f.layout_job(inline_job(a.title, font_semi(13.0), p.text, p, terms, text_w))
    });
    let summary = ui.fonts(|f| {
        f.layout_job(inline_job(a.summary, font_ui(11.5), p.muted, p, terms, text_w))
    });
    let snippet = hit.snippet.as_ref().map(|s| {
        ui.fonts(|f| f.layout_job(inline_job(s, font_ui(11.0), mix(p.muted, p.text, 0.25), p, terms, text_w)))
    });
    let h = 8.0
        + title.size().y
        + 2.0
        + summary.size().y
        + snippet.as_ref().map_or(0.0, |g| g.size().y + 4.0)
        + 8.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click());
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, Rounding::same(6.0), tint(color, if p.is_light { 40 } else { 30 }));
        painter.rect_stroke(rect, Rounding::same(6.0), Stroke::new(1.0, tint(color, 120)));
    } else if resp.hovered() {
        painter.rect_filled(rect, Rounding::same(6.0), tint(p.control, 200));
    }
    let stripe = Rect::from_min_size(
        egui::pos2(rect.min.x + 5.0, rect.min.y + 8.0),
        Vec2::new(3.0, h - 16.0),
    );
    painter.rect_filled(stripe, Rounding::same(1.5), if selected { color } else { tint(color, 150) });
    let x = rect.min.x + 16.0;
    let mut y = rect.min.y + 8.0;
    let th = title.size().y;
    painter.galley(egui::pos2(x, y), title, p.text);
    y += th + 2.0;
    let sh = summary.size().y;
    painter.galley(egui::pos2(x, y), summary, p.muted);
    y += sh + 4.0;
    if let Some(g) = snippet {
        painter.galley(egui::pos2(x, y), g, p.muted);
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

// ───────────────────────────── reader ─────────────────────────────

fn reader(ui: &mut Ui, st: &HelpState, narrow: bool, p: &ThemePalette) -> Option<Nav> {
    if st.selected == HOME {
        return home_page(ui, narrow, p);
    }
    let a = find(st.selected).unwrap_or(&ARTICLES[0]);
    let color = a.color();
    let terms = highlight_terms(&st.query);
    let mut nav = None;

    if narrow {
        let back = ui
            .add(egui::Button::new(RichText::new("‹  All topics").font(font_ui(12.5)).color(p.accent)).frame(false))
            .on_hover_cursor(CursorIcon::PointingHand);
        if back.clicked() {
            nav = Some(Nav::Back);
        }
        ui.add_space(4.0);
    }

    ScrollArea::vertical()
        .id_salt(("pb_help_reader", a.id))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let w = ui.available_width().min(760.0);
            ui.set_max_width(w);
            ui.spacing_mut().item_spacing.y = 4.0;

            // Breadcrumb: Help home › category
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let home = ui
                    .add(egui::Button::new(RichText::new("HELP HOME").font(font_semi(10.5)).color(p.accent)).frame(false))
                    .on_hover_text("Back to the Help home page")
                    .on_hover_cursor(CursorIcon::PointingHand);
                if home.clicked() {
                    nav = Some(Nav::Home);
                }
                ui.label(RichText::new("›").font(font_ui(11.0)).color(p.muted));
                let (r, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                ui.painter().circle_filled(r.center(), 4.0, a.category.color());
                ui.label(
                    RichText::new(a.category.label().to_uppercase())
                        .font(font_semi(10.5))
                        .color(mix(a.category.color(), p.text, if p.is_light { 0.55 } else { 0.2 })),
                );
            });
            ui.add_space(2.0);

            // Title + accent underline
            ui.add(egui::Label::new(RichText::new(a.title).font(font_semi(22.0)).color(p.text)).wrap());
            let (bar, _) = ui.allocate_exact_size(Vec2::new(w, 6.0), Sense::hover());
            let y = bar.center().y;
            ui.painter().line_segment(
                [egui::pos2(bar.min.x, y), egui::pos2(bar.min.x + 56.0, y)],
                Stroke::new(3.0, color),
            );
            ui.painter().line_segment(
                [egui::pos2(bar.min.x + 62.0, y), egui::pos2(bar.max.x, y)],
                Stroke::new(1.0, tint(p.line, 140)),
            );
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(inline_job(a.summary, font_ui(13.5), mix(p.muted, p.text, 0.35), p, &terms, w))
                    .wrap(),
            );
            ui.add_space(6.0);

            // Tags
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(5.0, 5.0);
                for tag in a.tags {
                    if tag_chip(ui, tag, color, p).on_hover_text(format!("Show every topic tagged #{tag}")).clicked() {
                        nav = Some(Nav::Tag(tag));
                    }
                }
            });
            ui.add_space(10.0);

            render_body(ui, a.body, color, p, &terms);

            // See also
            if !a.related.is_empty() {
                ui.add_space(16.0);
                section_label(ui, "SEE ALSO", p);
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
                    for id in a.related {
                        if let Some(r) = find(id) {
                            if related_card(ui, r, p).clicked() {
                                nav = Some(Nav::Open(r.id));
                            }
                        }
                    }
                });
            }

            // Prev / next within the category
            let peers: Vec<&Article> = ARTICLES.iter().filter(|x| x.category == a.category).collect();
            if let Some(pos) = peers.iter().position(|x| x.id == a.id) {
                ui.add_space(18.0);
                let (sep, _) = ui.allocate_exact_size(Vec2::new(w, 1.0), Sense::hover());
                ui.painter().hline(sep.x_range(), sep.center().y, Stroke::new(1.0, tint(p.line, 140)));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if pos > 0 {
                        let prev = peers[pos - 1];
                        if nav_link(ui, &format!("‹  {}", prev.title), p).clicked() {
                            nav = Some(Nav::Open(prev.id));
                        }
                    }
                    if let Some(next) = peers.get(pos + 1) {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if nav_link(ui, &format!("{}  ›", next.title), p).clicked() {
                                nav = Some(Nav::Open(next.id));
                            }
                        });
                    }
                });
            }
            ui.add_space(24.0);
        });
    nav
}

// ───────────────────────────── home ─────────────────────────────

/// The landing page: a first-time welcome, the Getting started path, common tasks,
/// category cards and how to search.
fn home_page(ui: &mut Ui, narrow: bool, p: &ThemePalette) -> Option<Nav> {
    let mut nav = None;
    let accent = Category::GettingStarted.color();
    ScrollArea::vertical()
        .id_salt("pb_help_home")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let w = ui.available_width().min(780.0);
            ui.set_max_width(w);
            ui.spacing_mut().item_spacing.y = 4.0;

            // Hero
            ui.label(RichText::new("HELP").font(font_semi(10.5)).color(accent));
            ui.add(
                egui::Label::new(RichText::new("Welcome to Protocol Bench").font(font_semi(24.0)).color(p.text))
                    .wrap(),
            );
            let (bar, _) = ui.allocate_exact_size(Vec2::new(w, 6.0), Sense::hover());
            ui.painter().line_segment(
                [egui::pos2(bar.min.x, bar.center().y), egui::pos2(bar.min.x + 56.0, bar.center().y)],
                Stroke::new(3.0, accent),
            );
            ui.add_space(4.0);
            let intro = "Protocol Bench moves live values between industrial and oilfield links — **Modbus**, \
**MQTT**, **OPC UA**, serial and TCP **streams**, **HTTP**, growing **files**, **LAS** and **SNMP**. A value that \
arrives on one link can leave on another. This guide explains how to set it up, how to configure each protocol, \
and how to route data from where it comes in to where it needs to go.";
            ui.add(egui::Label::new(inline_job(intro, font_ui(13.5), p.text, p, &[], w)).wrap());
            ui.add_space(4.0);
            callout(
                ui,
                CalloutKind::Tip,
                "**The idea in one line:** connections live in the protocol panels → **Input** nodes turn \
fields into **points** → **wires** carry points → **Output** nodes send them out. A **hub** groups it all.",
                p,
                &[],
            );

            // Start here
            heading(ui, "New here? Start with these", accent, p);
            ui.add(
                egui::Label::new(
                    RichText::new(
                        "Five short topics, best read in order. Together they take about ten minutes and \
cover the ideas everything else builds on.",
                    )
                    .font(font_ui(12.5))
                    .color(p.muted),
                )
                .wrap(),
            );
            ui.add_space(4.0);
            let start: Vec<&Article> =
                ARTICLES.iter().filter(|a| a.category == Category::GettingStarted).collect();
            for (i, a) in start.iter().enumerate() {
                if start_row(ui, i + 1, a, p).clicked() {
                    nav = Some(Nav::Open(a.id));
                }
            }

            // Common tasks
            heading(ui, "Common tasks", p.accent, p);
            let items: Vec<CardItem> = COMMON
                .iter()
                .filter_map(|id| find(id))
                .map(|a| CardItem { title: a.title, meta: a.category.label(), body: a.summary, color: a.color() })
                .collect();
            if let Some(i) = card_grid(ui, &items, 230.0, p) {
                nav = Some(Nav::Open(find(COMMON[i]).map_or("welcome", |a| a.id)));
            }

            // Browse by category
            heading(ui, "Browse by category", p.accent, p);
            ui.add(
                egui::Label::new(
                    RichText::new(
                        "Every topic belongs to one category. Pick a card here, or use the coloured \
category buttons under the search box to filter the topic list at any time.",
                    )
                    .font(font_ui(12.5))
                    .color(p.muted),
                )
                .wrap(),
            );
            ui.add_space(4.0);
            let counts: Vec<String> = Category::ALL
                .iter()
                .map(|c| {
                    let n = ARTICLES.iter().filter(|a| a.category == *c).count();
                    if n == 1 { "1 topic".into() } else { format!("{n} topics") }
                })
                .collect();
            let cats: Vec<CardItem> = Category::ALL
                .iter()
                .zip(counts.iter())
                .map(|(c, n)| CardItem { title: c.label(), meta: n.as_str(), body: c.blurb(), color: c.color() })
                .collect();
            if let Some(i) = card_grid(ui, &cats, 230.0, p) {
                nav = Some(Nav::Category(Category::ALL[i]));
            }

            // Search
            heading(ui, "Search", p.accent, p);
            let how = "Type in the **search box** at the top to look through every topic's title, tags and \
text. Every word must match; the best matches come first and the words you typed are highlighted. Start a \
word with `#` to filter by tag. Use **↑ ↓** to move through results, **Enter** to open one and **Esc** to clear.";
            ui.add(egui::Label::new(inline_job(how, font_ui(13.0), p.text, p, &[], w)).wrap());
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
                ui.label(RichText::new("Try:").font(font_ui(12.0)).color(p.muted));
                for q in EXAMPLE_SEARCHES {
                    if pill(ui, q, None, false, None, p).on_hover_text("Search for this").clicked() {
                        nav = Some(Nav::Search(q));
                    }
                }
            });

            if narrow {
                ui.add_space(14.0);
                if nav_link(ui, "Browse all topics  ›", p).clicked() {
                    nav = Some(Nav::Back);
                }
            }
            ui.add_space(24.0);
        });
    nav
}

/// One numbered row of the "start here" path.
fn start_row(ui: &mut Ui, n: usize, a: &Article, p: &ThemePalette) -> egui::Response {
    let color = a.color();
    let w = ui.available_width();
    let text_w = (w - 96.0).max(80.0);
    let title = ui.fonts(|f| f.layout_job(inline_job(a.title, font_semi(13.5), p.text, p, &[], text_w)));
    let sum = ui.fonts(|f| f.layout_job(inline_job(a.summary, font_ui(12.0), p.muted, p, &[], text_w)));
    let h = (12.0 + title.size().y + 2.0 + sum.size().y + 12.0).max(48.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click());
    let fill = if resp.hovered() { tint(color, 34) } else { tint(p.control, 120) };
    ui.painter().rect(rect, Rounding::same(7.0), fill, Stroke::new(1.0, tint(color, if resp.hovered() { 150 } else { 70 })));
    let c = egui::pos2(rect.min.x + 24.0, rect.center().y);
    ui.painter().circle_filled(c, 12.0, tint(color, 220));
    let ng = ui.fonts(|f| f.layout_no_wrap(n.to_string(), font_semi(12.0), Color32::WHITE));
    ui.painter().galley(c - ng.size() / 2.0, ng, Color32::WHITE);
    let th = title.size().y;
    let x = rect.min.x + 48.0;
    let top = rect.center().y - (th + 2.0 + sum.size().y) / 2.0;
    ui.painter().galley(egui::pos2(x, top), title, p.text);
    ui.painter().galley(egui::pos2(x, top + th + 2.0), sum, p.muted);
    let rg = ui.fonts(|f| f.layout_no_wrap("Read  ›".into(), font_ui(12.0), p.accent));
    let rpos = egui::pos2(rect.max.x - 12.0 - rg.size().x, rect.center().y - rg.size().y / 2.0);
    ui.painter().galley(rpos, rg, p.accent);
    ui.add_space(4.0);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

struct CardItem<'a> {
    title: &'a str,
    meta: &'a str,
    body: &'a str,
    color: Color32,
}

/// Wrapping grid of equal-width cards; each row takes its tallest card's height.
/// Returns the index of the clicked card.
fn card_grid(ui: &mut Ui, items: &[CardItem<'_>], min_w: f32, p: &ThemePalette) -> Option<usize> {
    let gap = 8.0;
    let avail = ui.available_width();
    let cols = (((avail + gap) / (min_w + gap)).floor() as usize).clamp(1, 3);
    let card_w = (avail - gap * (cols as f32 - 1.0)) / cols as f32;
    let text_w = card_w - 28.0;
    let mut clicked = None;
    for (row_i, row) in items.chunks(cols).enumerate() {
        let laid: Vec<_> = row
            .iter()
            .map(|it| {
                let t = ui.fonts(|f| f.layout_job(inline_job(it.title, font_semi(13.0), p.text, p, &[], text_w)));
                let m = ui.fonts(|f| f.layout_no_wrap(it.meta.to_string(), font_ui(10.5), p.muted));
                let b = ui.fonts(|f| f.layout_job(inline_job(it.body, font_ui(11.5), p.muted, p, &[], text_w)));
                (t, m, b)
            })
            .collect();
        let h = laid
            .iter()
            .map(|(t, m, b)| 10.0 + t.size().y + 1.0 + m.size().y + 5.0 + b.size().y + 10.0)
            .fold(0.0, f32::max);
        let (row_rect, _) = ui.allocate_exact_size(Vec2::new(avail, h), Sense::hover());
        for (ci, (it, (t, m, b))) in row.iter().zip(laid).enumerate() {
            let x0 = row_rect.min.x + ci as f32 * (card_w + gap);
            let rect = Rect::from_min_size(egui::pos2(x0, row_rect.min.y), Vec2::new(card_w, h));
            let resp = ui
                .interact(rect, ui.id().with(("pb_help_card", it.title, row_i, ci)), Sense::click())
                .on_hover_cursor(CursorIcon::PointingHand);
            let fill = if resp.hovered() { tint(it.color, 38) } else { tint(p.control, 120) };
            let stroke = Stroke::new(1.0, tint(it.color, if resp.hovered() { 160 } else { 80 }));
            ui.painter().rect(rect, Rounding::same(7.0), fill, stroke);
            ui.painter().rect_filled(
                Rect::from_min_size(rect.min + Vec2::new(7.0, 10.0), Vec2::new(3.0, h - 20.0)),
                Rounding::same(1.5),
                it.color,
            );
            let x = rect.min.x + 18.0;
            let mut y = rect.min.y + 10.0;
            let (th, mh) = (t.size().y, m.size().y);
            ui.painter().galley(egui::pos2(x, y), t, p.text);
            y += th + 1.0;
            ui.painter().galley(egui::pos2(x, y), m, p.muted);
            y += mh + 5.0;
            ui.painter().galley(egui::pos2(x, y), b, p.muted);
            if resp.clicked() {
                clicked = Some(row_i * cols + ci);
            }
        }
        ui.add_space(gap - 4.0);
    }
    clicked
}

fn section_label(ui: &mut Ui, text: &str, p: &ThemePalette) {
    ui.label(RichText::new(text).font(font_semi(10.5)).color(p.muted));
}

fn tag_chip(ui: &mut Ui, tag: &str, color: Color32, p: &ThemePalette) -> egui::Response {
    let g = ui.fonts(|f| f.layout_no_wrap(format!("#{tag}"), font_mono(11.0), mix(color, p.text, 0.45)));
    let size = g.size() + Vec2::new(14.0, 6.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let fill = if resp.hovered() { tint(color, 60) } else { tint(color, 28) };
    ui.painter().rect(rect, Rounding::same(4.0), fill, Stroke::new(1.0, tint(color, 90)));
    let c = mix(color, p.text, 0.45);
    ui.painter().galley(rect.min + Vec2::new(7.0, 3.0), g, c);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

fn related_card(ui: &mut Ui, a: &Article, p: &ThemePalette) -> egui::Response {
    let color = a.color();
    let g = ui.fonts(|f| f.layout_no_wrap(a.title.to_string(), font_semi(12.0), p.text));
    let c = ui.fonts(|f| f.layout_no_wrap(a.category.label().to_string(), font_ui(10.0), p.muted));
    let w = g.size().x.max(c.size().x) + 28.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 40.0), Sense::click());
    let fill = if resp.hovered() { tint(color, 40) } else { tint(p.control, 170) };
    ui.painter().rect(rect, Rounding::same(6.0), fill, Stroke::new(1.0, tint(color, 110)));
    ui.painter().rect_filled(
        Rect::from_min_size(rect.min + Vec2::new(6.0, 8.0), Vec2::new(3.0, 24.0)),
        Rounding::same(1.5),
        color,
    );
    ui.painter().galley(rect.min + Vec2::new(16.0, 5.0), g, p.text);
    ui.painter().galley(rect.min + Vec2::new(16.0, 23.0), c, p.muted);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

fn nav_link(ui: &mut Ui, text: &str, p: &ThemePalette) -> egui::Response {
    ui.add(egui::Button::new(RichText::new(text).font(font_ui(12.5)).color(p.accent)).frame(false))
        .on_hover_cursor(CursorIcon::PointingHand)
}

// ───────────────────────────── markup rendering ─────────────────────────────

fn render_body(ui: &mut Ui, body: &str, accent: Color32, p: &ThemePalette, terms: &[String]) {
    let lines: Vec<&str> = body.lines().collect();
    let mut i = 0;
    let mut para = String::new();
    let width = ui.available_width();
    let flush = |ui: &mut Ui, para: &mut String| {
        if !para.trim().is_empty() {
            ui.add(egui::Label::new(inline_job(para.trim(), font_ui(13.0), p.text, p, terms, width)).wrap());
            ui.add_space(6.0);
        }
        para.clear();
    };
    while i < lines.len() {
        let raw = lines[i];
        let t = raw.trim();
        if t.is_empty() {
            flush(ui, &mut para);
            i += 1;
            continue;
        }
        if let Some(h) = t.strip_prefix("### ") {
            flush(ui, &mut para);
            ui.add_space(6.0);
            ui.label(RichText::new(h).font(font_semi(13.5)).color(mix(accent, p.text, 0.55)));
            ui.add_space(2.0);
        } else if let Some(h) = t.strip_prefix("## ") {
            flush(ui, &mut para);
            heading(ui, h, accent, p);
        } else if let Some(h) = t.strip_prefix("# ") {
            flush(ui, &mut para);
            heading(ui, h, accent, p);
        } else if t.starts_with("```") {
            flush(ui, &mut para);
            let mut code = Vec::new();
            i += 1;
            while i < lines.len() && !lines[i].trim().starts_with("```") {
                code.push(lines[i]);
                i += 1;
            }
            code_block(ui, &code.join("\n"), accent, p);
        } else if t.starts_with('|') {
            flush(ui, &mut para);
            let mut rows: Vec<Vec<String>> = Vec::new();
            while i < lines.len() && lines[i].trim().starts_with('|') {
                let row = lines[i].trim();
                let is_sep = row.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '));
                if !is_sep {
                    let cells = row.trim_matches('|').split('|').map(|c| c.trim().to_string()).collect();
                    rows.push(cells);
                }
                i += 1;
            }
            table(ui, &rows, accent, p, terms);
            continue;
        } else if let Some((kind, text)) = callout_prefix(t) {
            flush(ui, &mut para);
            let mut buf = text.to_string();
            while i + 1 < lines.len() {
                let n = lines[i + 1].trim();
                match callout_prefix(n) {
                    Some((k2, more)) if k2 == kind => {
                        buf.push(' ');
                        buf.push_str(more);
                        i += 1;
                    }
                    _ => break,
                }
            }
            callout(ui, kind, &buf, p, terms);
        } else if let Some((depth, text)) = bullet(raw) {
            flush(ui, &mut para);
            list_item(ui, ListMark::Bullet(depth), text, accent, p, terms);
        } else if let Some((n, text)) = numbered(t) {
            flush(ui, &mut para);
            list_item(ui, ListMark::Number(n), text, accent, p, terms);
        } else {
            if !para.is_empty() {
                para.push(' ');
            }
            para.push_str(t);
        }
        i += 1;
    }
    flush(ui, &mut para);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CalloutKind {
    Tip,
    Caution,
    Note,
}

fn callout_prefix(t: &str) -> Option<(CalloutKind, &str)> {
    if let Some(r) = t.strip_prefix("!>") {
        Some((CalloutKind::Caution, r.trim()))
    } else if let Some(r) = t.strip_prefix("i>") {
        Some((CalloutKind::Note, r.trim()))
    } else {
        t.strip_prefix('>').map(|r| (CalloutKind::Tip, r.trim()))
    }
}

fn bullet(raw: &str) -> Option<(usize, &str)> {
    let indent = raw.len() - raw.trim_start().len();
    let t = raw.trim_start();
    let rest = t.strip_prefix("- ").or_else(|| t.strip_prefix("• "))?;
    Some((indent / 2, rest))
}

fn numbered(t: &str) -> Option<(u32, &str)> {
    let dot = t.find(". ")?;
    let n: u32 = t[..dot].parse().ok()?;
    Some((n, &t[dot + 2..]))
}

fn heading(ui: &mut Ui, text: &str, accent: Color32, p: &ThemePalette) {
    ui.add_space(10.0);
    let color = mix(accent, p.text, if p.is_light { 0.45 } else { 0.15 });
    ui.label(RichText::new(text).font(font_semi(16.0)).color(color));
    let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 3.0), Sense::hover());
    ui.painter().hline(
        r.min.x..=r.min.x + 32.0,
        r.center().y,
        Stroke::new(2.0, tint(accent, 200)),
    );
    ui.add_space(4.0);
}

enum ListMark {
    Bullet(usize),
    Number(u32),
}

fn list_item(ui: &mut Ui, mark: ListMark, text: &str, accent: Color32, p: &ThemePalette, terms: &[String]) {
    let depth = match mark {
        ListMark::Bullet(d) => d,
        ListMark::Number(_) => 0,
    };
    let indent = 4.0 + depth as f32 * 18.0;
    let gutter = 22.0;
    let avail = ui.available_width();
    let text_w = (avail - indent - gutter).max(60.0);
    let job = inline_job(text, font_ui(13.0), p.text, p, terms, text_w);
    let galley = ui.fonts(|f| f.layout_job(job));
    let h = galley.size().y.max(18.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(avail, h), Sense::hover());
    let first_line_mid = rect.min.y + 9.0;
    let mx = rect.min.x + indent + 7.0;
    match mark {
        ListMark::Bullet(d) => {
            if d == 0 {
                ui.painter().circle_filled(egui::pos2(mx, first_line_mid), 3.0, accent);
            } else {
                ui.painter().circle_stroke(egui::pos2(mx, first_line_mid), 2.6, Stroke::new(1.2, accent));
            }
        }
        ListMark::Number(n) => {
            let c = egui::pos2(mx + 2.0, first_line_mid);
            ui.painter().circle_filled(c, 9.0, tint(accent, 210));
            let ng = ui.fonts(|f| f.layout_no_wrap(n.to_string(), font_semi(10.5), Color32::WHITE));
            let pos = c - ng.size() / 2.0;
            ui.painter().galley(pos, ng, Color32::WHITE);
        }
    }
    ui.painter().galley(egui::pos2(rect.min.x + indent + gutter, rect.min.y), galley, p.text);
    ui.add_space(3.0);
}

fn callout(ui: &mut Ui, kind: CalloutKind, text: &str, p: &ThemePalette, terms: &[String]) {
    let (color, label) = match kind {
        CalloutKind::Tip => (p.ok, "TIP"),
        CalloutKind::Caution => (p.warn, "CAUTION"),
        CalloutKind::Note => (crate::theme::Proto::INDIGO, "NOTE"),
    };
    ui.add_space(4.0);
    let avail = ui.available_width();
    let resp = egui::Frame::none()
        .fill(tint(color, if p.is_light { 34 } else { 22 }))
        .stroke(Stroke::new(1.0, tint(color, 80)))
        .rounding(Rounding::same(6.0))
        .inner_margin(egui::Margin { left: 16.0, right: 12.0, top: 8.0, bottom: 8.0 })
        .show(ui, |ui| {
            ui.set_width(avail - 28.0);
            ui.label(RichText::new(label).font(font_semi(10.0)).color(mix(color, p.text, 0.3)));
            ui.add(egui::Label::new(inline_job(text, font_ui(12.5), p.text, p, terms, avail - 28.0)).wrap());
        })
        .response;
    let r = resp.rect;
    ui.painter().rect_filled(
        Rect::from_min_max(r.min, egui::pos2(r.min.x + 4.0, r.max.y)),
        Rounding { nw: 6.0, sw: 6.0, ne: 0.0, se: 0.0 },
        color,
    );
    ui.add_space(6.0);
}

fn code_block(ui: &mut Ui, code: &str, accent: Color32, p: &ThemePalette) {
    let bg = if p.is_light { mix(p.pane, Color32::BLACK, 0.06) } else { mix(p.bg, Color32::BLACK, 0.35) };
    let avail = ui.available_width();
    ui.add_space(2.0);
    egui::Frame::none()
        .fill(bg)
        .stroke(Stroke::new(1.0, tint(p.line, 160)))
        .rounding(Rounding::same(6.0))
        .inner_margin(egui::Margin::symmetric(12.0, 9.0))
        .show(ui, |ui| {
            ui.set_width(avail - 24.0);
            let code_c = mix(accent, p.text, if p.is_light { 0.25 } else { 0.55 });
            ui.add(egui::Label::new(RichText::new(code).font(font_mono(12.0)).color(code_c)).wrap());
        });
    ui.add_space(6.0);
}

fn table(ui: &mut Ui, rows: &[Vec<String>], accent: Color32, p: &ThemePalette, terms: &[String]) {
    if rows.is_empty() {
        return;
    }
    let cols = rows.iter().map(|r| r.len()).max().unwrap_or(1);
    let avail = ui.available_width();
    let pad = 8.0;
    // Column weights from the longest cell text, softened so narrow columns stay readable.
    let mut weights = vec![4.0f32; cols];
    for r in rows {
        for (c, cell) in r.iter().enumerate() {
            let len = cell.chars().filter(|ch| !matches!(ch, '*' | '`')).count() as f32;
            weights[c] = weights[c].max(len.min(60.0));
        }
    }
    let weights: Vec<f32> = weights.iter().map(|w| w.sqrt().max(2.4)).collect();
    let total: f32 = weights.iter().sum();
    let widths: Vec<f32> = weights.iter().map(|w| avail * w / total).collect();

    ui.add_space(2.0);
    let start_y = ui.cursor().min.y;
    let left = ui.cursor().min.x;
    for (ri, r) in rows.iter().enumerate() {
        let header = ri == 0;
        let font = if header { font_semi(11.5) } else { font_ui(12.5) };
        let color = if header { mix(accent, p.text, if p.is_light { 0.4 } else { 0.25 }) } else { p.text };
        let galleys: Vec<_> = (0..cols)
            .map(|c| {
                let txt = r.get(c).map(String::as_str).unwrap_or("");
                let job = inline_job(txt, font.clone(), color, p, terms, (widths[c] - pad * 2.0).max(20.0));
                ui.fonts(|f| f.layout_job(job))
            })
            .collect();
        let h = galleys.iter().map(|g| g.size().y).fold(0.0, f32::max) + 10.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::new(avail, h), Sense::hover());
        let fill = if header {
            tint(accent, if p.is_light { 40 } else { 30 })
        } else if ri % 2 == 0 {
            tint(p.control, 110)
        } else {
            Color32::TRANSPARENT
        };
        if fill != Color32::TRANSPARENT {
            ui.painter().rect_filled(rect, Rounding::ZERO, fill);
        }
        let mut x = rect.min.x;
        for (c, g) in galleys.into_iter().enumerate() {
            ui.painter().galley(egui::pos2(x + pad, rect.min.y + 5.0), g, color);
            x += widths[c];
        }
        ui.spacing_mut().item_spacing.y = 0.0;
    }
    let end_y = ui.cursor().min.y;
    let outline = Rect::from_min_max(egui::pos2(left, start_y), egui::pos2(left + avail, end_y));
    ui.painter().add(Shape::rect_stroke(outline, Rounding::same(4.0), Stroke::new(1.0, tint(p.line, 150))));
    ui.spacing_mut().item_spacing.y = 4.0;
    ui.add_space(8.0);
}

// ───────────────────────────── inline styles ─────────────────────────────

#[derive(Clone, Copy, Default)]
struct Inline {
    bold: bool,
    code: bool,
    em: bool,
}

/// Parse `**bold**`, `` `code` `` and `*em*` into a wrapped [`LayoutJob`], highlighting
/// search terms with a soft background.
fn inline_job(
    text: &str,
    font: FontId,
    color: Color32,
    p: &ThemePalette,
    terms: &[String],
    wrap_w: f32,
) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap_w;
    let mut style = Inline::default();
    let mut buf = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '`' {
            push_span(&mut job, &buf, style, &font, color, p, terms);
            buf.clear();
            style.code = !style.code;
        } else if ch == '*' && !style.code {
            if chars.peek() == Some(&'*') {
                chars.next();
                push_span(&mut job, &buf, style, &font, color, p, terms);
                buf.clear();
                style.bold = !style.bold;
            } else if style.em || chars.peek().is_some_and(|c| !c.is_whitespace()) {
                push_span(&mut job, &buf, style, &font, color, p, terms);
                buf.clear();
                style.em = !style.em;
            } else {
                buf.push(ch);
            }
        } else {
            buf.push(ch);
        }
    }
    push_span(&mut job, &buf, style, &font, color, p, terms);
    job
}

fn push_span(
    job: &mut LayoutJob,
    text: &str,
    style: Inline,
    base: &FontId,
    color: Color32,
    p: &ThemePalette,
    terms: &[String],
) {
    if text.is_empty() {
        return;
    }
    let mut fmt = TextFormat { font_id: base.clone(), color, ..Default::default() };
    if style.code {
        fmt.font_id = font_mono(base.size * 0.93);
        fmt.color = if p.is_light { mix(p.accent, Color32::BLACK, 0.25) } else { mix(p.accent, p.text, 0.35) };
        fmt.background = tint(p.control, 220);
    } else if style.bold {
        fmt.font_id = font_semi(base.size);
        fmt.color = if color == p.muted { p.text } else { color };
    } else if style.em {
        fmt.italics = true;
        fmt.color = mix(color, p.muted, 0.3);
    }

    if terms.is_empty() {
        job.append(text, 0.0, fmt);
        return;
    }
    // Split on case-insensitive term matches (ASCII lower-casing keeps byte offsets intact).
    let lower = text.to_ascii_lowercase();
    let mut marks: Vec<(usize, usize)> = Vec::new();
    for t in terms {
        let mut from = 0;
        while let Some(at) = lower[from..].find(t.as_str()) {
            let s = from + at;
            marks.push((s, s + t.len()));
            from = s + t.len();
        }
    }
    if marks.is_empty() {
        job.append(text, 0.0, fmt);
        return;
    }
    marks.sort();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for m in marks {
        match merged.last_mut() {
            Some(last) if m.0 <= last.1 => last.1 = last.1.max(m.1),
            _ => merged.push(m),
        }
    }
    let hl_bg = tint(p.warn, if p.is_light { 90 } else { 70 });
    let mut pos = 0;
    for (s, e) in merged {
        if !text.is_char_boundary(s) || !text.is_char_boundary(e) {
            continue;
        }
        if s > pos {
            job.append(&text[pos..s], 0.0, fmt.clone());
        }
        let mut h = fmt.clone();
        h.background = hl_bg;
        job.append(&text[s..e], 0.0, h);
        pos = e;
    }
    if pos < text.len() {
        job.append(&text[pos..], 0.0, fmt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_ranks_title_matches_first() {
        let hits = search("modbus byte order", None);
        assert_eq!(hits.first().map(|h| h.article.id), Some("modbus-types"));
    }

    #[test]
    fn tag_filter_requires_tag() {
        let hits = search("#oilfield", None);
        assert!(!hits.is_empty());
        assert!(hits.iter().all(|h| h.article.tags.iter().any(|t| t.contains("oilfield"))));
    }

    #[test]
    fn category_filter_limits_results() {
        let hits = search("", Some(Category::Protocols));
        assert!(hits.iter().all(|h| h.article.category == Category::Protocols));
    }

    #[test]
    fn snippet_is_char_safe() {
        let s = snippet("°C ↔ °F conversion and psi ↔ kPa", "psi").unwrap();
        assert!(s.contains("psi"));
    }
}
