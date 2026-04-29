//! Fault definitions for the chaos framework (T7.1).

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Which subsystem a fault targets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FaultTarget {
    /// A specific storage node (by index in the cluster).
    StorageNode(usize),
    /// The metadata plane (sled / FoundationDB).
    Metadata,
    /// The network path between two node indices.
    Network { from: usize, to: usize },
    /// The chunk GC daemon.
    GarbageCollector,
    /// The replication chain head.
    ReplicationHead,
}

/// What kind of fault to inject.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FaultKind {
    /// Kill a process / drop all in-flight ops; restart after `restart_delay`.
    Crash { restart_delay: Duration },
    /// Pause the target (SIGSTOP); resume after `pause_duration`.
    Pause { pause_duration: Duration },
    /// Drop a fraction of packets on a network path.
    PacketLoss { rate: f64 },
    /// Add latency to every operation.
    Latency { added_ms: u64, jitter_ms: u64 },
    /// Corrupt a random byte in every `nth` write.
    BitFlip { nth: u64 },
    /// Fill the data disk to `capacity_pct`% (0–100).
    DiskFull { capacity_pct: u8 },
    /// Return an I/O error on every `nth` read.
    IoError { nth: u64 },
    /// Split the cluster so `left_nodes` cannot reach `right_nodes`.
    NetworkPartition { left_nodes: Vec<usize>, right_nodes: Vec<usize> },
    /// Slow clock on the target node by `drift_ms` per second.
    ClockSkew { drift_ms: i64 },
}

/// A single fault with its activation schedule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fault {
    pub target: FaultTarget,
    pub kind: FaultKind,
    /// Delay before injection begins.
    pub start_after: Duration,
    /// How long the fault stays active (`None` = until end of scenario).
    pub duration: Option<Duration>,
}

impl Fault {
    pub fn crash_node(node: usize, after: Duration, restart_after: Duration) -> Self {
        Self {
            target: FaultTarget::StorageNode(node),
            kind: FaultKind::Crash { restart_delay: restart_after },
            start_after: after,
            duration: None,
        }
    }

    pub fn packet_loss(from: usize, to: usize, rate: f64, after: Duration, dur: Duration) -> Self {
        Self {
            target: FaultTarget::Network { from, to },
            kind: FaultKind::PacketLoss { rate },
            start_after: after,
            duration: Some(dur),
        }
    }

    pub fn latency(node: usize, added_ms: u64, jitter_ms: u64) -> Self {
        Self {
            target: FaultTarget::StorageNode(node),
            kind: FaultKind::Latency { added_ms, jitter_ms },
            start_after: Duration::ZERO,
            duration: None,
        }
    }

    pub fn partition(left: Vec<usize>, right: Vec<usize>, after: Duration, dur: Duration) -> Self {
        Self {
            target: FaultTarget::Network { from: 0, to: 0 },
            kind: FaultKind::NetworkPartition { left_nodes: left, right_nodes: right },
            start_after: after,
            duration: Some(dur),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crash_fault_round_trips() {
        let f = Fault::crash_node(0, Duration::from_secs(5), Duration::from_secs(2));
        let json = serde_json::to_string(&f).unwrap();
        let back: Fault = serde_json::from_str(&json).unwrap();
        assert!(matches!(back.kind, FaultKind::Crash { .. }));
    }

    #[test]
    fn packet_loss_rate_stored() {
        let f = Fault::packet_loss(0, 1, 0.3, Duration::ZERO, Duration::from_secs(10));
        assert!(matches!(f.kind, FaultKind::PacketLoss { rate } if (rate - 0.3).abs() < 1e-9));
    }
}
