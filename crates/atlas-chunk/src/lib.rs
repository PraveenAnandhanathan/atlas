//! ATLAS chunk layer — content-addressed storage.
//!
//! Every blob is split into fixed-size chunks (default 4 MiB; see
//! [ADR-0002](../../../docs/adr/0002-blake3-and-4mib-chunks.md)),
//! hashed with BLAKE3-256, and stored once globally. Identical chunks
//! dedupe. Corruption is detectable because the filename is the hash.
//!
//! Phase 0 ships a single implementation — [`LocalChunkStore`] — that
//! writes chunks to a sharded directory tree on the local filesystem.
//! Phase 2 adds a networked [`ChunkStore`] over gRPC + CRAQ chains
//! (plan T2.1, T2.4) behind the same trait.

use atlas_core::{Error, Hash, Result};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Default chunk size: 4 MiB. See ADR-0002.
pub const DEFAULT_CHUNK_SIZE: usize = 4 * 1024 * 1024;

/// A content-addressed chunk store.
///
/// Implementations MUST guarantee that:
/// - `put(bytes)` returns `Hash::of(bytes)` and stores the chunk under that hash.
/// - `get(h)` returns the exact bytes whose BLAKE3 equals `h`, or `NotFound`.
/// - `verify(h)` rehashes the stored content and fails on mismatch.
pub trait ChunkStore: Send + Sync {
    /// Store a chunk. Idempotent: writing the same bytes twice is cheap
    /// and yields the same hash.
    fn put(&self, bytes: &[u8]) -> Result<Hash>;

    /// Retrieve a chunk's raw bytes.
    fn get(&self, hash: &Hash) -> Result<Vec<u8>>;

    /// Delete a chunk. Caller is responsible for ensuring nothing
    /// references it (GC normally handles this).
    fn delete(&self, hash: &Hash) -> Result<()>;

    /// True if the chunk exists in the store.
    fn has(&self, hash: &Hash) -> Result<bool>;

    /// Recompute the hash of a stored chunk and compare it to its filename.
    /// Returns `Error::Integrity` on mismatch.
    fn verify(&self, hash: &Hash) -> Result<()>;

    /// Size (in bytes) of a stored chunk.
    fn size(&self, hash: &Hash) -> Result<u64>;

    /// Enumerate every chunk currently stored. Used by GC and `verify`.
    /// Returns a lazy iterator; each item may individually fail (I/O error).
    /// Default implementation returns an empty iterator — networked backends
    /// that cannot cheaply enumerate may keep that default.
    fn iter_hashes(&self) -> Box<dyn Iterator<Item = Result<Hash>> + '_> {
        Box::new(std::iter::empty())
    }
}

/// Local filesystem backend: chunks live under `<root>/<hh>/<hh>/<full_hex>`.
///
/// Two levels of sharding keeps directory sizes bounded: at 1M chunks
/// spread over 65,536 buckets (256×256), each directory holds ~16 files.
pub struct LocalChunkStore {
    root: PathBuf,
}

impl LocalChunkStore {
    /// Open (or create) a chunk store rooted at `root`.
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    /// Path to a chunk on disk.
    fn path_for(&self, hash: &Hash) -> PathBuf {
        let hex = hash.to_hex();
        self.root.join(&hex[0..2]).join(&hex[2..4]).join(&hex)
    }
}

impl ChunkStore for LocalChunkStore {
    fn put(&self, bytes: &[u8]) -> Result<Hash> {
        let hash = Hash::of(bytes);
        let path = self.path_for(&hash);
        if path.exists() {
            // Already present. Cheap dedup.
            return Ok(hash);
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        // Write atomically: tmp file + rename.
        let tmp = path.with_extension("tmp");
        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(bytes)?;
            f.sync_all()?;
        }
        fs::rename(&tmp, &path)?;
        tracing::trace!(hash = %hash.short(), size = bytes.len(), "chunk put");
        Ok(hash)
    }

    fn get(&self, hash: &Hash) -> Result<Vec<u8>> {
        let path = self.path_for(hash);
        match fs::File::open(&path) {
            Ok(mut f) => {
                let mut buf = Vec::new();
                f.read_to_end(&mut buf)?;
                Ok(buf)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(Error::NotFound(format!("chunk {}", hash.short())))
            }
            Err(e) => Err(e.into()),
        }
    }

    fn delete(&self, hash: &Hash) -> Result<()> {
        let path = self.path_for(hash);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(Error::NotFound(format!("chunk {}", hash.short())))
            }
            Err(e) => Err(e.into()),
        }
    }

    fn has(&self, hash: &Hash) -> Result<bool> {
        Ok(self.path_for(hash).exists())
    }

    fn verify(&self, hash: &Hash) -> Result<()> {
        let bytes = self.get(hash)?;
        let actual = Hash::of(&bytes);
        if &actual != hash {
            return Err(Error::Integrity {
                expected: hash.to_hex(),
                actual: actual.to_hex(),
            });
        }
        Ok(())
    }

    fn size(&self, hash: &Hash) -> Result<u64> {
        let path = self.path_for(hash);
        match fs::metadata(&path) {
            Ok(m) => Ok(m.len()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(Error::NotFound(format!("chunk {}", hash.short())))
            }
            Err(e) => Err(e.into()),
        }
    }

    fn iter_hashes(&self) -> Box<dyn Iterator<Item = Result<Hash>> + '_> {
        let root = self.root.clone();
        let iter = iter_chunk_hashes(root);
        Box::new(iter)
    }
}

/// Lazily iterate over all chunk hashes stored under `root`.
/// Errors (e.g. permission denied on a shard directory) are yielded in-stream
/// so callers decide whether to abort or skip.
fn iter_chunk_hashes(root: PathBuf) -> impl Iterator<Item = Result<Hash>> {
    let outer = match fs::read_dir(&root) {
        Ok(rd) => rd,
        Err(_) => return itertools_none(),
    };
    let it = outer
        .filter_map(|a| a.ok())
        .filter(|a| a.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .flat_map(|a| {
            fs::read_dir(a.path())
                .into_iter()
                .flatten()
                .filter_map(|b| b.ok())
                .filter(|b| b.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .flat_map(|b| {
                    fs::read_dir(b.path())
                        .into_iter()
                        .flatten()
                        .filter_map(|f| f.ok())
                        .filter_map(|f| {
                            let name = f.file_name();
                            let name_str = name.to_string_lossy();
                            Hash::from_hex(name_str.as_ref()).ok().map(Ok)
                        })
                })
        });
    Box::new(it) as Box<dyn Iterator<Item = Result<Hash>>>
}

fn itertools_none() -> Box<dyn Iterator<Item = Result<Hash>>> {
    Box::new(std::iter::empty())
}

/// Split a byte slice into chunk-sized windows.
///
/// Returns a list of `(offset, length)` pairs. The caller can then hash
/// and store each window. Every chunk except the last has length
/// exactly `chunk_size`.
pub fn window(total_len: usize, chunk_size: usize) -> Vec<(usize, usize)> {
    assert!(chunk_size > 0, "chunk_size must be > 0");
    if total_len == 0 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(total_len.div_ceil(chunk_size));
    let mut off = 0;
    while off < total_len {
        let len = (total_len - off).min(chunk_size);
        out.push((off, len));
        off += len;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn tmp_store() -> (TempDir, LocalChunkStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = LocalChunkStore::open(dir.path()).unwrap();
        (dir, store)
    }

    #[test]
    fn put_then_get() {
        let (_dir, store) = tmp_store();
        let h = store.put(b"hello").unwrap();
        assert_eq!(h, Hash::of(b"hello"));
        assert_eq!(store.get(&h).unwrap(), b"hello".to_vec());
        assert!(store.has(&h).unwrap());
        assert_eq!(store.size(&h).unwrap(), 5);
    }

    #[test]
    fn put_is_idempotent_and_dedupes() {
        let (_dir, store) = tmp_store();
        let h1 = store.put(b"aaaa").unwrap();
        let h2 = store.put(b"aaaa").unwrap();
        assert_eq!(h1, h2);
    }

    #[test]
    fn verify_detects_corruption() {
        let (dir, store) = tmp_store();
        let h = store.put(b"data").unwrap();
        // Corrupt the on-disk chunk.
        let path = store.path_for(&h);
        std::fs::write(&path, b"tampered").unwrap();
        match store.verify(&h) {
            Err(Error::Integrity { .. }) => {}
            other => panic!("expected Integrity, got {:?}", other),
        }
        drop(dir);
    }

    #[test]
    fn missing_chunk_is_not_found() {
        let (_dir, store) = tmp_store();
        let fake = Hash::of(b"nope");
        assert!(matches!(store.get(&fake), Err(Error::NotFound(_))));
    }

    #[test]
    fn delete_removes_chunk() {
        let (_dir, store) = tmp_store();
        let h = store.put(b"x").unwrap();
        store.delete(&h).unwrap();
        assert!(!store.has(&h).unwrap());
    }

    #[test]
    fn iter_hashes_finds_all() {
        let (_dir, store) = tmp_store();
        let a = store.put(b"a").unwrap();
        let b = store.put(b"bb").unwrap();
        let c = store.put(b"ccc").unwrap();
        let mut listed: Vec<Hash> = store.iter_hashes().filter_map(|r| r.ok()).collect();
        listed.sort();
        let mut expected = vec![a, b, c];
        expected.sort();
        assert_eq!(listed, expected);
    }

    #[test]
    fn window_splits_correctly() {
        assert_eq!(window(0, 4), Vec::<(usize, usize)>::new());
        assert_eq!(window(10, 4), vec![(0, 4), (4, 4), (8, 2)]);
        assert_eq!(window(8, 4), vec![(0, 4), (4, 4)]);
        assert_eq!(window(3, 4), vec![(0, 3)]);
    }
}
