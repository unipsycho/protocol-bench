//! Load signed-later protocol packs from a modules directory via libloading.
//!
//! Signature verification is a stub today — ABI version skew still refuses load.

use crate::pack_engine::HostBridge;
use libloading::Library;
use pb_abi::{
    PbHostApi, PbPackAbiVersionFn, PbPackApi, PbPackRegisterFn, PB_ABI_VERSION,
    PB_PACK_ABI_VERSION_SYMBOL, PB_PACK_REGISTER_SYMBOL,
};
use pb_points::PointStore;
use std::ffi::CStr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// One successfully registered pack (library must outlive the opaque handle).
pub struct LoadedPack {
    pub id: String,
    pub title: String,
    pub api: PbPackApi,
    pub host: Box<PbHostApi>,
    /// Keeps the host bridge alive for callbacks.
    pub _bridge: Box<HostBridge>,
    pub library: Library,
    pub path: PathBuf,
}

#[derive(Debug)]
pub enum PackLoadError {
    Io(std::io::Error),
    Open(libloading::Error),
    MissingSymbol(String),
    AbiSkew { pack: u32, host: u32 },
    RegisterFailed(i32),
    BadId,
    VerifyStub(String),
}

impl std::fmt::Display for PackLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io: {e}"),
            Self::Open(e) => write!(f, "open: {e}"),
            Self::MissingSymbol(s) => write!(f, "missing symbol {s}"),
            Self::AbiSkew { pack, host } => {
                write!(f, "ABI skew: pack={pack} host={host}")
            }
            Self::RegisterFailed(code) => write!(f, "pb_pack_register failed ({code})"),
            Self::BadId => write!(f, "pack id missing"),
            Self::VerifyStub(msg) => write!(f, "verify: {msg}"),
        }
    }
}

impl std::error::Error for PackLoadError {}

/// Default modules dir: `$PB_MODULES`, else `<exe_dir>/modules`, else `./modules`.
pub fn default_modules_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("PB_MODULES") {
        return PathBuf::from(dir);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let beside = parent.join("modules");
            if beside.is_dir() {
                return beside;
            }
        }
    }
    PathBuf::from("modules")
}

/// Stub verify-before-load. Real ed25519 / Authenticode lands with licensing.
/// Today: file must exist and be a regular file.
pub fn verify_pack_stub(path: &Path) -> Result<(), PackLoadError> {
    let meta = std::fs::metadata(path).map_err(PackLoadError::Io)?;
    if !meta.is_file() {
        return Err(PackLoadError::VerifyStub(format!(
            "{} is not a regular file",
            path.display()
        )));
    }
    if meta.len() == 0 {
        return Err(PackLoadError::VerifyStub(format!(
            "{} is empty",
            path.display()
        )));
    }
    Ok(())
}

/// Scan `modules_dir` for pack shared libraries and register those with matching ABI.
pub fn load_packs(modules_dir: &Path, points: Arc<Mutex<PointStore>>) -> Vec<LoadedPack> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(modules_dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !is_pack_library(&path) {
            continue;
        }
        match load_one(&path, Arc::clone(&points)) {
            Ok(pack) => {
                eprintln!(
                    "pb-engine: loaded pack '{}' ({}) from {}",
                    pack.id,
                    pack.title,
                    path.display()
                );
                out.push(pack);
            }
            Err(err) => {
                eprintln!("pb-engine: skip {}: {err}", path.display());
            }
        }
    }
    out
}

fn is_pack_library(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".so") || lower.ends_with(".dll") || lower.ends_with(".dylib")
}

fn load_one(path: &Path, points: Arc<Mutex<PointStore>>) -> Result<LoadedPack, PackLoadError> {
    verify_pack_stub(path)?;
    let library = unsafe { Library::new(path) }.map_err(PackLoadError::Open)?;

    let abi_fn: libloading::Symbol<PbPackAbiVersionFn> = unsafe {
        library
            .get(PB_PACK_ABI_VERSION_SYMBOL)
            .map_err(|_| PackLoadError::MissingSymbol("pb_pack_abi_version".into()))?
    };
    let pack_abi = unsafe { abi_fn() };
    if pack_abi != PB_ABI_VERSION {
        return Err(PackLoadError::AbiSkew {
            pack: pack_abi,
            host: PB_ABI_VERSION,
        });
    }

    let register: libloading::Symbol<PbPackRegisterFn> = unsafe {
        library
            .get(PB_PACK_REGISTER_SYMBOL)
            .map_err(|_| PackLoadError::MissingSymbol("pb_pack_register".into()))?
    };

    let mut api = unsafe { std::mem::zeroed::<PbPackApi>() };
    let rc = unsafe { register(&mut api) };
    if rc != 0 {
        return Err(PackLoadError::RegisterFailed(rc));
    }
    if api.abi_version != PB_ABI_VERSION {
        return Err(PackLoadError::AbiSkew {
            pack: api.abi_version,
            host: PB_ABI_VERSION,
        });
    }
    if api.id.is_null() {
        return Err(PackLoadError::BadId);
    }

    let id = unsafe { CStr::from_ptr(api.id) }
        .to_string_lossy()
        .into_owned();
    let title = if api.title.is_null() {
        id.clone()
    } else {
        unsafe { CStr::from_ptr(api.title) }
            .to_string_lossy()
            .into_owned()
    };

    let bridge = Box::new(HostBridge { points });
    let host = Box::new(PbHostApi {
        abi_version: PB_ABI_VERSION,
        ctx: bridge.as_ref() as *const HostBridge as *mut _,
        upsert_point: Some(crate::pack_engine::host_upsert_point),
        log_message: Some(crate::pack_engine::host_log_message),
    });

    Ok(LoadedPack {
        id,
        title,
        api,
        host,
        _bridge: bridge,
        library,
        path: path.to_path_buf(),
    })
}
