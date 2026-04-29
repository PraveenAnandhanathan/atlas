//! Pre-built chaos scenarios (T7.1).

use crate::{Fault, Invariant, InvariantKind, Workload, WorkloadKind};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// A fully specified chaos experiment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChaosScenario {
    pub name: String,
    pub description: String,
    /// Total wall-clock budget for the run.
    pub duration: Duration,
    pub faults: Vec<Fault>,
    pub workload: Workload,
    pub invariants: Vec<Invariant>,
}

impl ChaosScenario {
    /// Single-node crash-recovery: write data, crash node 0, verify reads
    /// still work after restart.
    pub fn single_node_crash() -> Self {
        Self {
            name: "single_node_crash".into(),
            description: "Write 1 GiB, crash the storage node at T+10s, verify data integrity after restart.".into(),
            duration: Duration::from_secs(120),
            faults: vec![
                Fault::crash_node(0, Duration::from_secs(10), Duration::from_secs(5)),
            ],
            workload: Workload { kind: WorkloadKind::SequentialReadWrite { size_mb: 1024 }, parallelism: 4 },
            invariants: vec![
                Invariant { kind: InvariantKind::DataIntegrity },
                Invariant { kind: InvariantKind::NoCorruptChunks },
            ],
        }
    }

    /// Network partition: split a 3-node cluster for 30s, verify
    /// strong consistency is maintained.
    pub fn network_partition_3node() -> Self {
        Self {
            name: "network_partition_3node".into(),
            description: "Partition nodes [0] from [1,2] for 30s during mixed read/write workload.".into(),
            duration: Duration::from_secs(90),
            faults: vec![
                Fault::partition(vec![0], vec![1, 2], Duration::from_secs(10), Duration::from_secs(30)),
            ],
            workload: Workload { kind: WorkloadKind::Mixed { read_pct: 70 }, parallelism: 8 },
            invariants: vec![
                Invariant { kind: InvariantKind::DataIntegrity },
                Invariant { kind: InvariantKind::NoSplitBrain },
                Invariant { kind: InvariantKind::ReplicationFactorMaintained { min_replicas: 2 } },
            ],
        }
    }

    /// Rolling restart: cycle through all nodes one at a time while
    /// traffic flows, verifying zero downtime.
    pub fn rolling_restart(node_count: usize) -> Self {
        let faults: Vec<Fault> = (0..node_count)
            .map(|i| Fault::crash_node(
                i,
                Duration::from_secs(10 + (i as u64) * 20),
                Duration::from_secs(5),
            ))
            .collect();
        Self {
            name: "rolling_restart".into(),
            description: format!("Rolling restart across {node_count} nodes with continuous traffic."),
            duration: Duration::from_secs(60 + node_count as u64 * 25),
            faults,
            workload: Workload { kind: WorkloadKind::SequentialReadWrite { size_mb: 256 }, parallelism: 4 },
            invariants: vec![
                Invariant { kind: InvariantKind::DataIntegrity },
                Invariant { kind: InvariantKind::NoCorruptChunks },
            ],
        }
    }

    /// Disk-full on one node: fill node 0 to 95%, verify placement
    /// redirects new writes to other nodes.
    pub fn disk_full_failover() -> Self {
        Self {
            name: "disk_full_failover".into(),
            description: "Fill node 0 disk to 95%; new writes must land on other nodes.".into(),
            duration: Duration::from_secs(60),
            faults: vec![Fault {
                target: crate::FaultTarget::StorageNode(0),
                kind: crate::FaultKind::DiskFull { capacity_pct: 95 },
                start_after: Duration::from_secs(5),
                duration: None,
            }],
            workload: Workload { kind: WorkloadKind::WriteHeavy { size_mb: 512 }, parallelism: 2 },
            invariants: vec![
                Invariant { kind: InvariantKind::WritesSucceed },
                Invariant { kind: InvariantKind::DataIntegrity },
            ],
        }
    }

    /// Bit-flip corruption on every 1000th write; verify integrity checks catch it.
    pub fn bit_flip_detection() -> Self {
        Self {
            name: "bit_flip_detection".into(),
            description: "Inject a bit-flip every 1000 writes; verify BLAKE3 detects all corruptions.".into(),
            duration: Duration::from_secs(60),
            faults: vec![Fault {
                target: crate::FaultTarget::StorageNode(0),
                kind: crate::FaultKind::BitFlip { nth: 1000 },
                start_after: Duration::ZERO,
                duration: None,
            }],
            workload: Workload { kind: WorkloadKind::WriteHeavy { size_mb: 128 }, parallelism: 1 },
            invariants: vec![
                Invariant { kind: InvariantKind::NoSilentCorruption },
            ],
        }
    }

    /// All built-in scenarios for nightly CI.
    pub fn nightly_suite(node_count: usize) -> Vec<Self> {
        vec![
            Self::single_node_crash(),
            Self::network_partition_3node(),
            Self::rolling_restart(node_count),
            Self::disk_full_failover(),
            Self::bit_flip_detection(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nightly_suite_non_empty() {
        assert!(!ChaosScenario::nightly_suite(3).is_empty());
    }

    #[test]
    fn rolling_restart_fault_count_matches_nodes() {
        let s = ChaosScenario::rolling_restart(5);
        assert_eq!(s.faults.len(), 5);
    }

    #[test]
    fn scenarios_serialize() {
        for s in ChaosScenario::nightly_suite(3) {
            let _ = serde_json::to_string(&s).unwrap();
        }
    }
}
