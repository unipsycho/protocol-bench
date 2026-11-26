//! CAN pack — SocketCAN (Linux) signals in and out.
//!
//! * **Input** binds decode a signal (`id`, `start`, `len`, `order`, `type`, scale / offset)
//!   from matching frames → `can_N`. `id` may be a J1939 `pgn:NNNNN` to match any source.
//!   No matching frame for `stale_secs` turns the point bad.
//! * **Output** binds encode host points into frames (one 8-byte buffer per frame id) that are
//!   sent every Output interval once a value is known.
//!
//! Interfaces: real adapters (`can0`) or virtual ones for testing:
//! `sudo ip link add dev vcan0 type vcan && sudo ip link set up vcan0`.

pub mod pack;
