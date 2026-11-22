//! LAS pack (cdylib): follow LAS 1.2 / 2.0 well-log files (Input) and append curve rows to a
//! LAS file (Output).
//!
//! * [`las`] — header / data-row parsing and writing (pure)
//! * `pack` — C-ABI pack: one worker thread per endpoint

pub mod las;
mod pack;
