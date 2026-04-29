//! Apply a tuning profile to a volume or namespace (T7.6).

use crate::profile::{TuningProfile, WorkloadKind};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Applied tuning state for a set of volume/namespace targets.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TunerState {
    profiles: HashMap<String, TuningProfile>,
}

impl TunerState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the profile for a named volume or namespace.
    pub fn apply(&mut self, target: impl Into<String>, kind: WorkloadKind) -> &TuningProfile {
        let profile = TuningProfile::for_workload(kind);
        let key = target.into();
        self.profiles.insert(key.clone(), profile);
        &self.profiles[&key]
    }

    pub fn get(&self, target: &str) -> Option<&TuningProfile> {
        self.profiles.get(target)
    }

    pub fn remove(&mut self, target: &str) -> bool {
        self.profiles.remove(target).is_some()
    }

    pub fn list(&self) -> Vec<(&str, &TuningProfile)> {
        self.profiles.iter().map(|(k, v)| (k.as_str(), v)).collect()
    }
}

/// Recommend a workload profile based on observed I/O statistics.
pub fn recommend(read_bytes: u64, write_bytes: u64, avg_object_size: u64) -> WorkloadKind {
    let total = read_bytes + write_bytes;
    if total == 0 {
        return WorkloadKind::Interactive;
    }
    let read_ratio = read_bytes as f64 / total as f64;
    if write_bytes > read_bytes && avg_object_size < 2 * 1024 * 1024 {
        WorkloadKind::Streaming
    } else if read_ratio >= 0.9 && avg_object_size > 100 * 1024 * 1024 {
        WorkloadKind::Training
    } else if read_ratio >= 0.9 && avg_object_size < 10 * 1024 * 1024 {
        WorkloadKind::Inference
    } else if write_bytes > read_bytes {
        WorkloadKind::Build
    } else {
        WorkloadKind::Interactive
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_and_get() {
        let mut state = TunerState::new();
        state.apply("vol-1", WorkloadKind::Training);
        let p = state.get("vol-1").unwrap();
        assert_eq!(p.workload, WorkloadKind::Training);
    }

    #[test]
    fn recommend_training_for_large_reads() {
        let kind = recommend(900, 100, 200 * 1024 * 1024);
        assert_eq!(kind, WorkloadKind::Training);
    }

    #[test]
    fn recommend_inference_for_small_reads() {
        let kind = recommend(9000, 1000, 1 * 1024 * 1024);
        assert_eq!(kind, WorkloadKind::Inference);
    }
}
