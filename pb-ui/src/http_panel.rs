//! HTTP / REST panel — outbound Clients only (locked PB HTTP plan).
//! Hub Input/Output nodes own poll/push binds; this panel owns endpoints.

use egui::{Layout, RichText, Sense, UiBuilder};

use crate::connection_list::{self, text_btn, RowAction};
use crate::theme::{apply_form_spacing, font_semi, font_ui, form_inset, space, ThemePalette};

const LABEL_W: f32 = 100.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpAuthKind {
    None,
    Bearer,
    Basic,
}

impl HttpAuthKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Bearer => "Bearer",
            Self::Basic => "Basic",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "bearer" => Self::Bearer,
            "basic" => Self::Basic,
            _ => Self::None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Bearer => "bearer",
            Self::Basic => "basic",
        }
    }
}

#[derive(Debug, Clone)]
pub struct HttpHeader {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct HttpClient {
    pub id: String,
    pub label: String,
    pub base_url: String,
    pub auth: HttpAuthKind,
    pub token: String,
    pub username: String,
    pub password: String,
    pub timeout_secs: String,
    pub headers: Vec<HttpHeader>,
    pub draft_header_name: String,
    pub draft_header_value: String,
}

impl HttpClient {
    pub fn default_local() -> Self {
        Self {
            id: "api".into(),
            label: "Local API".into(),
            base_url: "http://127.0.0.1:8080".into(),
            auth: HttpAuthKind::None,
            token: String::new(),
            username: String::new(),
            password: String::new(),
            timeout_secs: "10".into(),
            headers: vec![HttpHeader {
                name: "Content-Type".into(),
                value: "application/json".into(),
            }],
            draft_header_name: String::new(),
            draft_header_value: String::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct HttpPanelState {
    pub clients: Vec<HttpClient>,
    pub selected: Option<usize>,
    pub pending_delete: Option<(usize, String)>,
}

impl Default for HttpPanelState {
    fn default() -> Self {
        Self {
            clients: vec![HttpClient::default_local()],
            selected: None,
            pending_delete: None,
        }
    }
}

impl HttpPanelState {
    pub fn selected(&self) -> Option<&HttpClient> {
        self.selected.and_then(|i| self.clients.get(i))
    }

    pub fn selected_mut(&mut self) -> Option<&mut HttpClient> {
        self.selected.and_then(|i| self.clients.get_mut(i))
    }

    pub fn connection_choices(&self) -> Vec<(String, String)> {
        self.clients
            .iter()
            .map(|c| {
                let label = if c.label.trim().is_empty() {
                    c.id.clone()
                } else {
                    c.label.clone()
                };
                (c.id.clone(), label)
            })
            .collect()
    }

    pub fn client_by_id(&self, id: &str) -> Option<&HttpClient> {
        let id = id.trim();
        if id.is_empty() {
            return None;
        }
        self.clients.iter().find(|c| c.id == id)
    }

    fn ensure_clients(&mut self) {
        if self.clients.is_empty() {
            self.clients.push(HttpClient::default_local());
            self.selected = None;
        } else if let Some(i) = self.selected {
            if i >= self.clients.len() {
                self.selected = None;
            }
        }
    }

    fn unique_id(&self, base: &str) -> String {
        connection_list::unique_id(base, |id| self.clients.iter().any(|c| c.id == id))
    }
}

/// Join client base URL with an Input path (`/v2/...` or query-only).
pub fn join_url(base: &str, path: &str) -> String {
    let base = base.trim().trim_end_matches('/');
    let path = path.trim();
    if path.is_empty() {
        return base.to_string();
    }
    if path.starts_with("http://") || path.starts_with("https://") {
        return path.to_string();
    }
    if path.starts_with('/') {
        format!("{base}{path}")
    } else {
        format!("{base}/{path}")
    }
}

/// One-shot GET/POST preview for the Hub Input sample pane (UI only; pack polls later).
pub fn fetch_preview(client: &HttpClient, method: &str, path: &str) -> Result<String, String> {
    let url = join_url(&client.base_url, path);
    if url.is_empty() || client.base_url.trim().is_empty() {
        return Err("Pick an HTTP client with a Base URL.".into());
    }
    let timeout = client
        .timeout_secs
        .parse::<u64>()
        .ok()
        .filter(|n| *n > 0)
        .unwrap_or(10);
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(timeout))
        .build();
    let method = method.trim().to_ascii_uppercase();
    let mut req = match method.as_str() {
        "POST" => agent.request("POST", &url),
        _ => agent.get(&url),
    };
    match client.auth {
        HttpAuthKind::None => {}
        HttpAuthKind::Bearer => {
            let token = client.token.trim();
            if !token.is_empty() {
                req = req.set("Authorization", &format!("Bearer {token}"));
            }
        }
        HttpAuthKind::Basic => {
            let raw = format!("{}:{}", client.username, client.password);
            let b64 = basic_auth_b64(&raw);
            req = req.set("Authorization", &format!("Basic {b64}"));
        }
    }
    for h in &client.headers {
        let name = h.name.trim();
        if name.is_empty() {
            continue;
        }
        // Don't override Authorization we just set.
        if name.eq_ignore_ascii_case("authorization") {
            continue;
        }
        req = req.set(name, &h.value);
    }
    let resp = req.call().map_err(|e| format!("Request failed: {e}"))?;
    let status = resp.status();
    let body = resp
        .into_string()
        .map_err(|e| format!("Read body failed: {e}"))?;
    if !(200..300).contains(&status) {
        let snip = body.chars().take(180).collect::<String>();
        return Err(format!("HTTP {status}: {snip}"));
    }
    // Pretty-print when JSON; otherwise keep raw text for diagnosis.
    match serde_json::from_str::<serde_json::Value>(&body) {
        Ok(v) => serde_json::to_string_pretty(&v).map_err(|e| e.to_string()),
        Err(_) => Ok(body),
    }
}

fn basic_auth_b64(raw: &str) -> String {
    // Minimal base64 (no extra crate) for Basic auth previews.
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = raw.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        let b0 = bytes[i] as u32;
        let b1 = if i + 1 < bytes.len() {
            bytes[i + 1] as u32
        } else {
            0
        };
        let b2 = if i + 2 < bytes.len() {
            bytes[i + 2] as u32
        } else {
            0
        };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((triple >> 18) & 63) as usize] as char);
        out.push(TABLE[((triple >> 12) & 63) as usize] as char);
        if i + 1 < bytes.len() {
            out.push(TABLE[((triple >> 6) & 63) as usize] as char);
        } else {
            out.push('=');
        }
        if i + 2 < bytes.len() {
            out.push(TABLE[(triple & 63) as usize] as char);
        } else {
            out.push('=');
        }
        i += 3;
    }
    out
}

/// Flatten JSON object/array keys into dotted paths (arrays use `[]` then `.field`).
pub fn json_leaf_paths(value: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    walk_paths(value, String::new(), &mut out);
    out
}

fn walk_paths(value: &serde_json::Value, prefix: String, out: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            if map.is_empty() && !prefix.is_empty() {
                out.push(prefix);
                return;
            }
            for (k, v) in map {
                let next = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                walk_paths(v, next, out);
            }
        }
        serde_json::Value::Array(arr) => {
            if arr.is_empty() {
                if !prefix.is_empty() {
                    out.push(prefix);
                }
                return;
            }
            // Use first element as shape sample.
            let next = if prefix.is_empty() {
                "[]".into()
            } else {
                format!("{prefix}[]")
            };
            walk_paths(&arr[0], next, out);
        }
        _ => {
            if !prefix.is_empty() {
                out.push(prefix);
            }
        }
    }
}

pub fn paint(ui: &mut egui::Ui, p: &ThemePalette, state: &mut HttpPanelState) -> bool {
    state.ensure_clients();
    let mut dirty = false;
    let full = ui.available_rect_before_wrap();
    let inner = form_inset(full);
    ui.allocate_new_ui(
        UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::top_down(egui::Align::Min)),
        |ui| {
            dirty = paint_body(ui, p, state);
        },
    );
    ui.allocate_rect(full, Sense::hover());
    dirty
}

fn paint_body(ui: &mut egui::Ui, p: &ThemePalette, state: &mut HttpPanelState) -> bool {
    apply_form_spacing(ui);
    let mut dirty = false;
    ui.label(
        RichText::new("Outbound JSON HTTP clients for hub Inputs (poll) and Outputs (push).")
            .font(font_ui(12.5))
            .color(p.muted),
    );
    ui.add_space(space::AFTER_BLURB);

    if connection_list::heading_with_add(ui, "Clients", "+ Add", p.live, p) {
        let id = state.unique_id("api");
        let mut c = HttpClient::default_local();
        c.id = id.clone();
        c.label = id;
        state.clients.push(c);
        state.selected = Some(state.clients.len() - 1);
        dirty = true;
    }
    ui.add_space(space::AFTER_HEADING);
    ui.label(
        RichText::new("Endpoints only — bind JSON paths on hub Input / Output nodes.")
            .font(font_ui(12.0))
            .color(p.muted),
    );
    ui.add_space(space::AFTER_BLURB);

    let mut select = None;
    let show_delete = state.clients.len() > 1;
    for (i, client) in state.clients.iter().enumerate() {
        let selected = state.selected == Some(i);
        let label = if client.label.is_empty() {
            client.id.clone()
        } else {
            client.label.clone()
        };
        let subtitle = client.base_url.clone();
        match connection_list::selectable_row(ui, &label, &subtitle, selected, show_delete, p) {
            Some(RowAction::Select) => select = Some(i),
            Some(RowAction::Delete) => {
                state.pending_delete = Some((i, label));
            }
            None => {}
        }
        ui.add_space(2.0);
    }
    if let Some(i) = select {
        state.selected = if state.selected == Some(i) {
            None
        } else {
            Some(i)
        };
    }
    if let Some(i) = connection_list::confirm_delete(ui.ctx(), &mut state.pending_delete, p) {
        if i < state.clients.len() {
            let was = state.selected;
            state.clients.remove(i);
            state.selected = match was {
                Some(s) if s == i => None,
                Some(s) if s > i => Some(s - 1),
                other => other,
            };
            state.ensure_clients();
            dirty = true;
        }
    }

    ui.add_space(space::AFTER_FIELD + 2.0);
    if state.selected().is_none() {
        ui.label(
            RichText::new("Select a client to edit.")
                .font(font_ui(12.5))
                .color(p.muted),
        );
        return dirty;
    }

    egui::ScrollArea::vertical()
        .id_salt("pb_http_cfg")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            dirty |= edit_selected(ui, state, p);
        });
    dirty
}

fn edit_selected(ui: &mut egui::Ui, state: &mut HttpPanelState, p: &ThemePalette) -> bool {
    let mut dirty = false;
    ui.label(
        RichText::new("CLIENT")
            .font(font_ui(10.5))
            .color(p.muted),
    );
    ui.add_space(space::AFTER_HEADING);
    {
        let c = state.selected_mut().unwrap();
        dirty |= field(ui, "Id", &mut c.id, p);
        dirty |= field(ui, "Label", &mut c.label, p);
        dirty |= field(ui, "Base URL", &mut c.base_url, p);
        dirty |= field(ui, "Timeout (s)", &mut c.timeout_secs, p);

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = space::ITEM_X;
            ui.add_sized(
                [LABEL_W, 20.0],
                egui::Label::new(
                    RichText::new("Auth")
                        .font(font_ui(12.0))
                        .color(p.muted),
                ),
            );
            let mut auth = c.auth;
            egui::ComboBox::from_id_salt("pb_http_auth")
                .selected_text(auth.label())
                .width(120.0)
                .show_ui(ui, |ui| {
                    for kind in [HttpAuthKind::None, HttpAuthKind::Bearer, HttpAuthKind::Basic] {
                        ui.selectable_value(&mut auth, kind, kind.label());
                    }
                });
            if auth != c.auth {
                c.auth = auth;
                dirty = true;
            }
        });
        match c.auth {
            HttpAuthKind::None => {}
            HttpAuthKind::Bearer => {
                dirty |= field(ui, "Token", &mut c.token, p);
            }
            HttpAuthKind::Basic => {
                dirty |= field(ui, "Username", &mut c.username, p);
                dirty |= password(ui, "Password", &mut c.password, p);
            }
        }
    }

    ui.add_space(space::SECTION);
    ui.label(
        RichText::new("DEFAULT HEADERS")
            .font(font_ui(10.5))
            .color(p.muted),
    );
    ui.add_space(space::AFTER_HEADING);
    let mut remove_h = None;
    let mut add_h = false;
    {
        let c = state.selected_mut().unwrap();
        for (i, h) in c.headers.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("{}: {}", h.name, h.value))
                        .font(font_ui(12.0))
                        .color(p.text),
                );
                if text_btn(ui, "Remove", false, p).clicked() {
                    remove_h = Some(i);
                }
            });
        }
        dirty |= field(ui, "Name", &mut c.draft_header_name, p);
        dirty |= field(ui, "Value", &mut c.draft_header_value, p);
        if text_btn(ui, "Add header", false, p).clicked() {
            add_h = true;
        }
    }
    if let Some(i) = remove_h {
        state.selected_mut().unwrap().headers.remove(i);
        dirty = true;
    }
    if add_h {
        let c = state.selected_mut().unwrap();
        let name = c.draft_header_name.trim().to_string();
        let value = c.draft_header_value.trim().to_string();
        if !name.is_empty() {
            c.headers.push(HttpHeader { name, value });
            c.draft_header_name.clear();
            c.draft_header_value.clear();
            dirty = true;
        }
    }

    ui.add_space(space::AFTER_FIELD);
    ui.label(
        RichText::new("Tokens / passwords stored in project JSON (no keyring yet).")
            .font(font_ui(11.0))
            .color(p.muted),
    );
    dirty
}

fn field(ui: &mut egui::Ui, label: &str, value: &mut String, p: &ThemePalette) -> bool {
    connection_list::labeled_field(ui, LABEL_W, label, value, p)
}

fn password(ui: &mut egui::Ui, label: &str, value: &mut String, p: &ThemePalette) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::ITEM_X;
        ui.add_sized(
            [LABEL_W, 20.0],
            egui::Label::new(
                RichText::new(label)
                    .font(font_ui(12.0))
                    .color(p.muted),
            ),
        );
        let w = (ui.available_width() - 8.0).clamp(140.0, 420.0);
        if ui
            .add(
                egui::TextEdit::singleline(value)
                    .password(true)
                    .desired_width(w)
                    .margin(egui::Margin::symmetric(6.0, 3.0)),
            )
            .changed()
        {
            changed = true;
        }
    });
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_url_handles_slashes() {
        assert_eq!(
            join_url("https://api.coinbase.com", "/v2/prices/BTC-USD/spot"),
            "https://api.coinbase.com/v2/prices/BTC-USD/spot"
        );
        assert_eq!(
            join_url("https://api.coinbase.com/", "v2/prices/BTC-USD/spot"),
            "https://api.coinbase.com/v2/prices/BTC-USD/spot"
        );
    }

    #[test]
    fn leaf_paths_include_nested_amount() {
        let v: serde_json::Value =
            serde_json::from_str(r#"{"data":{"amount":"90000.12","currency":"USD"}}"#).unwrap();
        let paths = json_leaf_paths(&v);
        assert!(paths.iter().any(|p| p == "data.amount"));
        assert!(paths.iter().any(|p| p == "data.currency"));
    }
}

#[allow(dead_code)]
fn _font_semi(p: &ThemePalette) {
    let _ = font_semi(12.0);
    let _ = p;
}
