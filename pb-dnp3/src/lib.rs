//! DNP3 pack — TCP master (client) for RTUs / outstations.
//!
//! * Each Input interval the master reads class 0 (static values) plus class 1 / 2 / 3 events;
//!   binds (`kind` + `index` → `dnp3_N`) take binary inputs, binary output status, counters,
//!   analog inputs and analog output status. A point reported with ONLINE cleared is bad; a
//!   dead link turns every point bad and is retried.
//! * DEVICE RESTART in IIN is cleared automatically (WRITE g80v1).
//! * Output binds operate controls with DIRECT OPERATE: `binary_output` sends a CROB (LATCH ON
//!   / OFF from the point's truthiness), `analog_output` sends g41v3 (float, default) or g41v1.
//!
//! Codec and a demo outstation live in `pb-proto::dnp3`.

pub mod pack;
