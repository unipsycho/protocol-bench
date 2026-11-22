//! Modbus protocol pack (cdylib): TCP + RTU client (this phase), server (next).
//!
//! * [`codec`] — register ↔ typed value, read planning (pure)
//! * [`frame`] — PDU / MBAP / RTU framing and CRC (pure)
//! * [`bank`] — register bank + request handler (pure; backs the server and the loopback tests)
//! * [`client`] — blocking TCP / serial master session
//! * `pack` — C-ABI pack: per-connection worker threads

pub mod bank;
pub mod client;
pub mod codec;
pub mod frame;
pub mod server;

mod pack;
