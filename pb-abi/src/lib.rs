//! Stable C ABI shared by Protocol Bench hosts and protocol packs.
//!
//! Packs export [`pb_pack_abi_version`] and [`pb_pack_register`]. The host
//! refuses to load a pack when the ABI major does not match [`PB_ABI_VERSION`].

use std::os::raw::{c_char, c_int, c_void};

/// Host / pack ABI major. Bump when the C layout or semantics break.
pub const PB_ABI_VERSION: u32 = 1;

/// Value kind for [`PbHostApi::upsert_point`].
pub const PB_VALUE_BOOL: i32 = 0;
pub const PB_VALUE_I64: i32 = 1;
pub const PB_VALUE_F64: i32 = 2;
pub const PB_VALUE_STRING: i32 = 3;
pub const PB_VALUE_BYTES: i32 = 4;

/// Point quality for host upserts.
pub const PB_QUALITY_GOOD: i32 = 0;
pub const PB_QUALITY_BAD: i32 = 1;
pub const PB_QUALITY_UNCERTAIN: i32 = 2;

/// Log levels for [`PbHostApi::log_message`].
pub const PB_LOG_DEBUG: i32 = 0;
pub const PB_LOG_INFO: i32 = 1;
pub const PB_LOG_WARN: i32 = 2;
pub const PB_LOG_ERROR: i32 = 3;

/// Host callbacks packs may call. All pointers are host-owned unless noted.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PbHostApi {
    pub abi_version: u32,
    pub ctx: *mut c_void,
    /// Upsert a typed point. `value` / `value_len` depend on `value_kind`.
    pub upsert_point: Option<
        unsafe extern "C" fn(
            ctx: *mut c_void,
            name: *const c_char,
            value_kind: i32,
            value: *const c_void,
            value_len: usize,
            quality: i32,
            source: *const c_char,
        ),
    >,
    /// Host log line (UTF-8).
    pub log_message:
        Option<unsafe extern "C" fn(ctx: *mut c_void, level: i32, message: *const c_char)>,
}

/// Pack entry points the host calls after a successful register.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PbPackApi {
    pub abi_version: u32,
    /// Stable pack id, e.g. `"mqtt"` (static C string).
    pub id: *const c_char,
    /// Human title, e.g. `"MQTT"` (static C string).
    pub title: *const c_char,
    pub create: Option<unsafe extern "C" fn(host: *const PbHostApi) -> *mut c_void>,
    pub destroy: Option<unsafe extern "C" fn(handle: *mut c_void)>,
    /// JSON config blob (UTF-8). Returns 0 on success.
    pub configure:
        Option<unsafe extern "C" fn(handle: *mut c_void, json: *const c_char, len: usize) -> c_int>,
    pub connect: Option<unsafe extern "C" fn(handle: *mut c_void) -> c_int>,
    pub disconnect: Option<unsafe extern "C" fn(handle: *mut c_void) -> c_int>,
    pub subscribe: Option<
        unsafe extern "C" fn(handle: *mut c_void, topic: *const c_char, qos: u8) -> c_int,
    >,
    pub publish: Option<
        unsafe extern "C" fn(
            handle: *mut c_void,
            topic: *const c_char,
            payload: *const u8,
            len: usize,
            qos: u8,
            retain: c_int,
        ) -> c_int,
    >,
    /// Host-driven point write → pack may publish / forward.
    pub write_point: Option<
        unsafe extern "C" fn(
            handle: *mut c_void,
            name: *const c_char,
            value_kind: i32,
            value: *const c_void,
            value_len: usize,
        ) -> c_int,
    >,
    /// Non-blocking poll (drain events, refresh status).
    pub poll: Option<unsafe extern "C" fn(handle: *mut c_void)>,
    /// Write UTF-8 state label into `out` (NUL-terminated). Returns bytes written or -1.
    pub state_label: Option<
        unsafe extern "C" fn(handle: *mut c_void, out: *mut c_char, out_len: usize) -> c_int,
    >,
    pub last_error: Option<
        unsafe extern "C" fn(handle: *mut c_void, out: *mut c_char, out_len: usize) -> c_int,
    >,
    /// 1 connected, 0 otherwise.
    pub is_connected: Option<unsafe extern "C" fn(handle: *mut c_void) -> c_int>,
    /// JSON array of subscribed topic strings into `out`. Returns bytes or -1.
    pub topics_json: Option<
        unsafe extern "C" fn(handle: *mut c_void, out: *mut c_char, out_len: usize) -> c_int,
    >,
}

/// Exported by every pack: ABI major this binary was built against.
pub type PbPackAbiVersionFn = unsafe extern "C" fn() -> u32;

/// Exported by every pack: fill [`PbPackApi`]. Returns 0 on success.
pub type PbPackRegisterFn = unsafe extern "C" fn(out: *mut PbPackApi) -> c_int;

pub const PB_PACK_ABI_VERSION_SYMBOL: &[u8] = b"pb_pack_abi_version\0";
pub const PB_PACK_REGISTER_SYMBOL: &[u8] = b"pb_pack_register\0";

/// Copy a UTF-8 string into a C buffer (NUL-terminated). Returns bytes excluding NUL, or -1.
pub unsafe fn write_cstr(out: *mut c_char, out_len: usize, text: &str) -> c_int {
    if out.is_null() || out_len == 0 {
        return -1;
    }
    let max = out_len.saturating_sub(1);
    let bytes = text.as_bytes();
    let n = bytes.len().min(max);
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), out as *mut u8, n);
        *out.add(n) = 0;
    }
    n as c_int
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;
    use std::mem::MaybeUninit;

    #[test]
    fn write_cstr_null_terminates() {
        let mut buf = [0u8; 8];
        let n = unsafe { write_cstr(buf.as_mut_ptr() as *mut c_char, buf.len(), "hi") };
        assert_eq!(n, 2);
        let s = unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) };
        assert_eq!(s.to_str().unwrap(), "hi");
    }

    #[test]
    fn pack_api_is_c_layout() {
        let _ = MaybeUninit::<PbPackApi>::uninit();
        let _ = MaybeUninit::<PbHostApi>::uninit();
        assert_eq!(PB_ABI_VERSION, 1);
    }
}
