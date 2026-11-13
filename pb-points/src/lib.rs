//! Host-owned typed points. Protocol packs never own this store — they upsert
//! only through the pack ABI host callbacks that the service wires here.

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
    pub name: String,
    pub value: PointValue,
    pub quality: Quality,
    pub source: String,
    pub updated_ms: u64,
}

/// Host point table. Packs reach this only via host ABI callbacks.
#[derive(Debug, Default, Clone)]
pub struct PointStore {
    points: BTreeMap<String, Point>,
}

impl PointStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert(
        &mut self,
        name: impl Into<String>,
        value: PointValue,
        quality: Quality,
        source: impl Into<String>,
    ) {
        let name = name.into();
        self.points.insert(
            name.clone(),
            Point {
                name,
                value,
                quality,
                source: source.into(),
                updated_ms: now_ms(),
            },
        );
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
        store.upsert("t/a", PointValue::String("1".into()), Quality::Good, "mqtt");
        store.upsert("t/a", PointValue::String("2".into()), Quality::Bad, "mqtt");
        let p = store.get("t/a").unwrap();
        assert_eq!(p.value, PointValue::String("2".into()));
        assert_eq!(p.quality, Quality::Bad);
        assert_eq!(store.len(), 1);
    }
}
