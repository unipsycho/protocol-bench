//! Network layer. In the browser (wasm) this is the WebSocket feed plus `fetch` for layouts and
//! history. On a desktop build there is no server: [`crate::demo`] plays that part.

#[cfg(target_arch = "wasm32")]
use crate::state::Cmd;
use crate::state::Shared;
use std::cell::RefCell;
use std::rc::Rc;

#[cfg(target_arch = "wasm32")]
mod web {
    use super::*;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{CloseEvent, MessageEvent, RequestInit, Response, WebSocket};

    pub fn location() -> Option<web_sys::Location> {
        web_sys::window().map(|w| w.location())
    }

    pub fn is_kiosk() -> bool {
        location().and_then(|l| l.search().ok()).map(|s| s.contains("kiosk")).unwrap_or(false)
    }

    /// Dashboard name from `#/Name`.
    pub fn initial_name() -> Option<String> {
        let h = location()?.hash().ok()?;
        let n = h.trim_start_matches('#').trim_start_matches('/');
        if n.is_empty() {
            return None;
        }
        js_sys::decode_uri_component(n).ok().and_then(|s| s.as_string())
    }

    pub fn set_hash(name: &str) {
        if name.is_empty() {
            return;
        }
        if let Some(l) = location() {
            let enc: String = js_sys::encode_uri_component(name).into();
            let _ = l.set_hash(&format!("/{enc}"));
        }
    }

    pub fn connect_ws(shared: Rc<RefCell<Shared>>, ctx: egui::Context) {
        let Some(loc) = location() else { return };
        let proto = if loc.protocol().ok().as_deref() == Some("https:") { "wss" } else { "ws" };
        let host = loc.host().unwrap_or_default();
        open_ws(format!("{proto}://{host}/ws"), shared, ctx);
    }

    fn open_ws(url: String, shared: Rc<RefCell<Shared>>, ctx: egui::Context) {
        let Ok(ws) = WebSocket::new(&url) else {
            shared.borrow_mut().last_error = format!("cannot open {url}");
            return;
        };
        {
            let (shared, ctx) = (shared.clone(), ctx.clone());
            let f = Closure::<dyn FnMut()>::new(move || {
                let mut s = shared.borrow_mut();
                s.connected = true;
                s.last_error.clear();
                ctx.request_repaint();
            });
            ws.set_onopen(Some(f.as_ref().unchecked_ref()));
            f.forget();
        }
        {
            let (shared, ctx) = (shared.clone(), ctx.clone());
            let f = Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
                if let Some(t) = e.data().as_string() {
                    shared.borrow_mut().apply(&t);
                    ctx.request_repaint();
                }
            });
            ws.set_onmessage(Some(f.as_ref().unchecked_ref()));
            f.forget();
        }
        {
            let f = Closure::<dyn FnMut(CloseEvent)>::new(move |_| {
                shared.borrow_mut().connected = false;
                ctx.request_repaint();
                let (shared, ctx, url) = (shared.clone(), ctx.clone(), url.clone());
                let retry = Closure::once_into_js(move || open_ws(url, shared, ctx));
                if let Some(w) = web_sys::window() {
                    let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(retry.unchecked_ref(), 1500);
                }
            });
            ws.set_onclose(Some(f.as_ref().unchecked_ref()));
            f.forget();
        }
    }

    async fn fetch(method: &str, path: &str, body: Option<String>) -> Result<(u16, String), String> {
        let win = web_sys::window().ok_or("no window")?;
        let init = RequestInit::new();
        init.set_method(method);
        if let Some(b) = &body {
            init.set_body(&JsValue::from_str(b));
        }
        let req = web_sys::Request::new_with_str_and_init(path, &init).map_err(|e| format!("{e:?}"))?;
        let resp: Response = JsFuture::from(win.fetch_with_request(&req)).await.map_err(|e| format!("{e:?}"))?.dyn_into().map_err(|_| "bad response")?;
        let text = JsFuture::from(resp.text().map_err(|e| format!("{e:?}"))?).await.map_err(|e| format!("{e:?}"))?;
        Ok((resp.status(), text.as_string().unwrap_or_default()))
    }

    fn enc(s: &str) -> String {
        js_sys::encode_uri_component(s).into()
    }

    /// Run queued commands (one async task each).
    pub fn pump(shared: &Rc<RefCell<Shared>>, ctx: &egui::Context, cmds: Vec<Cmd>) {
        for cmd in cmds {
            let (shared, ctx) = (shared.clone(), ctx.clone());
            wasm_bindgen_futures::spawn_local(async move {
                run(cmd, &shared).await;
                ctx.request_repaint();
            });
        }
    }

    async fn run(cmd: Cmd, shared: &Rc<RefCell<Shared>>) {
        match cmd {
            Cmd::List => match fetch("GET", "/api/dashboards", None).await {
                Ok((200, text)) => {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                        let names = v["names"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
                        let title = v["title"].as_str().unwrap_or("").to_string();
                        shared.borrow_mut().on_list(names, title, v["can_edit"].as_bool().unwrap_or(true));
                    }
                }
                Ok((s, _)) => shared.borrow_mut().last_error = format!("dashboards: HTTP {s}"),
                Err(e) => shared.borrow_mut().last_error = format!("dashboards: {e}"),
            },
            Cmd::Load(name) => match fetch("GET", &format!("/api/dashboards/{}", enc(&name)), None).await {
                Ok((200, text)) => shared.borrow_mut().on_layout(&name, Some(&text)),
                Ok((404, _)) => shared.borrow_mut().on_layout(&name, None),
                Ok((s, _)) => shared.borrow_mut().last_error = format!("load '{name}': HTTP {s}"),
                Err(e) => shared.borrow_mut().last_error = format!("load '{name}': {e}"),
            },
            Cmd::Save(name, json) => match fetch("PUT", &format!("/api/dashboards/{}", enc(&name)), Some(json)).await {
                Ok((200, _)) => {}
                Ok((403, _)) => shared.borrow_mut().last_error = "This dashboard is read-only".into(),
                Ok((s, t)) => shared.borrow_mut().last_error = format!("save '{name}': HTTP {s} {t}"),
                Err(e) => shared.borrow_mut().last_error = format!("save '{name}': {e}"),
            },
            Cmd::Delete(name) => {
                let _ = fetch("DELETE", &format!("/api/dashboards/{}", enc(&name)), None).await;
            }
            Cmd::History(points, since) => {
                let q = points.iter().map(|p| enc(p)).collect::<Vec<_>>().join(",");
                if let Ok((200, text)) = fetch("GET", &format!("/api/history?points={q}&since={since}&max=1500"), None).await {
                    if let Ok(serde_json::Value::Object(m)) = serde_json::from_str::<serde_json::Value>(&text) {
                        let mut s = shared.borrow_mut();
                        for (p, rows) in m {
                            let rows: Vec<(u64, f64)> = rows.as_array().map(|a| a.iter().filter_map(|r| Some((r[0].as_u64()?, r[1].as_f64()?))).collect()).unwrap_or_default();
                            s.on_history(&p, rows);
                        }
                    }
                }
            }
        }
    }
}

/// Start the live feed and ask for the dashboard list (wasm); nothing to do natively.
pub fn start(shared: &Rc<RefCell<Shared>>, ctx: &egui::Context) {
    #[cfg(target_arch = "wasm32")]
    {
        shared.borrow_mut().outbox.push(Cmd::List);
        web::connect_ws(shared.clone(), ctx.clone());
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (shared, ctx);
}

/// Execute whatever the UI queued this frame (wasm); the desktop demo has its own backend.
pub fn pump(shared: &Rc<RefCell<Shared>>, ctx: &egui::Context, sh: &mut Shared) {
    let cmds = std::mem::take(&mut sh.outbox);
    #[cfg(target_arch = "wasm32")]
    web::pump(shared, ctx, cmds);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (shared, ctx, cmds);
}

pub fn set_hash(name: &str) {
    #[cfg(target_arch = "wasm32")]
    web::set_hash(name);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = name;
}

pub fn is_kiosk() -> bool {
    #[cfg(target_arch = "wasm32")]
    return web::is_kiosk();
    #[cfg(not(target_arch = "wasm32"))]
    false
}

pub fn initial_name() -> Option<String> {
    #[cfg(target_arch = "wasm32")]
    return web::initial_name();
    #[cfg(not(target_arch = "wasm32"))]
    None
}

/// Local wall-clock time `HH:MM` (browser time zone; UTC on the desktop demo).
pub fn clock_hm() -> String {
    #[cfg(target_arch = "wasm32")]
    {
        let d = js_sys::Date::new_0();
        format!("{:02}:{:02}", d.get_hours(), d.get_minutes())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        format!("{:02}:{:02}", (s / 3600) % 24, (s / 60) % 60)
    }
}
