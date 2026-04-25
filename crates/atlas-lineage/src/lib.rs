//! Lineage edge journal and graph query service (T4.1, T4.2, T4.3).
//!
//! Records directed producer→consumer edges between ATLAS content-addressed
//! objects and supports BFS traversal to answer "where did this come from?"
//! or "what depends on this?".  Edges are appended to a JSONL journal on disk.
//! A sampling control lets high-throughput processes record only a fraction
//! of edges while still maintaining a statistically useful lineage graph.

pub mod journal;
pub mod rollup;

pub use journal::LineageJournal;
pub use rollup::{rollup_window, RollupBucket};

use atlas_core::Hash;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LineageError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, LineageError>;

/// The semantic relationship between source and sink objects.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    /// Source was read to produce/influence sink.
    Read,
    /// Sink was written (source = prior content hash or zero).
    Write,
    /// Sink was algorithmically derived from source.
    Derive,
    /// Source was byte-for-byte copied to produce sink.
    Copy,
    /// Source was processed by a model/transform to produce sink.
    Transform,
}

impl std::fmt::Display for EdgeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Derive => "derive",
            Self::Copy => "copy",
            Self::Transform => "transform",
        };
        write!(f, "{s}")
    }
}

impl std::str::FromStr for EdgeKind {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        match s {
            "read" => Ok(Self::Read),
            "write" => Ok(Self::Write),
            "derive" => Ok(Self::Derive),
            "copy" => Ok(Self::Copy),
            "transform" => Ok(Self::Transform),
            _ => Err(format!("unknown edge kind: {s:?}")),
        }
    }
}

/// A directed edge in the lineage graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineageEdge {
    /// UUID v4 — unique identifier for this edge.
    pub id: String,
    /// Milliseconds since Unix epoch.
    pub ts: u64,
    pub kind: EdgeKind,
    /// Producer / input object.
    pub source_hash: Hash,
    /// Consumer / output object.
    pub sink_hash: Hash,
    /// Agent that created the edge (process name, user, service).
    pub agent: String,
    /// Arbitrary key-value metadata.
    pub xattrs: HashMap<String, String>,
}

impl LineageEdge {
    pub fn new(kind: EdgeKind, source: Hash, sink: Hash, agent: impl Into<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            ts: now_millis(),
            kind,
            source_hash: source,
            sink_hash: sink,
            agent: agent.into(),
            xattrs: HashMap::new(),
        }
    }
}

/// Controls what fraction of edges are written to the journal.
#[derive(Debug, Clone)]
pub struct SamplingConfig {
    /// 0.0 = none, 1.0 = all. Values outside [0,1] are clamped.
    pub rate: f64,
}

impl Default for SamplingConfig {
    fn default() -> Self {
        Self { rate: 1.0 }
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn h(n: u8) -> Hash {
        let mut b = [0u8; 32];
        b[0] = n;
        Hash::from_bytes(b)
    }

    #[test]
    fn record_and_parents() {
        let dir = tempdir().unwrap();
        let mut j = LineageJournal::open(dir.path()).unwrap();
        let e = LineageEdge::new(EdgeKind::Derive, h(1), h(2), "test");
        j.record(e).unwrap();
        let parents = j.parents(&h(2)).unwrap();
        assert_eq!(parents.len(), 1);
        assert_eq!(parents[0].source_hash, h(1));
    }

    #[test]
    fn children_query() {
        let dir = tempdir().unwrap();
        let mut j = LineageJournal::open(dir.path()).unwrap();
        j.record(LineageEdge::new(EdgeKind::Copy, h(1), h(2), "a"))
            .unwrap();
        j.record(LineageEdge::new(EdgeKind::Copy, h(1), h(3), "a"))
            .unwrap();
        let children = j.children(&h(1)).unwrap();
        assert_eq!(children.len(), 2);
    }

    #[test]
    fn ancestors_bfs() {
        let dir = tempdir().unwrap();
        let mut j = LineageJournal::open(dir.path()).unwrap();
        // h(1) -> h(2) -> h(3)
        j.record(LineageEdge::new(EdgeKind::Derive, h(1), h(2), "a"))
            .unwrap();
        j.record(LineageEdge::new(EdgeKind::Derive, h(2), h(3), "a"))
            .unwrap();
        let ancestors = j.ancestors(&h(3), 10).unwrap();
        assert_eq!(ancestors.len(), 2);
    }

    #[test]
    fn descendants_bfs() {
        let dir = tempdir().unwrap();
        let mut j = LineageJournal::open(dir.path()).unwrap();
        j.record(LineageEdge::new(EdgeKind::Transform, h(1), h(2), "a"))
            .unwrap();
        j.record(LineageEdge::new(EdgeKind::Transform, h(2), h(3), "a"))
            .unwrap();
        let desc = j.descendants(&h(1), 10).unwrap();
        assert_eq!(desc.len(), 2);
    }

    #[test]
    fn sampling_control() {
        let dir = tempdir().unwrap();
        let mut j = LineageJournal::open(dir.path())
            .unwrap()
            .with_sampling(SamplingConfig { rate: 0.0 });
        for i in 0..10u8 {
            j.record_sampled(LineageEdge::new(EdgeKind::Read, h(i), h(i + 1), "a"))
                .unwrap();
        }
        // rate=0 → nothing recorded
        assert!(j.all_edges().unwrap().is_empty());
    }

    #[test]
    fn rollup_window_test() {
        let edges = vec![
            LineageEdge {
                id: "1".into(),
                ts: 1000,
                kind: EdgeKind::Read,
                source_hash: h(1),
                sink_hash: h(2),
                agent: "a".into(),
                xattrs: HashMap::new(),
            },
            LineageEdge {
                id: "2".into(),
                ts: 2000,
                kind: EdgeKind::Write,
                source_hash: h(3),
                sink_hash: h(4),
                agent: "a".into(),
                xattrs: HashMap::new(),
            },
        ];
        let buckets = rollup_window(&edges, 5); // 5-second windows
        assert!(!buckets.is_empty());
    }
}
