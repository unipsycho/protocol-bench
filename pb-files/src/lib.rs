//! File follow pack (cdylib): growing CSV / JSON-lines files → points.
//!
//! * [`tail`] — byte-offset tailing with truncate / rotate detection and saved state
//! * [`header`] — header modes and smart preamble / header / data-start detection
//! * [`parse`] — CSV line splitting, JSON paths, value typing
//! * `pack` — C-ABI pack: one worker thread per source

pub mod header;
pub mod parse;
mod pack;
/// Byte-offset tailing lives in the shared `pb-tail` crate.
pub use pb_tail as tail;
