//! Safety invariants checked throughout a chaos run (T7.1).

use serde::{Deserialize, Serialize};

/// A property that must hold at all times during the experiment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invariant {
    pub kind: InvariantKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvariantKind {
    /// Every written chunk can be read back and passes its BLAKE3 hash check.
    DataIntegrity,
    /// No chunk stored on disk has a hash mismatch.
    NoCorruptChunks,
    /// No two nodes believe they are authoritative for conflicting data
    /// (detects split-brain in the replication layer).
    NoSplitBrain,
    /// Every object is present on at least `min_replicas` nodes.
    ReplicationFactorMaintained { min_replicas: usize },
    /// All client write calls return Ok (no silent drops under faults).
    WritesSucceed,
    /// Any corruption introduced by a fault is detected (not silently stored).
    NoSilentCorruption,
    /// Metadata operations remain linearisable (checked via a causal history log).
    MetadataLinearisable,
    /// The GC never deletes a chunk that is still referenced.
    GcSafety,
}

impl InvariantKind {
    pub fn description(&self) -> &'static str {
        match self {
            Self::DataIntegrity => "all reads return the last written value with correct hash",
            Self::NoCorruptChunks => "no stored chunk fails its hash verification",
            Self::NoSplitBrain => "no conflicting authoritative state across nodes",
            Self::ReplicationFactorMaintained { .. } => "replication factor >= minimum at all times",
            Self::WritesSucceed => "all client writes return success",
            Self::NoSilentCorruption => "all corruption is detected before acknowledgement",
            Self::MetadataLinearisable => "metadata ops form a valid linearisation",
            Self::GcSafety => "GC never deletes a live chunk",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_variants_have_descriptions() {
        let variants = [
            InvariantKind::DataIntegrity,
            InvariantKind::NoCorruptChunks,
            InvariantKind::NoSplitBrain,
            InvariantKind::ReplicationFactorMaintained { min_replicas: 2 },
            InvariantKind::WritesSucceed,
            InvariantKind::NoSilentCorruption,
            InvariantKind::MetadataLinearisable,
            InvariantKind::GcSafety,
        ];
        for v in &variants {
            assert!(!v.description().is_empty());
        }
    }
}
