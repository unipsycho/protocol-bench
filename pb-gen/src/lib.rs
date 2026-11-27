//! Generator pack: invents data instead of reading a device.
//!
//! A *connection* is a generator **board** (a name, a random seed, a time-scale). Each Input bind
//! on a hub node is one **signal**: a model (sine, random walk, formula, …) plus a **trigger**
//! that says *when* it produces a value (fixed interval, random timer, or bursts). Signals become
//! ordinary host points, so they can be wired to a hub, Aggregator, Calculation or Output like any
//! other source. A generator has no Output side: nothing can be wired into it.
//!
//! * [`model`] (the `pb-signal` crate) — the signal models and the trigger schedule. Pure and
//!   deterministic: given a seed and explicit `(t, dt)` they produce the same values every time.
//! * [`pack`] — the worker that owns the clock, fires signals when due and publishes points.

pub mod pack;

pub use pb_signal as model;
pub use pb_signal::{catalog, Signal, Trigger};
