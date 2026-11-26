//! Pure protocol codecs shared by the ext packs and the `pb-sim` demo devices.
//!
//! Packs are `cdylib`s that export the pack ABI entry points, so two of them can never be linked
//! into one binary. Anything a pack *and* another binary need (the simulator, tests) lives here,
//! with no pack entry points — the same rule as `pb-tail` / `pb-json`.

pub mod can;
pub mod dnp3;
pub mod sparkplug;
pub mod wits;
