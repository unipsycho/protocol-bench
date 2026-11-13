//! Host-owned typed points. Protocol packs never own this store — they upsert
//! only through the pack ABI host callbacks that the service wires here.
//!
//! Point **identity** is a short id `XXXX_n` (3–4 char protocol code + `_` +
//! sequential decimal, e.g. `mqtt_1`). Numbers are never reused within a
//! project. The long wire address (broker/topic/field) lives in `source`.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// Typed point value held by the host.
#[derive(Debug, Clone, PartialEq)]
pub enum PointValue {
    Bool(bool),
    I64(i64),
    F64(f64),
    String(String),
    Bytes(Vec<u8>),
}

impl PointValue {
    pub fn as_display(&self) -> String {
        match self {
            Self::Bool(v) => v.to_string(),
            Self::I64(v) => v.to_string(),
            Self::F64(v) => v.to_string(),
            Self::String(v) => v.clone(),
            Self::Bytes(v) => format!("<{} bytes>", v.len()),
        }
    }
}

/// Point quality for UI / mappings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quality {
    Good,
    Bad,
    Uncertain,
}

impl Quality {
    pub fn label(self) -> &'static str {
        match self {
            Self::Good => "good",
            Self::Bad => "bad",
            Self::Uncertain => "uncertain",
        }
    }

    pub fn from_abi(code: i32) -> Self {
        match code {
            0 => Self::Good,
            1 => Self::Bad,
            _ => Self::Uncertain,
        }
    }

    pub fn to_abi(self) -> i32 {
        match self {
            Self::Good => 0,
            Self::Bad => 1,
            Self::Uncertain => 2,
        }
    }
}

/// One named point in the host store.
#[derive(Debug, Clone, PartialEq)]
pub struct Point {
    /// Short id, e.g. `mqtt_k7m2`.
    pub name: String,
    pub value: PointValue,
    pub quality: Quality,
    /// Wire address / source metadata, e.g. `mqtt/<client>/<topic>::path`.
    pub source: String,
    pub updated_ms: u64,
}

/// Persisted short-id ↔ source binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PointBinding {
    pub id: String,
    pub source: String,
}

/// Host point table. Packs reach this only via host ABI callbacks.
#[derive(Debug, Clone)]
pub struct PointStore {
    points: BTreeMap<String, Point>,
    /// source address → short id
    ids_by_source: BTreeMap<String, String>,
    /// short id → source address
    sources_by_id: BTreeMap<String, String>,
    /// Next sequential number to issue (starts at 1; never decreases).
    next_seq: u64,
}

impl Default for PointStore {
    fn default() -> Self {
        Self {
            points: BTreeMap::new(),
            ids_by_source: BTreeMap::new(),
            sources_by_id: BTreeMap::new(),
            next_seq: 1,
        }
    }
}

impl PointStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Upsert by short id (name) with source metadata.
    pub fn upsert(
        &mut self,
        name: impl Into<String>,
        value: PointValue,
        quality: Quality,
        source: impl Into<String>,
    ) {
        let name = name.into();
        let source = source.into();
        if is_short_point_id(&name) && is_address_path(&source) {
            self.bind(&name, &source);
        }
        self.points.insert(
            name.clone(),
            Point {
                name,
                value,
                quality,
                source,
                updated_ms: now_ms(),
            },
        );
    }

    /// Pack-facing upsert: `name` may be a long address or a short id.
    /// Returns the short id stored.
    pub fn upsert_from_pack(
        &mut self,
        name: &str,
        value: PointValue,
        quality: Quality,
        source: &str,
    ) -> String {
        let name = name.trim();
        let source = source.trim();
        if is_address_path(name) {
            // Packs still pass the long path as the name; host owns short ids.
            let proto = protocol_code(source).unwrap_or_else(|| protocol_from_address(name));
            let id = self.ensure_id(proto, name);
            self.points.insert(
                id.clone(),
                Point {
                    name: id.clone(),
                    value,
                    quality,
                    source: name.to_string(),
                    updated_ms: now_ms(),
                },
            );
            return id;
        }
        let meta = if is_address_path(source) {
            source.to_string()
        } else {
            self.sources_by_id
                .get(name)
                .cloned()
                .unwrap_or_else(|| source.to_string())
        };
        if is_short_point_id(name) && is_address_path(&meta) {
            self.bind(name, &meta);
        }
        self.points.insert(
            name.to_string(),
            Point {
                name: name.to_string(),
                value,
                quality,
                source: meta,
                updated_ms: now_ms(),
            },
        );
        name.to_string()
    }

    /// Stable short id for a source address (allocate if missing).
    pub fn ensure_id(&mut self, proto: &str, source: &str) -> String {
        let source = source.trim();
        if source.is_empty() {
            return self.alloc_id(proto);
        }
        if let Some(id) = self.ids_by_source.get(source) {
            return id.clone();
        }
        let id = self.alloc_id(proto);
        self.bind(&id, source);
        id
    }

    /// Register an explicit id ↔ source pair (project load / user override).
    /// Does not free or reuse earlier sequence numbers.
    pub fn bind(&mut self, id: &str, source: &str) {
        let id = id.trim();
        let source = source.trim();
        if id.is_empty() || source.is_empty() {
            return;
        }
        if let Some(old_src) = self.sources_by_id.insert(id.to_string(), source.to_string()) {
            if old_src != source {
                self.ids_by_source.remove(&old_src);
            }
        }
        if let Some(old_id) = self.ids_by_source.insert(source.to_string(), id.to_string()) {
            if old_id != id {
                // Keep the old id reserved (never reuse); only drop its reverse map entry.
                self.sources_by_id.remove(&old_id);
            }
        }
        self.note_issued_id(id);
    }

    pub fn id_for_source(&self, source: &str) -> Option<&str> {
        self.ids_by_source.get(source).map(|s| s.as_str())
    }

    pub fn source_for_id(&self, id: &str) -> Option<&str> {
        self.sources_by_id.get(id).map(|s| s.as_str())
    }

    pub fn export_bindings(&self) -> Vec<PointBinding> {
        self.sources_by_id
            .iter()
            .map(|(id, source)| PointBinding {
                id: id.clone(),
                source: source.clone(),
            })
            .collect()
    }

    pub fn import_bindings(&mut self, bindings: &[PointBinding]) {
        for b in bindings {
            self.bind(&b.id, &b.source);
        }
    }

    /// Next sequence number that will be issued (1-based).
    pub fn next_seq(&self) -> u64 {
        self.next_seq.max(1)
    }

    /// Raise the high-water mark (project load). Never lowers it.
    pub fn set_next_seq_at_least(&mut self, n: u64) {
        if n > 0 {
            self.next_seq = self.next_seq.max(n);
        }
    }

    pub fn get(&self, name: &str) -> Option<&Point> {
        self.points.get(name)
    }

    pub fn remove(&mut self, name: &str) -> Option<Point> {
        self.points.remove(name)
    }

    pub fn len(&self) -> usize {
        self.points.len()
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Point> {
        self.points.values()
    }

    pub fn snapshot(&self) -> Vec<Point> {
        self.points.values().cloned().collect()
    }

    fn alloc_id(&mut self, proto: &str) -> String {
        let proto = normalize_proto(proto);
        loop {
            let n = self.next_seq.max(1);
            self.next_seq = n.saturating_add(1);
            let id = format!("{proto}_{n}");
            // Skip if somehow reserved (imported gap / other proto collision).
            if !self.sources_by_id.contains_key(&id) && !self.points.contains_key(&id) {
                return id;
            }
        }
    }

    fn note_issued_id(&mut self, id: &str) {
        if let Some(n) = parse_seq_suffix(id) {
            self.next_seq = self.next_seq.max(n.saturating_add(1));
        }
    }
}

/// `mqtt_1`-style short ids (3–4 char proto + `_` + decimal sequence).
pub fn is_short_point_id(name: &str) -> bool {
    let Some((proto, suffix)) = name.split_once('_') else {
        return false;
    };
    let plen = proto.len();
    if !(3..=4).contains(&plen) || !proto.chars().all(|c| c.is_ascii_alphanumeric()) {
        return false;
    }
    !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit())
}

/// Long wire address: contains `/` (e.g. `mqtt/local/topic::path`).
pub fn is_address_path(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty() && name.contains('/')
}

fn protocol_code(source: &str) -> Option<&str> {
    let s = source.trim();
    if s.is_empty() || is_address_path(s) {
        return None;
    }
    if (3..=4).contains(&s.len()) && s.chars().all(|c| c.is_ascii_alphanumeric()) {
        Some(s)
    } else {
        None
    }
}

fn protocol_from_address(address: &str) -> &str {
    address
        .split_once('/')
        .map(|(p, _)| p)
        .filter(|p| (3..=4).contains(&p.len()))
        .unwrap_or("mqtt")
}

fn normalize_proto(proto: &str) -> String {
    let p = proto.trim().to_ascii_lowercase();
    if (3..=4).contains(&p.len()) && p.chars().all(|c| c.is_ascii_alphanumeric()) {
        p
    } else {
        "mqtt".into()
    }
}

fn parse_seq_suffix(id: &str) -> Option<u64> {
    let suffix = id.split_once('_')?.1;
    if suffix.is_empty() || !suffix.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    suffix.parse().ok()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_replaces_value() {
        let mut store = PointStore::new();
        store.upsert("mqtt_1", PointValue::String("1".into()), Quality::Good, "mqtt/local/a");
        store.upsert("mqtt_1", PointValue::String("2".into()), Quality::Bad, "mqtt/local/a");
        let p = store.get("mqtt_1").unwrap();
        assert_eq!(p.value, PointValue::String("2".into()));
        assert_eq!(p.quality, Quality::Bad);
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn pack_address_becomes_sequential_id() {
        let mut store = PointStore::new();
        let id = store.upsert_from_pack(
            "mqtt/local/pb/in",
            PointValue::String("hi".into()),
            Quality::Good,
            "mqtt",
        );
        assert_eq!(id, "mqtt_1");
        let p = store.get(&id).unwrap();
        assert_eq!(p.source, "mqtt/local/pb/in");
        // Same address keeps the same id.
        let id2 = store.upsert_from_pack(
            "mqtt/local/pb/in",
            PointValue::String("yo".into()),
            Quality::Good,
            "mqtt",
        );
        assert_eq!(id2, "mqtt_1");
        let id3 = store.upsert_from_pack(
            "mqtt/local/other",
            PointValue::String("z".into()),
            Quality::Good,
            "mqtt",
        );
        assert_eq!(id3, "mqtt_2");
    }

    #[test]
    fn never_reuses_sequence_numbers() {
        let mut store = PointStore::new();
        let a = store.ensure_id("mqtt", "mqtt/local/a");
        let b = store.ensure_id("mqtt", "mqtt/local/b");
        assert_eq!(a, "mqtt_1");
        assert_eq!(b, "mqtt_2");
        // Drop live point + rebind source to a new address — number 1 stays retired.
        store.remove("mqtt_1");
        store.ids_by_source.remove("mqtt/local/a");
        store.sources_by_id.remove("mqtt_1");
        let c = store.ensure_id("mqtt", "mqtt/local/c");
        assert_eq!(c, "mqtt_3");
    }

    #[test]
    fn explicit_short_id_binds_source() {
        let mut store = PointStore::new();
        store.upsert_from_pack(
            "mqtt_7",
            PointValue::String("1".into()),
            Quality::Good,
            "mqtt/local/t::temp",
        );
        assert_eq!(store.id_for_source("mqtt/local/t::temp"), Some("mqtt_7"));
        assert_eq!(store.source_for_id("mqtt_7"), Some("mqtt/local/t::temp"));
        assert_eq!(store.ensure_id("mqtt", "mqtt/local/next"), "mqtt_8");
    }

    #[test]
    fn import_export_bindings() {
        let mut store = PointStore::new();
        store.import_bindings(&[PointBinding {
            id: "mqtt_9".into(),
            source: "mqtt/local/x".into(),
        }]);
        let id = store.ensure_id("mqtt", "mqtt/local/x");
        assert_eq!(id, "mqtt_9");
        assert_eq!(store.ensure_id("mqtt", "mqtt/local/y"), "mqtt_10");
        let exported = store.export_bindings();
        assert!(exported.iter().any(|b| b.id == "mqtt_9"));
    }
}
