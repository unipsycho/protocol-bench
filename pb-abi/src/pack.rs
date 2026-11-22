//! Safe layer over the C ABI for pack authors.
//!
//! Every pack used to repeat ~150 lines of `extern "C"` shims (create / destroy / configure /
//! state_label / ...) and its own copy of the host-callback wrapper. Now a pack implements
//! [`Pack`] and calls [`export_pack!`]; this module owns all the `unsafe` pointer handling.
//!
//! ```ignore
//! impl pb_abi::Pack for PackState {
//!     const ID: &'static [u8] = b"snmp\0";
//!     const TITLE: &'static [u8] = b"SNMP\0";
//!     fn new(host: HostRef) -> Self { ... }
//!     fn configure(&mut self, json: &str) -> Result<(), String> { ... }
//!     ...
//! }
//! pb_abi::export_pack!(PackState);
//! ```
//!
//! Threading contract: the host calls a pack from one thread at a time, but packs run worker
//! threads that call back into the host. The host bridge serialises `upsert_point` behind a
//! mutex and a pack must join its workers before `destroy` returns — [`Pack::shutdown`] (and
//! `Drop`) is where that happens. That is why [`HostRef`] is `Send + Sync`.

use std::ffi::{c_char, c_int, c_void, CStr, CString};

use crate::{
    write_cstr, PbHostApi, PbPackApi, PB_ABI_VERSION, PB_VALUE_BOOL, PB_VALUE_F64, PB_VALUE_I64,
    PB_VALUE_STRING,
};

/// Copyable handle to the host's callbacks; safe to move into worker threads.
#[derive(Clone, Copy)]
pub struct HostRef(PbHostApi);

// SAFETY: see the module docs — the host serialises upserts and outlives every worker.
unsafe impl Send for HostRef {}
unsafe impl Sync for HostRef {}

/// Strip interior NULs so a stray `\0` never makes a C string (and so a log line) vanish.
fn cstring(text: &str) -> CString {
    CString::new(text.replace('\0', " ")).unwrap_or_default()
}

impl HostRef {
    pub fn new(api: PbHostApi) -> Self {
        Self(api)
    }

    /// Host log line (`PB_LOG_*` level). Silently dropped when the host gave no logger.
    pub fn log(&self, level: i32, msg: &str) {
        if let Some(log) = self.0.log_message {
            let c = cstring(msg);
            // SAFETY: callback and ctx come from the host and are valid for the pack's lifetime.
            unsafe { log(self.0.ctx, level, c.as_ptr()) };
        }
    }

    /// Upsert a typed point (`kind` = `PB_VALUE_*`, `bytes` in native endianness for numbers).
    pub fn upsert(&self, name: &str, source: &str, kind: i32, bytes: &[u8], quality: i32) {
        let Some(up) = self.0.upsert_point else { return };
        let (n, s) = (cstring(name), cstring(source));
        // SAFETY: see `log`; the pointers live until the call returns.
        unsafe { up(self.0.ctx, n.as_ptr(), kind, bytes.as_ptr() as *const c_void, bytes.len(), quality, s.as_ptr()) };
    }

    pub fn upsert_bool(&self, name: &str, source: &str, v: bool, quality: i32) {
        self.upsert(name, source, PB_VALUE_BOOL, &[v as u8], quality);
    }

    pub fn upsert_i64(&self, name: &str, source: &str, v: i64, quality: i32) {
        self.upsert(name, source, PB_VALUE_I64, &v.to_ne_bytes(), quality);
    }

    /// Unsigned values above `i64::MAX` fall back to a float rather than wrapping negative.
    pub fn upsert_u64(&self, name: &str, source: &str, v: u64, quality: i32) {
        match i64::try_from(v) {
            Ok(i) => self.upsert_i64(name, source, i, quality),
            Err(_) => self.upsert_f64(name, source, v as f64, quality),
        }
    }

    pub fn upsert_f64(&self, name: &str, source: &str, v: f64, quality: i32) {
        self.upsert(name, source, PB_VALUE_F64, &v.to_ne_bytes(), quality);
    }

    pub fn upsert_text(&self, name: &str, source: &str, v: &str, quality: i32) {
        self.upsert(name, source, PB_VALUE_STRING, v.as_bytes(), quality);
    }
}

/// What a protocol pack implements. Defaults cover packs that are read-only / need no polling.
pub trait Pack: Sized + 'static {
    /// Stable pack id, NUL-terminated (`b"modbus\0"`).
    const ID: &'static [u8];
    /// Human title, NUL-terminated.
    const TITLE: &'static [u8];
    /// Register the `subscribe` entry point. Off => the host sees `None` and skips the call.
    const SUBSCRIBE: bool = false;
    /// Register the `publish` entry point (broker-style packs only).
    const PUBLISH: bool = false;

    fn new(host: HostRef) -> Self;
    /// Apply the JSON document the service built (`pb-service` runtime.rs).
    fn configure(&mut self, json: &str) -> Result<(), String>;
    /// Called after `configure` returned `Err`: record the message and log it.
    fn config_failed(&mut self, err: String);

    /// Start / stop background work. The default `connect` / `disconnect` call this.
    fn set_enabled(&mut self, _on: bool) {}
    fn connect(&mut self) -> c_int {
        self.set_enabled(true);
        0
    }
    fn disconnect(&mut self) -> c_int {
        self.set_enabled(false);
        0
    }
    /// Join workers. Called by `destroy` before the pack is dropped.
    fn shutdown(&mut self) {}

    /// Host egress: the point `name` changed (`bytes` are the host's textual / native value).
    fn write_point(&mut self, _name: &str, _kind: i32, _bytes: &[u8]) -> c_int {
        0
    }
    /// Non-blocking drain, called each engine tick.
    fn poll(&mut self) {}
    fn subscribe(&mut self, _topic: &str, _qos: u8) -> c_int {
        -1
    }
    fn publish(&mut self, _topic: &str, _payload: &[u8], _qos: u8, _retain: bool) -> c_int {
        -1
    }

    fn state_label(&self) -> String;
    fn last_error(&self) -> String;
    fn is_connected(&self) -> bool;
    /// Pack-specific status JSON; lands in the UI's slot `detail_json`.
    fn detail_json(&self) -> String;
}

/// Lossy UTF-8 view of a host value buffer — what most packs want in `write_point`.
pub fn value_text(bytes: &[u8]) -> std::borrow::Cow<'_, str> {
    String::from_utf8_lossy(bytes)
}

// ---- generic C shims (monomorphised per pack) ----------------------------------------------

unsafe fn pack<'a, P: Pack>(h: *mut c_void) -> Option<&'a mut P> {
    if h.is_null() {
        None
    } else {
        Some(&mut *(h as *mut P))
    }
}

unsafe extern "C" fn c_create<P: Pack>(host: *const PbHostApi) -> *mut c_void {
    if host.is_null() || (*host).abi_version != PB_ABI_VERSION {
        return std::ptr::null_mut();
    }
    Box::into_raw(Box::new(P::new(HostRef::new(*host)))) as *mut c_void
}

unsafe extern "C" fn c_destroy<P: Pack>(h: *mut c_void) {
    if !h.is_null() {
        let mut p = Box::from_raw(h as *mut P);
        p.shutdown();
    }
}

unsafe extern "C" fn c_configure<P: Pack>(h: *mut c_void, json: *const c_char, len: usize) -> c_int {
    let Some(p) = pack::<P>(h) else { return -1 };
    if json.is_null() {
        return -1;
    }
    let Ok(text) = std::str::from_utf8(std::slice::from_raw_parts(json as *const u8, len)) else {
        return -1;
    };
    match p.configure(text) {
        Ok(()) => 0,
        Err(e) => {
            p.config_failed(e);
            -1
        }
    }
}

unsafe extern "C" fn c_connect<P: Pack>(h: *mut c_void) -> c_int {
    pack::<P>(h).map_or(-1, |p| p.connect())
}

unsafe extern "C" fn c_disconnect<P: Pack>(h: *mut c_void) -> c_int {
    pack::<P>(h).map_or(-1, |p| p.disconnect())
}

unsafe extern "C" fn c_subscribe<P: Pack>(h: *mut c_void, topic: *const c_char, qos: u8) -> c_int {
    let Some(p) = pack::<P>(h) else { return -1 };
    if topic.is_null() {
        return -1;
    }
    match CStr::from_ptr(topic).to_str() {
        Ok(t) => p.subscribe(t, qos),
        Err(_) => -1,
    }
}

unsafe extern "C" fn c_publish<P: Pack>(
    h: *mut c_void,
    topic: *const c_char,
    payload: *const u8,
    len: usize,
    qos: u8,
    retain: c_int,
) -> c_int {
    let Some(p) = pack::<P>(h) else { return -1 };
    if topic.is_null() {
        return -1;
    }
    let Ok(topic) = CStr::from_ptr(topic).to_str() else { return -1 };
    let body: &[u8] = if payload.is_null() || len == 0 { &[] } else { std::slice::from_raw_parts(payload, len) };
    p.publish(topic, body, qos, retain != 0)
}

unsafe extern "C" fn c_write_point<P: Pack>(
    h: *mut c_void,
    name: *const c_char,
    kind: i32,
    value: *const c_void,
    len: usize,
) -> c_int {
    let Some(p) = pack::<P>(h) else { return -1 };
    if name.is_null() {
        return -1;
    }
    let Ok(name) = CStr::from_ptr(name).to_str() else { return -1 };
    let bytes: &[u8] = if value.is_null() || len == 0 { &[] } else { std::slice::from_raw_parts(value as *const u8, len) };
    p.write_point(name, kind, bytes)
}

unsafe extern "C" fn c_poll<P: Pack>(h: *mut c_void) {
    if let Some(p) = pack::<P>(h) {
        p.poll();
    }
}

unsafe extern "C" fn c_state_label<P: Pack>(h: *mut c_void, out: *mut c_char, n: usize) -> c_int {
    pack::<P>(h).map_or(-1, |p| write_cstr(out, n, &p.state_label()))
}

unsafe extern "C" fn c_last_error<P: Pack>(h: *mut c_void, out: *mut c_char, n: usize) -> c_int {
    pack::<P>(h).map_or(-1, |p| write_cstr(out, n, &p.last_error()))
}

unsafe extern "C" fn c_is_connected<P: Pack>(h: *mut c_void) -> c_int {
    pack::<P>(h).map_or(0, |p| p.is_connected() as c_int)
}

unsafe extern "C" fn c_topics_json<P: Pack>(h: *mut c_void, out: *mut c_char, n: usize) -> c_int {
    pack::<P>(h).map_or(-1, |p| write_cstr(out, n, &p.detail_json()))
}

/// Fill the host-facing function table for pack `P`. Used by [`export_pack!`].
///
/// # Safety
/// `out` must be null or point to writable memory for one `PbPackApi`.
pub unsafe fn fill_api<P: Pack>(out: *mut PbPackApi) -> c_int {
    if out.is_null() {
        return -1;
    }
    *out = PbPackApi {
        abi_version: PB_ABI_VERSION,
        id: P::ID.as_ptr() as *const c_char,
        title: P::TITLE.as_ptr() as *const c_char,
        create: Some(c_create::<P>),
        destroy: Some(c_destroy::<P>),
        configure: Some(c_configure::<P>),
        connect: Some(c_connect::<P>),
        disconnect: Some(c_disconnect::<P>),
        subscribe: if P::SUBSCRIBE { Some(c_subscribe::<P>) } else { None },
        publish: if P::PUBLISH { Some(c_publish::<P>) } else { None },
        write_point: Some(c_write_point::<P>),
        poll: Some(c_poll::<P>),
        state_label: Some(c_state_label::<P>),
        last_error: Some(c_last_error::<P>),
        is_connected: Some(c_is_connected::<P>),
        topics_json: Some(c_topics_json::<P>),
    };
    0
}

/// Export the two symbols the host looks up (`pb_pack_abi_version`, `pb_pack_register`).
///
/// Use exactly once per cdylib. A crate that exports these symbols cannot be linked into the same
/// binary as another pack — share code through a plain rlib instead (see `pb-tail`).
#[macro_export]
macro_rules! export_pack {
    ($pack:ty) => {
        #[no_mangle]
        pub unsafe extern "C" fn pb_pack_abi_version() -> u32 {
            $crate::PB_ABI_VERSION
        }

        #[no_mangle]
        pub unsafe extern "C" fn pb_pack_register(out: *mut $crate::PbPackApi) -> ::std::os::raw::c_int {
            $crate::pack::fill_api::<$pack>(out)
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PB_LOG_INFO, PB_QUALITY_GOOD};
    use std::sync::Mutex;

    static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

    #[derive(Default)]
    struct Dummy {
        enabled: bool,
        err: String,
        seen: Vec<String>,
    }

    impl Pack for Dummy {
        const ID: &'static [u8] = b"dummy\0";
        const TITLE: &'static [u8] = b"Dummy\0";
        fn new(_host: HostRef) -> Self {
            Self::default()
        }
        fn configure(&mut self, json: &str) -> Result<(), String> {
            if json == "ok" { Ok(()) } else { Err(format!("bad config: {json}")) }
        }
        fn config_failed(&mut self, err: String) {
            self.err = err;
        }
        fn set_enabled(&mut self, on: bool) {
            self.enabled = on;
        }
        fn write_point(&mut self, name: &str, _kind: i32, bytes: &[u8]) -> c_int {
            self.seen.push(format!("{name}={}", value_text(bytes)));
            0
        }
        fn state_label(&self) -> String {
            if self.enabled { "on".into() } else { "off".into() }
        }
        fn last_error(&self) -> String {
            self.err.clone()
        }
        fn is_connected(&self) -> bool {
            self.enabled
        }
        fn detail_json(&self) -> String {
            format!("{{\"seen\":{}}}", self.seen.len())
        }
    }

    unsafe extern "C" fn log_cb(_ctx: *mut c_void, _level: i32, msg: *const c_char) {
        LOG.lock().unwrap().push(CStr::from_ptr(msg).to_string_lossy().into_owned());
    }

    fn host() -> PbHostApi {
        PbHostApi { abi_version: PB_ABI_VERSION, ctx: std::ptr::null_mut(), upsert_point: None, log_message: Some(log_cb) }
    }

    fn read(f: unsafe extern "C" fn(*mut c_void, *mut c_char, usize) -> c_int, h: *mut c_void) -> String {
        let mut buf = [0 as c_char; 64];
        let n = unsafe { f(h, buf.as_mut_ptr(), buf.len()) };
        assert!(n >= 0);
        unsafe { CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned()
    }

    #[test]
    fn table_wires_every_entry_point() {
        let mut api = unsafe { std::mem::zeroed::<PbPackApi>() };
        assert_eq!(unsafe { fill_api::<Dummy>(&mut api) }, 0);
        assert!(api.subscribe.is_none() && api.publish.is_none(), "SUBSCRIBE / PUBLISH off => not registered");
        assert_eq!(unsafe { CStr::from_ptr(api.id) }.to_str().unwrap(), "dummy");
        assert_eq!(unsafe { fill_api::<Dummy>(std::ptr::null_mut()) }, -1);
    }

    #[test]
    fn lifecycle_through_the_c_table() {
        let mut api = unsafe { std::mem::zeroed::<PbPackApi>() };
        unsafe { fill_api::<Dummy>(&mut api) };
        let host = host();
        let mut wrong = host;
        wrong.abi_version = 99;
        unsafe {
            assert!((api.create.unwrap())(&wrong).is_null(), "ABI mismatch must refuse");
            assert!((api.create.unwrap())(std::ptr::null()).is_null());
            let h = (api.create.unwrap())(&host);
            assert!(!h.is_null());
            assert_eq!((api.configure.unwrap())(h, b"nope".as_ptr() as *const c_char, 4), -1);
            assert_eq!(read(api.last_error.unwrap(), h), "bad config: nope");
            assert_eq!((api.configure.unwrap())(h, b"ok".as_ptr() as *const c_char, 2), 0);
            assert_eq!(read(api.state_label.unwrap(), h), "off");
            assert_eq!((api.connect.unwrap())(h), 0);
            assert_eq!((api.is_connected.unwrap())(h), 1);
            assert_eq!(read(api.state_label.unwrap(), h), "on");
            let v = b"12.5";
            assert_eq!((api.write_point.unwrap())(h, c"t".as_ptr(), 2, v.as_ptr() as *const c_void, v.len()), 0);
            assert_eq!((api.write_point.unwrap())(h, std::ptr::null(), 2, std::ptr::null(), 0), -1);
            assert_eq!(read(api.topics_json.unwrap(), h), "{\"seen\":1}");
            assert_eq!((api.disconnect.unwrap())(h), 0);
            assert_eq!((api.is_connected.unwrap())(h), 0);
            // Null handle never crashes.
            assert_eq!((api.connect.unwrap())(std::ptr::null_mut()), -1);
            assert_eq!((api.is_connected.unwrap())(std::ptr::null_mut()), 0);
            (api.destroy.unwrap())(h);
            (api.destroy.unwrap())(std::ptr::null_mut());
        }
    }

    #[test]
    fn host_log_survives_interior_nul() {
        let h = HostRef::new(host());
        h.log(PB_LOG_INFO, "a\0b");
        assert!(LOG.lock().unwrap().iter().any(|l| l == "a b"));
        // No upsert callback: must be a quiet no-op.
        h.upsert_f64("x", "y", 1.0, PB_QUALITY_GOOD);
    }

    #[test]
    fn u64_above_i64_max_becomes_float() {
        use std::sync::atomic::{AtomicI32, Ordering};
        static KIND: AtomicI32 = AtomicI32::new(-1);
        unsafe extern "C" fn up(_c: *mut c_void, _n: *const c_char, kind: i32, _v: *const c_void, _l: usize, _q: i32, _s: *const c_char) {
            KIND.store(kind, Ordering::SeqCst);
        }
        let mut api = host();
        api.upsert_point = Some(up);
        let h = HostRef::new(api);
        h.upsert_u64("n", "s", 5, 0);
        assert_eq!(KIND.load(Ordering::SeqCst), PB_VALUE_I64);
        h.upsert_u64("n", "s", u64::MAX, 0);
        assert_eq!(KIND.load(Ordering::SeqCst), PB_VALUE_F64);
    }
}
