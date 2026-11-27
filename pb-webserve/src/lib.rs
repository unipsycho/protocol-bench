//! Tiny blocking HTTP/1.1 helper for packs whose server port also speaks WebSocket.
//!
//! [`classify`] looks at the start of a fresh connection *without consuming it*: a WebSocket
//! upgrade is handed back untouched (for `tungstenite::accept`), anything else is parsed into a
//! [`Request`] that the pack routes and answers with [`respond`]. Static files are served from a
//! pack's embedded [`Asset`] table. Deliberately small: GET / PUT / POST / DELETE, a
//! `Content-Length` body up to [`MAX_BODY`], one request per connection (`Connection: close`).
//!
//! Lives in a plain rlib because two packs can't link into one binary.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

/// Largest request body accepted (layout JSON is a few KB; this is generous).
pub const MAX_BODY: usize = 1 << 20;
const MAX_HEAD: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    /// Path without the query string, percent-decoded.
    pub path: String,
    /// Raw query string (after `?`), not decoded.
    pub query: String,
    /// Header names lower-cased.
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(&name.to_ascii_lowercase()).map(String::as_str)
    }

    /// Decoded value of one query parameter (`?a=1&b=x%20y`).
    pub fn query_param(&self, key: &str) -> Option<String> {
        self.query.split('&').filter_map(|kv| kv.split_once('=').or(Some((kv, "")))).find(|(k, _)| *k == key).map(|(_, v)| percent_decode(v))
    }

    /// Value of one cookie from the `Cookie` header.
    pub fn cookie(&self, name: &str) -> Option<String> {
        self.header("cookie")?.split(';').filter_map(|p| p.trim().split_once('=')).find(|(k, _)| *k == name).map(|(_, v)| v.to_string())
    }
}

#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
    pub headers: Vec<(String, String)>,
}

impl Response {
    pub fn new(status: u16, content_type: &'static str, body: impl Into<Vec<u8>>) -> Self {
        Self { status, content_type, body: body.into(), headers: Vec::new() }
    }
    pub fn html(body: impl Into<Vec<u8>>) -> Self {
        Self::new(200, "text/html; charset=utf-8", body)
    }
    pub fn json(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self::new(status, "application/json", body)
    }
    pub fn text(status: u16, body: &str) -> Self {
        Self::new(status, "text/plain; charset=utf-8", body.as_bytes().to_vec())
    }
    pub fn not_found() -> Self {
        Self::text(404, "Not found")
    }
    pub fn method_not_allowed() -> Self {
        Self::text(405, "Method not allowed")
    }
    pub fn bad_request(why: &str) -> Self {
        Self::text(400, why)
    }
    pub fn with_header(mut self, k: &str, v: &str) -> Self {
        self.headers.push((k.to_string(), v.to_string()));
        self
    }
}

pub enum Incoming {
    /// A WebSocket upgrade; the stream is untouched, pass it to `tungstenite::accept`.
    WebSocket(TcpStream),
    /// A plain HTTP request, fully read. Answer with [`respond`].
    Http(Request, TcpStream),
    /// Closed, malformed or timed out; nothing to do.
    Dropped,
}

/// Decide what a fresh connection is. Gives up after `wait` (the peer is too slow or silent).
pub fn classify(stream: TcpStream, wait: Duration) -> Incoming {
    let mut buf = vec![0u8; MAX_HEAD];
    let end = Instant::now() + wait;
    // Peeks must not block forever on a silent peer.
    let _ = stream.set_read_timeout(Some(Duration::from_millis(50)));
    let head_len = loop {
        match stream.peek(&mut buf) {
            Ok(0) => return Incoming::Dropped,
            Ok(n) => {
                if let Some(i) = find_head_end(&buf[..n]) {
                    let head = String::from_utf8_lossy(&buf[..i]).to_ascii_lowercase();
                    if head.lines().any(|l| l.replace(' ', "").starts_with("upgrade:websocket")) {
                        let _ = stream.set_read_timeout(Some(wait)); // the handshake needs patience
                        return Incoming::WebSocket(stream);
                    }
                    break i + 4;
                }
                if n == buf.len() {
                    return Incoming::Dropped; // head too large
                }
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(_) => return Incoming::Dropped,
        }
        if Instant::now() >= end {
            return Incoming::Dropped;
        }
        std::thread::sleep(Duration::from_millis(3));
    };
    // Plain HTTP: consume the head, then the body (bounded by the caller's patience).
    let mut stream = stream;
    let _ = stream.set_read_timeout(Some(wait));
    let mut head = vec![0u8; head_len];
    if stream.read_exact(&mut head).is_err() {
        return Incoming::Dropped;
    }
    let Some(mut req) = parse_head(&head[..head_len - 4]) else {
        let _ = respond(&mut stream, Response::bad_request("Bad request"));
        return Incoming::Dropped;
    };
    let len = req.header("content-length").and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(0);
    if len > MAX_BODY {
        let _ = respond(&mut stream, Response::text(413, "Body too large"));
        return Incoming::Dropped;
    }
    if len > 0 {
        let mut body = vec![0u8; len];
        if stream.read_exact(&mut body).is_err() {
            return Incoming::Dropped;
        }
        req.body = body;
    }
    Incoming::Http(req, stream)
}

fn find_head_end(b: &[u8]) -> Option<usize> {
    b.windows(4).position(|w| w == b"\r\n\r\n")
}

fn parse_head(head: &[u8]) -> Option<Request> {
    let text = std::str::from_utf8(head).ok()?;
    let mut lines = text.split("\r\n");
    let mut first = lines.next()?.split_whitespace();
    let method = first.next()?.to_ascii_uppercase();
    let target = first.next()?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let mut headers = HashMap::new();
    for l in lines {
        if let Some((k, v)) = l.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    Some(Request { method, path: percent_decode(path), query: query.to_string(), headers, body: Vec::new() })
}

pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    out.push(v);
                    i += 3;
                    continue;
                }
                out.push(b'%');
            }
            b'+' => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        302 => "Found",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        _ => "OK",
    }
}

/// Write a response and flush. The caller drops the stream afterwards (`Connection: close`).
pub fn respond(stream: &mut TcpStream, r: Response) -> std::io::Result<()> {
    let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n",
        r.status,
        reason(r.status),
        r.content_type,
        r.body.len()
    );
    for (k, v) in &r.headers {
        // Header values come from pack code, never from the peer; still refuse line breaks.
        if !k.contains(['\r', '\n']) && !v.contains(['\r', '\n']) {
            head.push_str(&format!("{k}: {v}\r\n"));
        }
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(&r.body)?;
    stream.flush()
}

/// One embedded file: URL path (`/`, `/app.js`), media type, bytes.
#[derive(Clone, Copy)]
pub struct Asset {
    pub path: &'static str,
    pub content_type: &'static str,
    pub bytes: &'static [u8],
}

/// Serve `path` from an embedded table; `/` falls back to `/index.html`.
pub fn serve_asset(assets: &[Asset], path: &str) -> Response {
    let path = if path == "/" { "/index.html" } else { path };
    match assets.iter().find(|a| a.path == path) {
        Some(a) => Response::new(200, a.content_type, a.bytes.to_vec()),
        None => Response::not_found(),
    }
}

/// Media type from a file extension, for tables built from a directory.
pub fn content_type_for(name: &str) -> &'static str {
    match name.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "wasm" => "application/wasm",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn pair() -> (TcpStream, TcpStream) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let c = TcpStream::connect(l.local_addr().unwrap()).unwrap();
        let (s, _) = l.accept().unwrap();
        (c, s)
    }

    #[test]
    fn plain_get_is_parsed() {
        let (mut c, s) = pair();
        c.write_all(b"GET /api/x%20y?a=1&b=p%2Fq HTTP/1.1\r\nHost: h\r\nCookie: s=abc; t=1\r\n\r\n").unwrap();
        let Incoming::Http(r, _) = classify(s, Duration::from_secs(2)) else { panic!("not http") };
        assert_eq!((r.method.as_str(), r.path.as_str()), ("GET", "/api/x y"));
        assert_eq!(r.query_param("a").as_deref(), Some("1"));
        assert_eq!(r.query_param("b").as_deref(), Some("p/q"));
        assert_eq!(r.cookie("s").as_deref(), Some("abc"));
        assert!(r.body.is_empty());
    }

    #[test]
    fn put_body_is_read_even_when_it_arrives_late() {
        let (mut c, s) = pair();
        c.write_all(b"PUT /api/d HTTP/1.1\r\nContent-Length: 5\r\n\r\nhe").unwrap();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(60));
            c.write_all(b"llo").unwrap();
            c
        });
        let Incoming::Http(r, _) = classify(s, Duration::from_secs(2)) else { panic!("not http") };
        assert_eq!(r.body, b"hello");
        drop(t.join());
    }

    #[test]
    fn upgrade_is_left_unconsumed() {
        let (mut c, s) = pair();
        c.write_all(b"GET /ws HTTP/1.1\r\nHost: h\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n").unwrap();
        let Incoming::WebSocket(mut s) = classify(s, Duration::from_secs(2)) else { panic!("not ws") };
        let mut b = [0u8; 8];
        s.read_exact(&mut b).unwrap();
        assert_eq!(&b, b"GET /ws ");
    }

    #[test]
    fn oversized_body_is_refused_and_silent_peer_times_out() {
        let (mut c, s) = pair();
        c.write_all(format!("POST / HTTP/1.1\r\nContent-Length: {}\r\n\r\n", MAX_BODY + 1).as_bytes()).unwrap();
        assert!(matches!(classify(s, Duration::from_secs(2)), Incoming::Dropped));
        let mut out = String::new();
        c.read_to_string(&mut out).unwrap();
        assert!(out.starts_with("HTTP/1.1 413"));
        let (_c2, s2) = pair();
        assert!(matches!(classify(s2, Duration::from_millis(80)), Incoming::Dropped));
    }

    #[test]
    fn respond_and_assets() {
        let (mut c, mut s) = pair();
        let assets = [Asset { path: "/index.html", content_type: "text/html; charset=utf-8", bytes: b"<b>hi</b>" }];
        respond(&mut s, serve_asset(&assets, "/").with_header("X-A", "1")).unwrap();
        drop(s);
        let mut out = String::new();
        c.read_to_string(&mut out).unwrap();
        assert!(out.starts_with("HTTP/1.1 200 OK") && out.contains("X-A: 1") && out.ends_with("<b>hi</b>"), "{out}");
        assert_eq!(serve_asset(&assets, "/nope").status, 404);
    }

    #[test]
    fn percent_decoding_is_total() {
        assert_eq!(percent_decode("a%2"), "a%2");
        assert_eq!(percent_decode("%zz+x"), "%zz x");
        assert_eq!(percent_decode("%e2%82%ac"), "€");
    }
}
