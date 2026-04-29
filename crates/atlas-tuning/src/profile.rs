//! Per-workload tuning profiles (T7.6).

use serde::{Deserialize, Serialize};

/// Well-known workload categories that drive different I/O patterns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WorkloadKind {
    /// Large sequential reads; typical for training data loading.
    Training,
    /// Small random reads; latency-sensitive inference serving.
    Inference,
    /// Large sequential writes; CI/CD artifact storage.
    Build,
    /// Mixed read/write; general interactive use.
    Interactive,
    /// Append-only streaming log ingestion.
    Streaming,
}

impl WorkloadKind {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Training    => "training",
            Self::Inference   => "inference",
            Self::Build       => "build",
            Self::Interactive => "interactive",
            Self::Streaming   => "streaming",
        }
    }
}

/// A tuning profile that can be applied to a volume or namespace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuningProfile {
    pub workload: WorkloadKind,
    /// Target read-ahead size in bytes (0 = disabled).
    pub read_ahead_bytes: u64,
    /// Maximum concurrent chunk-fetch operations.
    pub max_concurrent_fetches: usize,
    /// Preferred chunk size for new writes, in bytes.
    pub chunk_size_bytes: u64,
    /// Enable BLAKE3 streaming verification during reads.
    pub inline_verify: bool,
    /// Maximum write-buffer before flush, in bytes.
    pub write_buffer_bytes: u64,
    /// Replication priority (higher = more I/O priority for replication).
    pub replication_priority: u8,
    /// Cache-eviction policy hint.
    pub cache_policy: CachePolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CachePolicy {
    /// Keep recently accessed objects (default LRU).
    Lru,
    /// Keep frequently accessed objects (LFU).
    Lfu,
    /// Bypass cache for all reads.
    Bypass,
    /// Pin all accessed objects; never evict.
    Pin,
}

impl TuningProfile {
    /// Return the built-in profile for the given workload.
    pub fn for_workload(kind: WorkloadKind) -> Self {
        match kind {
            WorkloadKind::Training => Self {
                workload: kind,
                read_ahead_bytes: 64 * 1024 * 1024,
                max_concurrent_fetches: 32,
                chunk_size_bytes: 16 * 1024 * 1024,
                inline_verify: false,
                write_buffer_bytes: 256 * 1024 * 1024,
                replication_priority: 5,
                cache_policy: CachePolicy::Lru,
            },
            WorkloadKind::Inference => Self {
                workload: kind,
                read_ahead_bytes: 0,
                max_concurrent_fetches: 64,
                chunk_size_bytes: 4 * 1024 * 1024,
                inline_verify: true,
                write_buffer_bytes: 16 * 1024 * 1024,
                replication_priority: 8,
                cache_policy: CachePolicy::Pin,
            },
            WorkloadKind::Build => Self {
                workload: kind,
                read_ahead_bytes: 4 * 1024 * 1024,
                max_concurrent_fetches: 16,
                chunk_size_bytes: 8 * 1024 * 1024,
                inline_verify: true,
                write_buffer_bytes: 64 * 1024 * 1024,
                replication_priority: 3,
                cache_policy: CachePolicy::Lru,
            },
            WorkloadKind::Interactive => Self {
                workload: kind,
                read_ahead_bytes: 1 * 1024 * 1024,
                max_concurrent_fetches: 8,
                chunk_size_bytes: 4 * 1024 * 1024,
                inline_verify: true,
                write_buffer_bytes: 8 * 1024 * 1024,
                replication_priority: 5,
                cache_policy: CachePolicy::Lfu,
            },
            WorkloadKind::Streaming => Self {
                workload: kind,
                read_ahead_bytes: 0,
                max_concurrent_fetches: 4,
                chunk_size_bytes: 1 * 1024 * 1024,
                inline_verify: false,
                write_buffer_bytes: 128 * 1024 * 1024,
                replication_priority: 2,
                cache_policy: CachePolicy::Bypass,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn training_has_large_read_ahead() {
        let p = TuningProfile::for_workload(WorkloadKind::Training);
        assert!(p.read_ahead_bytes >= 16 * 1024 * 1024);
    }

    #[test]
    fn inference_pins_cache() {
        let p = TuningProfile::for_workload(WorkloadKind::Inference);
        assert_eq!(p.cache_policy, CachePolicy::Pin);
    }

    #[test]
    fn streaming_bypasses_cache() {
        let p = TuningProfile::for_workload(WorkloadKind::Streaming);
        assert_eq!(p.cache_policy, CachePolicy::Bypass);
    }
}
