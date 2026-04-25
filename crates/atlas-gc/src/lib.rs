//! Garbage collector.
//!
//! Two complementary mechanisms:
//!
//! 1. **Mark-sweep** ([`mark_sweep`]) walks every commit reachable from
//!    every branch and marks the manifests + chunks they reach. Anything
//!    in the chunk store *not* marked is unreachable and gets swept.
//!    This is correct but expensive — O(reachable graph).
//!
//! 2. **Refcount journal** ([`Refcounts`]) keeps a per-chunk refcount in
//!    the meta store, updated incrementally on `put_blob_manifest` /
//!    `delete`. A chunk hits zero, it's eligible for immediate
//!    reclamation. Mark-sweep then runs as a periodic correctness
//!    backstop.
//!
//! The journal is *advisory*. Mark-sweep is the source of truth — if the
//! two ever disagree we trust mark-sweep and rebuild the journal.

use atlas_chunk::ChunkStore;
use atlas_core::{Error, Hash, ObjectKind, Result};
use atlas_meta::{keys, MetaStore};
use atlas_object::{Branch, DirectoryManifest, FileManifest};
use std::collections::{HashMap, HashSet};

/// Outcome of a mark-sweep pass.
#[derive(Debug, Default, Clone)]
pub struct GcReport {
    pub chunks_seen: usize,
    pub chunks_marked: usize,
    pub chunks_swept: usize,
    pub manifests_visited: usize,
}

/// Trace every reachable chunk from every branch head, then delete
/// chunks the chunk store has but no manifest references.
///
/// `dry_run = true` reports what *would* be swept without deleting.
pub fn mark_sweep(
    meta: &dyn MetaStore,
    chunks: &dyn ChunkStore,
    dry_run: bool,
) -> Result<GcReport> {
    let mut report = GcReport::default();
    let mut reachable: HashSet<Hash> = HashSet::new();
    let mut visited_manifests: HashSet<Hash> = HashSet::new();

    let branches: Vec<Branch> = meta.list_branches()?;
    for b in &branches {
        walk_commit(meta, b.head, &mut reachable, &mut visited_manifests)?;
    }
    report.manifests_visited = visited_manifests.len();
    report.chunks_marked = reachable.len();

    let on_disk = chunks.iter_hashes()?;
    report.chunks_seen = on_disk.len();
    for h in on_disk {
        if !reachable.contains(&h) {
            if !dry_run {
                if let Err(e) = chunks.delete(&h) {
                    if !matches!(e, Error::NotFound(_)) {
                        return Err(e);
                    }
                }
            }
            report.chunks_swept += 1;
        }
    }
    Ok(report)
}

fn walk_commit(
    meta: &dyn MetaStore,
    commit: Hash,
    reachable: &mut HashSet<Hash>,
    visited: &mut HashSet<Hash>,
) -> Result<()> {
    let mut stack = vec![commit];
    while let Some(c) = stack.pop() {
        if !visited.insert(c) {
            continue;
        }
        let Some(commit) = meta.get_commit(&c)? else {
            continue;
        };
        walk_dir(meta, commit.tree_hash, reachable, visited)?;
        for p in &commit.parents {
            if !visited.contains(p) {
                stack.push(*p);
            }
        }
    }
    Ok(())
}

fn walk_dir(
    meta: &dyn MetaStore,
    dir_hash: Hash,
    reachable: &mut HashSet<Hash>,
    visited: &mut HashSet<Hash>,
) -> Result<()> {
    if !visited.insert(dir_hash) {
        return Ok(());
    }
    let dir: DirectoryManifest = meta
        .get_dir_manifest(&dir_hash)?
        .ok_or_else(|| Error::NotFound(format!("dir {}", dir_hash.short())))?;
    for entry in &dir.entries {
        match entry.kind {
            ObjectKind::Dir => walk_dir(meta, entry.object_hash, reachable, visited)?,
            ObjectKind::File => walk_file(meta, entry.object_hash, reachable, visited)?,
            _ => {}
        }
    }
    Ok(())
}

fn walk_file(
    meta: &dyn MetaStore,
    file_hash: Hash,
    reachable: &mut HashSet<Hash>,
    visited: &mut HashSet<Hash>,
) -> Result<()> {
    if !visited.insert(file_hash) {
        return Ok(());
    }
    let file: FileManifest = meta
        .get_file_manifest(&file_hash)?
        .ok_or_else(|| Error::NotFound(format!("file {}", file_hash.short())))?;
    if let Some(blob) = meta.get_blob_manifest(&file.blob_hash)? {
        for c in &blob.chunks {
            reachable.insert(c.hash);
        }
    }
    Ok(())
}

// -- Refcount journal ----------------------------------------------------

/// Per-chunk refcount stored in the meta KV under `gcref:<hex>`.
pub struct Refcounts<'a> {
    meta: &'a dyn MetaStore,
}

impl<'a> Refcounts<'a> {
    pub fn new(meta: &'a dyn MetaStore) -> Self {
        Self { meta }
    }

    pub fn key(h: &Hash) -> String {
        format!("gcref:{}", h.to_hex())
    }

    pub fn get(&self, h: &Hash) -> Result<u64> {
        match self.meta.get_raw(&Self::key(h))? {
            Some(b) if b.len() == 8 => Ok(u64::from_le_bytes(b.try_into().unwrap())),
            _ => Ok(0),
        }
    }

    pub fn incr(&self, h: &Hash, by: u64) -> Result<u64> {
        let n = self.get(h)?.saturating_add(by);
        self.meta.put_raw(&Self::key(h), &n.to_le_bytes())?;
        Ok(n)
    }

    pub fn decr(&self, h: &Hash, by: u64) -> Result<u64> {
        let n = self.get(h)?.saturating_sub(by);
        if n == 0 {
            self.meta.delete(&Self::key(h))?;
        } else {
            self.meta.put_raw(&Self::key(h), &n.to_le_bytes())?;
        }
        Ok(n)
    }

    /// Throw the journal away and rebuild from scratch by walking the
    /// reachable graph. Use after disagreement with mark-sweep.
    pub fn rebuild(&self) -> Result<HashMap<Hash, u64>> {
        // Clear existing refcount entries.
        for (k, _) in self.meta.scan_prefix("gcref:")? {
            self.meta.delete(&k)?;
        }

        let mut counts: HashMap<Hash, u64> = HashMap::new();
        let mut visited: HashSet<Hash> = HashSet::new();
        for b in self.meta.list_branches()? {
            walk_for_counts(self.meta, b.head, &mut counts, &mut visited)?;
        }
        for (h, n) in &counts {
            self.meta.put_raw(&Self::key(h), &n.to_le_bytes())?;
        }
        Ok(counts)
    }

    /// Sweep chunks whose refcount is zero. Returns count deleted.
    pub fn sweep_zero(&self, chunks: &dyn ChunkStore) -> Result<usize> {
        let mut count = 0;
        for h in chunks.iter_hashes()? {
            if self.get(&h)? == 0 {
                if let Err(e) = chunks.delete(&h) {
                    if !matches!(e, Error::NotFound(_)) {
                        return Err(e);
                    }
                }
                count += 1;
            }
        }
        Ok(count)
    }
}

fn walk_for_counts(
    meta: &dyn MetaStore,
    commit: Hash,
    counts: &mut HashMap<Hash, u64>,
    visited: &mut HashSet<Hash>,
) -> Result<()> {
    let mut stack = vec![commit];
    while let Some(c) = stack.pop() {
        if !visited.insert(c) {
            continue;
        }
        let Some(commit) = meta.get_commit(&c)? else {
            continue;
        };
        walk_dir_counts(meta, commit.tree_hash, counts, visited)?;
        for p in &commit.parents {
            if !visited.contains(p) {
                stack.push(*p);
            }
        }
    }
    Ok(())
}

fn walk_dir_counts(
    meta: &dyn MetaStore,
    dir_hash: Hash,
    counts: &mut HashMap<Hash, u64>,
    visited: &mut HashSet<Hash>,
) -> Result<()> {
    if !visited.insert(dir_hash) {
        return Ok(());
    }
    let Some(dir) = meta.get_dir_manifest(&dir_hash)? else {
        return Ok(());
    };
    for entry in &dir.entries {
        match entry.kind {
            ObjectKind::Dir => walk_dir_counts(meta, entry.object_hash, counts, visited)?,
            ObjectKind::File => {
                if !visited.insert(entry.object_hash) {
                    continue;
                }
                if let Some(file) = meta.get_file_manifest(&entry.object_hash)? {
                    if let Some(blob) = meta.get_blob_manifest(&file.blob_hash)? {
                        for c in &blob.chunks {
                            *counts.entry(c.hash).or_default() += 1;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

// quiet unused-import warning when unused in this module's public surface
#[allow(dead_code)]
fn _keys_link() -> &'static str {
    keys::object_prefix()
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_core::Author;
    use atlas_fs::Fs;
    use atlas_version::Version;

    fn fixture() -> (tempfile::TempDir, Fs) {
        let dir = tempfile::tempdir().unwrap();
        let fs = Fs::init(dir.path()).unwrap();
        (dir, fs)
    }

    #[test]
    fn mark_sweep_keeps_referenced_chunks() {
        let (_d, fs) = fixture();
        fs.write("/a", b"hello world").unwrap();
        let v = Version::new(&fs);
        v.commit(Author::new("u", "u@x"), "first").unwrap();

        let report = mark_sweep(fs.meta(), fs.chunks(), false).unwrap();
        assert_eq!(report.chunks_swept, 0, "nothing should be swept");
        assert!(report.chunks_marked > 0);
    }

    #[test]
    fn mark_sweep_drops_orphan_chunk() {
        let (_d, fs) = fixture();
        fs.write("/a", b"x").unwrap();
        let v = Version::new(&fs);
        v.commit(Author::new("u", "u@x"), "c1").unwrap();

        // Inject an unreferenced chunk.
        let orphan = fs.chunks().put(b"orphan-bytes").unwrap();
        assert!(fs.chunks().has(&orphan).unwrap());

        let report = mark_sweep(fs.meta(), fs.chunks(), false).unwrap();
        assert!(report.chunks_swept >= 1);
        assert!(!fs.chunks().has(&orphan).unwrap());
    }

    #[test]
    fn refcount_rebuild_matches_mark_sweep() {
        let (_d, fs) = fixture();
        fs.write("/a", b"abc").unwrap();
        fs.write("/b", b"def").unwrap();
        let v = Version::new(&fs);
        v.commit(Author::new("u", "u@x"), "two files").unwrap();

        let rc = Refcounts::new(fs.meta());
        let counts = rc.rebuild().unwrap();
        let report = mark_sweep(fs.meta(), fs.chunks(), true).unwrap();
        assert_eq!(counts.len(), report.chunks_marked);
    }

    #[test]
    fn refcount_sweep_zero_drops_orphans() {
        let (_d, fs) = fixture();
        fs.write("/a", b"x").unwrap();
        Version::new(&fs)
            .commit(Author::new("u", "u@x"), "c")
            .unwrap();
        let rc = Refcounts::new(fs.meta());
        rc.rebuild().unwrap();

        let orphan = fs.chunks().put(b"unreferenced").unwrap();
        // refcount for orphan is 0 — sweep_zero should drop it.
        let n = rc.sweep_zero(fs.chunks()).unwrap();
        assert!(n >= 1);
        assert!(!fs.chunks().has(&orphan).unwrap());
    }
}
