//! CAN pack — CAN signals in and out over SocketCAN, a built-in virtual bus, or a PEAK PCAN
//! Ethernet gateway.
//!
//! * **Input** binds decode a signal (`id`, `start`, `len`, `order`, `type`, scale / offset)
//!   from matching frames → `can_N`. `id` may be a J1939 `pgn:NNNNN` to match any source.
//!   No matching frame for `stale_secs` turns the point bad.
//! * **Output** binds encode host points into frames (one 8-byte buffer per frame id) that are
//!   sent every Output interval once a value is known.
//!
//! Connection `transport`:
//! * `socketcan` (default) — Linux `interface` (`can0`, or `vcan0` from
//!   `sudo ip link add dev vcan0 type vcan && sudo ip link set up vcan0`).
//! * `virtual` — the built-in in-process bus named `bus`; no kernel module or root, any OS.
//!   Like SocketCAN, a frame reaches every other connection on the bus but not its sender.
//! * `pcan_gw` — PEAK PCAN Ethernet gateway at `host`. `protocol` `udp`: send to `port` (the
//!   gateway's LAN → CAN route), listen on `local_port` (its CAN → LAN route targets this PC);
//!   `tcp`: one connection to `host:port` both ways.

pub mod pack;
