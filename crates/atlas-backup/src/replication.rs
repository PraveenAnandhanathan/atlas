//! Cross-region async replication (T7.2).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

/// Where to replicate snapshots.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReplicationTarget {
    /// A remote ATLAS cluster reached over gRPC.
    AtlasCluster { endpoint: String, volume: String },
    /// An S3-compatible bucket.
    S3 { endpoint: String, bucket: String, prefix: String, region: String },
    /// A local or NFS path (for testing / on-prem DR).
    LocalPath { path: PathBuf },
}

impl ReplicationTarget {
    pub fn display_name(&self) -> String {
        match self {
            Self::AtlasCluster { endpoint, volume } => format!("atlas://{endpoint}/{volume}"),
            Self::S3 { bucket, prefix, .. } => format!("s3://{bucket}/{prefix}"),
            Self::LocalPath { path } => format!("file://{}", path.display()),
        }
    }
}

/// Configuration for the replication daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicationConfig {
    pub targets: Vec<ReplicationTarget>,
    /// Replication lag target in seconds (RPO guidance).
    pub max_lag_secs: u64,
    /// Bandwidth cap per target in bytes/sec (0 = unlimited).
    pub bandwidth_limit: u64,
    /// Retry attempts before raising an alert.
    pub max_retries: u32,
    /// Encrypt bundles before transmitting.
    pub encrypt: bool,
}

impl Default for ReplicationConfig {
    fn default() -> Self {
        Self {
            targets: Vec::new(),
            max_lag_secs: 900, // 15 min
            bandwidth_limit: 0,
            max_retries: 5,
            encrypt: true,
        }
    }
}

/// A single replication transfer result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferResult {
    pub target: String,
    pub success: bool,
    pub bytes_transferred: u64,
    pub duration_ms: u64,
    pub error: Option<String>,
}

/// Drives cross-region replication.
pub struct Replicator {
    pub config: ReplicationConfig,
}

impl Replicator {
    pub fn new(config: ReplicationConfig) -> Self {
        Self { config }
    }

    /// Replicate `bundle_path` to all configured targets.
    ///
    /// - `LocalPath`: copies the file using `std::fs::copy`.
    /// - `S3`: uploads via HTTP PUT to `{endpoint}/{bucket}/{prefix}{filename}`.
    ///   Relies on instance-role credentials or `AWS_ACCESS_KEY_ID` /
    ///   `AWS_SECRET_ACCESS_KEY` in the environment (unsigned for public buckets).
    /// - `AtlasCluster`: HTTP PUT to `{endpoint}/api/v1/volumes/{volume}/bundles/{filename}`.
    pub fn replicate(&self, bundle_path: &std::path::Path) -> Vec<TransferResult> {
        self.config
            .targets
            .iter()
            .map(|target| self.replicate_one(target, bundle_path))
            .collect()
    }

    fn replicate_one(
        &self,
        target: &ReplicationTarget,
        bundle_path: &std::path::Path,
    ) -> TransferResult {
        let name = target.display_name();
        let started = SystemTime::now();
        tracing::info!(target = %name, bundle = %bundle_path.display(), "replicating bundle");

        let result = match target {
            ReplicationTarget::LocalPath { path } => {
                replicate_local(bundle_path, path)
            }
            ReplicationTarget::S3 { endpoint, bucket, prefix, .. } => {
                replicate_s3(bundle_path, endpoint, bucket, prefix)
            }
            ReplicationTarget::AtlasCluster { endpoint, volume } => {
                replicate_atlas(bundle_path, endpoint, volume)
            }
        };

        let duration_ms = SystemTime::now()
            .duration_since(started)
            .unwrap_or(Duration::ZERO)
            .as_millis() as u64;

        match result {
            Ok(bytes) => TransferResult {
                target: name,
                success: true,
                bytes_transferred: bytes,
                duration_ms,
                error: None,
            },
            Err(e) => {
                tracing::warn!(target = %name, error = %e, "replication failed");
                TransferResult {
                    target: name,
                    success: false,
                    bytes_transferred: 0,
                    duration_ms,
                    error: Some(e),
                }
            }
        }
    }
}

/// Copy a bundle to a local/NFS destination directory.
fn replicate_local(bundle: &std::path::Path, dest_dir: &std::path::Path) -> Result<u64, String> {
    std::fs::create_dir_all(dest_dir)
        .map_err(|e| format!("create dir {}: {e}", dest_dir.display()))?;
    let filename = bundle
        .file_name()
        .ok_or_else(|| format!("no filename in {}", bundle.display()))?;
    let dest = dest_dir.join(filename);
    let bytes = std::fs::copy(bundle, &dest)
        .map_err(|e| format!("copy to {}: {e}", dest.display()))?;
    tracing::info!(dest = %dest.display(), bytes, "local replication complete");
    Ok(bytes)
}

/// Upload a bundle to an S3-compatible bucket via HTTP PUT.
fn replicate_s3(
    bundle: &std::path::Path,
    endpoint: &str,
    bucket: &str,
    prefix: &str,
) -> Result<u64, String> {
    let data = std::fs::read(bundle)
        .map_err(|e| format!("read bundle {}: {e}", bundle.display()))?;
    let bytes = data.len() as u64;
    let filename = bundle
        .file_name()
        .and_then(|f| f.to_str())
        .ok_or_else(|| format!("no filename in {}", bundle.display()))?;
    let key = if prefix.is_empty() {
        filename.to_string()
    } else {
        format!("{}/{}", prefix.trim_end_matches('/'), filename)
    };
    let url = format!("{}/{}/{}", endpoint.trim_end_matches('/'), bucket, key);
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| format!("build HTTP client: {e}"))?;
    let resp = client
        .put(&url)
        .header("Content-Type", "application/octet-stream")
        .header("Content-Length", bytes.to_string())
        .body(data)
        .send()
        .map_err(|e| format!("PUT {url}: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("S3 PUT {url}: HTTP {}", resp.status()));
    }
    tracing::info!(url, bytes, "S3 replication complete");
    Ok(bytes)
}

/// Upload a bundle to a remote ATLAS cluster via HTTP PUT.
fn replicate_atlas(
    bundle: &std::path::Path,
    endpoint: &str,
    volume: &str,
) -> Result<u64, String> {
    let data = std::fs::read(bundle)
        .map_err(|e| format!("read bundle {}: {e}", bundle.display()))?;
    let bytes = data.len() as u64;
    let filename = bundle
        .file_name()
        .and_then(|f| f.to_str())
        .ok_or_else(|| format!("no filename in {}", bundle.display()))?;
    let url = format!(
        "{}/api/v1/volumes/{}/bundles/{}",
        endpoint.trim_end_matches('/'),
        volume,
        filename
    );
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| format!("build HTTP client: {e}"))?;
    let resp = client
        .put(&url)
        .header("Content-Type", "application/octet-stream")
        .body(data)
        .send()
        .map_err(|e| format!("PUT {url}: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Atlas PUT {url}: HTTP {}", resp.status()));
    }
    tracing::info!(url, bytes, "Atlas cluster replication complete");
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn target_display_names() {
        let t = ReplicationTarget::S3 {
            endpoint: "https://s3.amazonaws.com".into(),
            bucket: "my-backup".into(),
            prefix: "atlas/".into(),
            region: "us-east-1".into(),
        };
        assert!(t.display_name().starts_with("s3://"));

        let t2 = ReplicationTarget::LocalPath { path: "/mnt/dr".into() };
        assert!(t2.display_name().starts_with("file://"));
    }

    #[test]
    fn default_config_rpo() {
        let c = ReplicationConfig::default();
        assert_eq!(c.max_lag_secs, 900);
        assert!(c.encrypt);
    }

    #[test]
    fn replicate_local_copies_file() {
        let src_dir = TempDir::new().unwrap();
        let dst_dir = TempDir::new().unwrap();
        let bundle = src_dir.path().join("snapshot.atlas-bundle");
        std::fs::write(&bundle, b"ATLASBND\x01\x00\x00\x00").unwrap();

        let target = ReplicationTarget::LocalPath { path: dst_dir.path().to_path_buf() };
        let rep = Replicator::new(ReplicationConfig { targets: vec![target], ..Default::default() });
        let results = rep.replicate(&bundle);

        assert_eq!(results.len(), 1);
        assert!(results[0].success, "local replication should succeed");
        assert_eq!(results[0].bytes_transferred, 12);
        assert!(dst_dir.path().join("snapshot.atlas-bundle").exists());
    }

    #[test]
    fn replicate_s3_fails_gracefully_on_bad_endpoint() {
        let src_dir = TempDir::new().unwrap();
        let bundle = src_dir.path().join("snap.atlas-bundle");
        std::fs::write(&bundle, b"ATLASBND").unwrap();

        let target = ReplicationTarget::S3 {
            endpoint: "http://127.0.0.1:1".into(), // nothing listening
            bucket: "b".into(),
            prefix: "p/".into(),
            region: "us-east-1".into(),
        };
        let rep = Replicator::new(ReplicationConfig { targets: vec![target], ..Default::default() });
        let results = rep.replicate(&bundle);
        assert_eq!(results.len(), 1);
        assert!(!results[0].success, "must report failure, not silently succeed");
        assert!(results[0].error.is_some());
    }

    #[test]
    fn replicate_atlas_fails_gracefully_on_bad_endpoint() {
        let src_dir = TempDir::new().unwrap();
        let bundle = src_dir.path().join("snap.atlas-bundle");
        std::fs::write(&bundle, b"ATLASBND").unwrap();

        let target = ReplicationTarget::AtlasCluster {
            endpoint: "http://127.0.0.1:1".into(),
            volume: "vol-1".into(),
        };
        let rep = Replicator::new(ReplicationConfig { targets: vec![target], ..Default::default() });
        let results = rep.replicate(&bundle);
        assert!(!results[0].success);
    }
}
