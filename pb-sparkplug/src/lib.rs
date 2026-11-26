//! Sparkplug B pack — MQTT with Sparkplug B payloads (Eclipse Sparkplug 3.0).
//!
//! A connection is an MQTT session in one of two roles:
//!
//! | Role | Inputs (`device` + `metric` → `spb_N`) | Outputs (host point → metric) |
//! | --- | --- | --- |
//! | **host** (SCADA / consumer) | metrics from NBIRTH / NDATA / DBIRTH / DDATA of the group (optionally one edge node); aliases resolved from BIRTH; DEATH turns them bad | NCMD / DCMD writes to the edge node |
//! | **edge** (edge node) | NCMD / DCMD commands sent to this node | NBIRTH on connect (all metrics, `bdSeq`), DBIRTH per device, then NDATA / DDATA on change; NDEATH is the MQTT will |
//!
//! The payload codec lives in `pb-proto::sparkplug`.

pub mod pack;
