//! Localhost TCP JSON IPC for Protocol Bench (PB-96).
//!
//! Framing: u32 LE length + UTF-8 JSON request/response.
//! Transport: 127.0.0.1 only. Auth: `ipc.token` next to `ipc.json`.

mod client;
mod frame;
mod paths;
mod proto;

pub use client::RemoteHandle;
pub use frame::{read_json, write_json};
pub use paths::{
    config_dir, default_project_path, endpoint_path, ensure_token, read_endpoint, read_token,
    token_path, write_endpoint, write_token, IpcEndpoint,
};
pub use proto::*;
