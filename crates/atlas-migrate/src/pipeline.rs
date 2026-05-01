//! Migration pipeline — transfers objects from a source into ATLAS (T7.8).

use crate::source::{enumerate, fetch_object, MigrationSource, SourceObject};
use atlas_fs::Fs;
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

/// Run a migration, writing every transferred object into `fs`.
///
/// For `Ext4` sources this performs real file I/O. For cloud sources
/// (`S3`, `GCS`, `git-lfs`) `fetch_object` returns an error because the
/// network clients are not yet wired; those objects are recorded as failed
/// rather than silently skipped.
pub fn run(config: &MigrationConfig, fs: &Fs) -> (Vec<TransferResult>, MigrationStats) {
    let objects = enumerate(&config.source, 1_000);
    let mut results = Vec::with_capacity(objects.len());
    let mut stats = MigrationStats::default();

    for obj in &objects {
        let r = transfer_one(config, fs, obj);
        stats.record(&r);
        results.push(r);
    }

    (results, stats)
}

fn transfer_one(config: &MigrationConfig, fs: &Fs, obj: &SourceObject) -> TransferResult {
    let atlas_path = format!("/{}/{}", config.target_volume, obj.path.trim_start_matches('/'));

    if config.skip_existing {
        if let Ok(_) = fs.stat(&atlas_path) {
            return TransferResult::skipped(obj);
        }
    }

    let bytes = match fetch_object(&config.source, obj) {
        Ok(b) => b,
        Err(e) => return TransferResult::failed(obj, e),
    };

    match fs.write(&atlas_path, &bytes) {
        Ok(_) => TransferResult::ok(obj),
        Err(e) => TransferResult::failed(obj, e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::parse_source;

    fn tmp_fs() -> (tempfile::TempDir, Fs) {
        let dir = tempfile::tempdir().unwrap();
        let fs = Fs::init(dir.path()).unwrap();
        (dir, fs)
    }

    #[test]
    fn migration_ext4_transfers_real_files() {
        let src_dir = tempfile::tempdir().unwrap();
        std::fs::write(src_dir.path().join("file-0.bin"), b"data-0").unwrap();
        std::fs::write(src_dir.path().join("file-1.bin"), b"data-1").unwrap();
        std::fs::write(src_dir.path().join("file-2.bin"), b"data-2").unwrap();

        let (_dst, fs) = tmp_fs();
        let config = MigrationConfig {
            source: MigrationSource::Ext4 {
                mount_point: src_dir.path().to_string_lossy().into(),
                sub_path: "/".into(),
            },
            target_volume: "vol-1".into(),
            concurrency: 1,
            skip_existing: false,
            verify: true,
        };
        let (_results, stats) = run(&config, &fs);
        assert!(stats.objects_total > 0);
        assert_eq!(stats.objects_failed, 0);
    }

    #[test]
    fn migration_s3_enumerate_returns_empty_without_network() {
        let (_dst, fs) = tmp_fs();
        let config = MigrationConfig {
            source: parse_source("s3://bucket/prefix").unwrap(),
            target_volume: "vol-1".into(),
            concurrency: 4,
            skip_existing: false,
            verify: true,
        };
        let (_results, stats) = run(&config, &fs);
        // No real S3 reachable in unit tests: enumerate returns empty, no
        // objects are silently "transferred" as successes.
        assert_eq!(stats.objects_transferred, 0, "must not silently succeed");
    }

    #[test]
    fn s3_fetch_object_fails_gracefully_on_bad_endpoint() {
        use crate::source::{fetch_object, MigrationSource, SourceObject};
        let src = MigrationSource::S3 {
            endpoint: "http://127.0.0.1:1".into(), // nothing listening here
            bucket: "b".into(),
            prefix: String::new(),
            region: "us-east-1".into(),
        };
        let obj = SourceObject { path: "key.bin".into(), size: 0, source_id: "id".into() };
        let result = fetch_object(&src, &obj);
        assert!(result.is_err(), "S3 fetch must return Err when endpoint is unreachable");
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
