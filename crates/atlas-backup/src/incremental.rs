//! Incremental backup chains (T7.2).

use atlas_core::Hash;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;

/// A manifest describing one incremental backup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupManifest {
    pub id: String,
    pub parent_id: Option<String>,
    pub commit_hash: Hash,
    pub created_at_ms: u64,
    pub chunk_count: u64,
    pub byte_count: u64,
    pub bundle_path: PathBuf,
    /// Hashes of chunks present in this bundle (deduplicated vs parent).
    pub new_chunk_hashes: Vec<Hash>,
}

impl BackupManifest {
    pub fn is_full(&self) -> bool {
        self.parent_id.is_none()
    }
}

/// A chain of incremental backup manifests.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BackupChain {
    pub manifests: Vec<BackupManifest>,
}

impl BackupChain {
    /// Return the most recent manifest.
    pub fn latest(&self) -> Option<&BackupManifest> {
        self.manifests.last()
    }

    /// Hashes that are already present in any manifest and need not be
    /// re-exported by the next incremental backup.
    pub fn known_chunks(&self) -> HashSet<Hash> {
        self.manifests
            .iter()
            .flat_map(|m| m.new_chunk_hashes.iter().copied())
            .collect()
    }

    /// Total storage consumed by this chain.
    pub fn total_bytes(&self) -> u64 {
        self.manifests.iter().map(|m| m.byte_count).sum()
    }

    /// Number of full backups in the chain.
    pub fn full_count(&self) -> usize {
        self.manifests.iter().filter(|m| m.is_full()).count()
    }
}

/// Drives an incremental backup run.
pub struct IncrementalBackup {
    pub chain: BackupChain,
    pub dest_dir: PathBuf,
}

impl IncrementalBackup {
    pub fn new(dest_dir: impl Into<PathBuf>) -> Self {
        Self { chain: BackupChain::default(), dest_dir: dest_dir.into() }
    }

    /// Compute which chunks need to be exported for `commit_hash` given
    /// what the chain already knows about.
    pub fn new_chunks<'a>(
        &self,
        all_chunks: &'a [Hash],
    ) -> Vec<&'a Hash> {
        let known = self.chain.known_chunks();
        all_chunks.iter().filter(|h| !known.contains(h)).collect()
    }

    /// Record a completed incremental export.
    pub fn record(
        &mut self,
        commit_hash: Hash,
        new_chunks: Vec<Hash>,
        byte_count: u64,
        bundle_path: PathBuf,
    ) {
        let parent_id = self.chain.latest().map(|m| m.id.clone());
        let id = format!("backup-{}", now_ms());
        self.chain.manifests.push(BackupManifest {
            id,
            parent_id,
            commit_hash,
            created_at_ms: now_ms(),
            chunk_count: new_chunks.len() as u64,
            byte_count,
            bundle_path,
            new_chunk_hashes: new_chunks,
        });
    }
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
    fn empty_chain_known_chunks_is_empty() {
        let b = IncrementalBackup::new("/tmp");
        assert!(b.chain.known_chunks().is_empty());
    }

    #[test]
    fn new_chunks_excludes_known() {
        let mut b = IncrementalBackup::new("/tmp");
        let h1 = Hash::ZERO;
        b.chain.manifests.push(BackupManifest {
            id: "1".into(), parent_id: None, commit_hash: Hash::ZERO,
            created_at_ms: 0, chunk_count: 1, byte_count: 0,
            bundle_path: "/tmp/b1".into(), new_chunk_hashes: vec![h1],
        });
        let all = vec![h1];
        let new = b.new_chunks(&all);
        assert!(new.is_empty(), "already-known chunk should be excluded");
    }

    #[test]
    fn record_increments_chain() {
        let mut b = IncrementalBackup::new("/tmp");
        b.record(Hash::ZERO, vec![], 0, "/tmp/b1.bundle".into());
        assert_eq!(b.chain.manifests.len(), 1);
        assert!(b.chain.manifests[0].parent_id.is_none());
        b.record(Hash::ZERO, vec![], 0, "/tmp/b2.bundle".into());
        assert_eq!(b.chain.manifests.len(), 2);
        assert!(b.chain.manifests[1].parent_id.is_some());
    }
}
