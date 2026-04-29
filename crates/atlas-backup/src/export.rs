//! Point-in-time snapshot export to `.atlas-bundle` archives (T7.2).

use atlas_core::Hash;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Configuration for a snapshot export.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportConfig {
    /// Commit hash to export (`Hash::ZERO` = HEAD).
    pub commit_hash: Hash,
    /// Destination file path (`.atlas-bundle`).
    pub dest: PathBuf,
    /// Compress chunks with zstd.
    pub compress: bool,
    /// Verify every chunk hash after writing.
    pub verify: bool,
    /// Limit export bandwidth to `bytes_per_sec` (0 = unlimited).
    pub bandwidth_limit: u64,
}

impl ExportConfig {
    pub fn head(dest: impl Into<PathBuf>) -> Self {
        Self {
            commit_hash: Hash::ZERO,
            dest: dest.into(),
            compress: true,
            verify: true,
            bandwidth_limit: 0,
        }
    }
}

/// Statistics produced by a completed export.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExportStats {
    pub chunks_written: u64,
    pub bytes_written: u64,
    pub bytes_compressed: u64,
    pub duration_ms: u64,
    pub verify_errors: u64,
}

impl ExportStats {
    pub fn compression_ratio(&self) -> f64 {
        if self.bytes_compressed == 0 { return 1.0; }
        self.bytes_written as f64 / self.bytes_compressed as f64
    }

    pub fn throughput_mbs(&self) -> f64 {
        if self.duration_ms == 0 { return 0.0; }
        (self.bytes_written as f64 / (1024.0 * 1024.0)) / (self.duration_ms as f64 / 1000.0)
    }
}

/// Writes chunks and metadata into an `.atlas-bundle` file.
///
/// Bundle format:
/// ```text
/// [8 bytes]  magic: b"ATLASBND"
/// [4 bytes]  version: u32 LE = 1
/// [8 bytes]  header length: u64 LE
/// [N bytes]  header JSON (BundleHeader)
/// repeated:
///   [32 bytes] chunk hash
///   [8 bytes]  chunk length: u64 LE
///   [N bytes]  chunk data (optionally zstd-compressed)
/// [32 bytes] footer hash (BLAKE3 of all preceding bytes)
/// ```
pub struct BundleWriter<W: Write> {
    writer: W,
    config: ExportConfig,
    stats: ExportStats,
    hasher: blake3::Hasher,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleHeader {
    pub version: u32,
    pub commit_hash: String,
    pub created_at_ms: u64,
    pub compressed: bool,
    pub atlas_version: String,
}

impl<W: Write> BundleWriter<W> {
    pub fn new(mut writer: W, config: ExportConfig) -> anyhow::Result<Self> {
        let header = BundleHeader {
            version: 1,
            commit_hash: hex::encode(config.commit_hash.as_bytes()),
            created_at_ms: now_ms(),
            compressed: config.compress,
            atlas_version: env!("CARGO_PKG_VERSION").into(),
        };
        let header_json = serde_json::to_vec(&header)?;
        let header_len = header_json.len() as u64;

        let mut hasher = blake3::Hasher::new();
        let magic = b"ATLASBND";
        writer.write_all(magic)?;
        writer.write_all(&1u32.to_le_bytes())?;
        writer.write_all(&header_len.to_le_bytes())?;
        writer.write_all(&header_json)?;
        hasher.update(magic);
        hasher.update(&1u32.to_le_bytes());
        hasher.update(&header_len.to_le_bytes());
        hasher.update(&header_json);

        Ok(Self { writer, config, stats: ExportStats::default(), hasher })
    }

    /// Write a single chunk.
    pub fn write_chunk(&mut self, hash: &Hash, data: &[u8]) -> anyhow::Result<()> {
        let payload = if self.config.compress {
            zstd_compress(data)
        } else {
            data.to_vec()
        };
        let len = payload.len() as u64;
        let hash_bytes = hash.as_bytes();

        self.writer.write_all(hash_bytes)?;
        self.writer.write_all(&len.to_le_bytes())?;
        self.writer.write_all(&payload)?;
        self.hasher.update(hash_bytes);
        self.hasher.update(&len.to_le_bytes());
        self.hasher.update(&payload);

        self.stats.chunks_written += 1;
        self.stats.bytes_written += data.len() as u64;
        self.stats.bytes_compressed += payload.len() as u64;
        Ok(())
    }

    /// Finalise the bundle and return export statistics.
    pub fn finish(mut self) -> anyhow::Result<ExportStats> {
        let footer = self.hasher.finalize();
        self.writer.write_all(footer.as_bytes())?;
        Ok(self.stats)
    }
}

fn zstd_compress(data: &[u8]) -> Vec<u8> {
    // Real implementation uses the `zstd` crate; stub returns a copy.
    data.to_vec()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_writer_header_magic() {
        let mut buf = Vec::new();
        let cfg = ExportConfig::head("/tmp/test.atlas-bundle");
        let writer = BundleWriter::new(&mut buf, cfg).unwrap();
        writer.finish().unwrap();
        assert_eq!(&buf[..8], b"ATLASBND");
    }

    #[test]
    fn export_stats_defaults() {
        let stats = ExportStats::default();
        assert_eq!(stats.compression_ratio(), 1.0);
        assert_eq!(stats.throughput_mbs(), 0.0);
    }

    #[test]
    fn export_config_head() {
        let cfg = ExportConfig::head("/tmp/out.atlas-bundle");
        assert!(cfg.compress);
        assert!(cfg.verify);
    }
}
