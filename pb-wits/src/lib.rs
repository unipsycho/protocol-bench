//! WITS level 0 pack — the oilfield / frac record stream (not WITSML).
//!
//! A WITS level 0 data block is ASCII lines:
//!
//! ```text
//! &&
//! 0108 1234.56      (record 01, item 08 = bit depth)
//! 0113 45.2
//! !!
//! ```
//!
//! Each data line is a 4-digit item code (2-digit record + 2-digit item) followed directly by
//! the value. `&&` opens a block and `!!` closes it; lines outside a block are accepted too
//! (some senders omit the markers).
//!
//! One connection (TCP client, TCP listener or serial) carries both directions:
//! * **Input** binds (`item` → `wits_N`) take values from received blocks. A configured null
//!   value (default `-9999` / `-999.25`) makes that point bad; no block for `stale_secs` makes
//!   every Input point bad until data returns.
//! * **Output** binds (host point → `item`) are sent as one block every Output interval, with
//!   the latest value of each bound point (points with no value yet are left out).

pub mod pack;

pub use pb_proto::wits as codec;
