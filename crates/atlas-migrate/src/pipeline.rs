//! Migration pipeline — transfers objects from a source into ATLAS (T7.8).

use crate::source::{enumerate, MigrationSource, SourceObject};
use serde::{Deserialize, Serialize};

/// Configuration for a migration run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationConfig {
    pub source: MigrationSource,
    /// Target ATLAS volume name.
    pub target_volume: String,
    /// Maximum concurrent transfers (0 = sequential).
    pub concurrency: usize,
    /// Skip objects already present in ATLAS (by content hash).
    pub skip_existing: bool,
    /// Verify BLAKE3 hash after transfer.
    pub verify: bool,
}

impl Default for MigrationConfig {
    fn default() -> Self {
        Self {
            source: MigrationSource::Ext4 { mount_point: "/".into(), sub_path: "/".into() },
            target_volume: "default".into(),
            concurrency: 8,
            skip_existing: true,
            verify: true,
        }
    }
}

/// Outcome for a single object transfer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferResult {
    pub path: String,
    pub bytes: u64,
    pub skipped: bool,
    pub error: Option<String>,
}

impl TransferResult {
    pub fn ok(obj: &SourceObject) -> Self {
        Self { path: obj.path.clone(), bytes: obj.size, skipped: false, error: None }
    }
    pub fn skipped(obj: &SourceObject) -> Self {
        Self { path: obj.path.clone(), bytes: 0, skipped: true, error: None }
    }
    pub fn failed(obj: &SourceObject, msg: impl Into<String>) -> Self {
        Self { path: obj.path.clone(), bytes: 0, skipped: false, error: Some(msg.into()) }
    }
}

/// Aggregate statistics for the migration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MigrationStats {
    pub objects_total: usize,
    pub objects_transferred: usize,
    pub objects_skipped: usize,
    pub objects_failed: usize,
    pub bytes_transferred: u64,
}

impl MigrationStats {
    pub fn record(&mut self, r: &TransferResult) {
        self.objects_total += 1;
        if r.skipped {
            self.objects_skipped += 1;
        } else if r.error.is_some() {
            self.objects_failed += 1;
        } else {
            self.objects_transferred += 1;
            self.bytes_transferred += r.bytes;
        }
    }

    pub fn success_rate(&self) -> f64 {
        if self.objects_total == 0 { return 1.0; }
        (self.objects_transferred + self.objects_skipped) as f64 / self.objects_total as f64
    }
}

/// Run a migration (stub implementation — real version would stream objects in
/// parallel workers, content-address chunks via `atlas_chunk`, and commit via
/// `atlas_fs::Fs::write()`).
pub fn run(config: &MigrationConfig) -> (Vec<TransferResult>, MigrationStats) {
    let objects = enumerate(&config.source, 1_000);
    let mut results = Vec::with_capacity(objects.len());
    let mut stats = MigrationStats::default();

    for obj in &objects {
        let r = TransferResult::ok(obj);
        stats.record(&r);
        results.push(r);
    }

    (results, stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::parse_source;

    #[test]
    fn migration_run_succeeds() {
        let config = MigrationConfig {
            source: parse_source("s3://bucket/prefix").unwrap(),
            target_volume: "vol-1".into(),
            concurrency: 4,
            skip_existing: false,
            verify: true,
        };
        let (_results, stats) = run(&config);
        assert!(stats.objects_total > 0);
        assert_eq!(stats.objects_failed, 0);
        assert!((stats.success_rate() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn stats_record_skipped() {
        let mut stats = MigrationStats::default();
        let obj = SourceObject { path: "a".into(), size: 100, source_id: "x".into() };
        stats.record(&TransferResult::skipped(&obj));
        assert_eq!(stats.objects_skipped, 1);
        assert_eq!(stats.bytes_transferred, 0);
    }
}
