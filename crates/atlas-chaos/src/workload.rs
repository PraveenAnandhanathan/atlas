//! Workload drivers for chaos experiments (T7.1).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkloadKind {
    SequentialReadWrite { size_mb: u64 },
    RandomRead { file_count: usize, file_size_mb: u64 },
    WriteHeavy { size_mb: u64 },
    Mixed { read_pct: u8 },
    MetadataStorm { op_count: u64 },
    CheckpointDelta { base_size_mb: u64, delta_pct: u8 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workload {
    pub kind: WorkloadKind,
    pub parallelism: usize,
}

impl Workload {
    /// Estimate total bytes touched by this workload (for progress reporting).
    pub fn approximate_bytes(&self) -> u64 {
        match &self.kind {
            WorkloadKind::SequentialReadWrite { size_mb } => size_mb * 1024 * 1024,
            WorkloadKind::RandomRead { file_count, file_size_mb } => {
                *file_count as u64 * file_size_mb * 1024 * 1024
            }
            WorkloadKind::WriteHeavy { size_mb } => size_mb * 1024 * 1024,
            WorkloadKind::Mixed { .. } => 512 * 1024 * 1024,
            WorkloadKind::MetadataStorm { op_count } => op_count * 256,
            WorkloadKind::CheckpointDelta { base_size_mb, delta_pct } => {
                base_size_mb * 1024 * 1024 * (*delta_pct as u64) / 100
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approximate_bytes_sequential() {
        let w = Workload { kind: WorkloadKind::SequentialReadWrite { size_mb: 1024 }, parallelism: 4 };
        assert_eq!(w.approximate_bytes(), 1024 * 1024 * 1024);
    }
}
