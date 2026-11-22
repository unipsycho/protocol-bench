//! SNMP pack (cdylib): v1 / v2c GET polling of agent OIDs → points.
//!
//! * [`ber`] — BER / SNMP message codec (pure)
//! * [`client`] — blocking UDP manager with retries and request batching
//! * `pack` — C-ABI pack: one worker thread per target

pub mod ber;
pub mod client;
mod pack;
