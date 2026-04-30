//! The filesystem engine.
//!
//! [`Fs`] owns a chunk store and a metadata store and exposes the
//! operations every frontend (CLI, FUSE, SDK) calls into.
//!
//! # Model
//!
//! - There is always exactly one *working root*: the directory manifest
//!   hash stored under the ref `"/"`. Writes update this ref.
//! - `HEAD` tracks a branch (normally `main`). `commit` reads the
//!   working root, seals a [`Commit`] that points at it, and advances
//!   the branch — all in `atlas-version`.
//! - Every write is copy-on-write: new manifests are created bottom-up
//!   from the write site to the root; untouched subtrees share storage.

use crate::path::{normalize_path, parent_and_name, split_path};
use atlas_chunk::{window, ChunkStore, LocalChunkStore, DEFAULT_CHUNK_SIZE};
use atlas_core::{time::now_millis, Author, Error, Hash, ObjectKind, Result};
use atlas_meta::{MetaStore, SledStore};
use atlas_object::{
    codec::seal, Branch, BranchProtection, ChunkRef, Commit, DirEntry, DirectoryManifest,
    FileManifest, HeadState, RefRecord, StoreConfig,
};
use dashmap::DashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Hook called before every mutating operation.
/// Implementations return `Err` to block the operation (e.g. quota exceeded).
pub trait WriteHook: Send + Sync {
    fn before_write(&self, path: &str, bytes_len: u64) -> Result<()>;
}

/// The kind of filesystem operation being authorized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpKind {
    Read,
    Write,
    Delete,
    Rename,
    List,
    Stat,
    Mkdir,
}

/// Hook called before every filesystem operation to enforce authorization.
/// Return `Err(Error::PermissionDenied(...))` to block the operation.
pub trait AuthHook: Send + Sync {
    fn authorize(&self, principal: &str, path: &str, op: OpKind) -> Result<()>;
}

/// Publicly visible metadata about a file or directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    pub kind: ObjectKind,
    pub hash: Hash,
    pub size: u64,
}

/// The filesystem engine.
///
/// Cheap to clone — the underlying stores are in [`Arc`].
///
/// A single `write_gate` mutex serialises all mutations (write, delete,
/// rename, mkdir) so concurrent callers cannot produce a split-brain
/// working root. Reads (stat, list, read) never acquire the gate.
///
/// `dir_cache` is a content-addressed cache of decoded `DirectoryManifest`
/// values keyed by hash. Because manifests are immutable once sealed, entries
/// are never invalidated — new mutations produce new hashes and new entries.
#[derive(Clone)]
pub struct Fs {
    pub(crate) chunks: Arc<dyn ChunkStore>,
    pub(crate) meta: Arc<dyn MetaStore>,
    pub(crate) root_dir: PathBuf,
    /// Serialises all working-root mutations. (P0-1)
    write_gate: Arc<Mutex<()>>,
    /// Optional pre-write hook (quota enforcement, rate limiting). (P0-2)
    write_hook: Option<Arc<dyn WriteHook>>,
    /// Optional authorization hook wired into every operation.
    auth_hook: Option<Arc<dyn AuthHook>>,
    /// The principal (user/service identity) performing operations on this handle.
    principal: Option<String>,
    /// In-process cache of decoded directory manifests (P2-1).
    dir_cache: Arc<DashMap<Hash, DirectoryManifest>>,
}

impl Fs {
    /// Create a new ATLAS store under `root_dir`. Fails if it already exists.
    pub fn init(root_dir: impl AsRef<Path>) -> Result<Self> {
        let root = root_dir.as_ref().to_path_buf();
        if root.join("config.bin").exists() {
            return Err(Error::AlreadyExists(format!(
                "store already initialized: {}",
                root.display()
            )));
        }
        std::fs::create_dir_all(&root)?;
        let fs = Self::open_paths(&root)?;

        // Persist config.
        let cfg = StoreConfig::default();
        fs.meta.put_config(&cfg)?;
        std::fs::write(
            root.join("config.bin"),
            bincode::serialize(&cfg).map_err(|e| Error::Serde(e.to_string()))?,
        )?;

        // Empty root directory manifest.
        let mut empty_root = DirectoryManifest {
            hash: Hash::ZERO,
            entries: Vec::new(),
            xattrs: Vec::new(),
            policy_ref: None,
        };
        let (root_hash, _bytes) = seal(&mut empty_root)?;
        fs.meta.put_dir_manifest(&empty_root)?;

        fs.meta.put_ref(&RefRecord {
            path: "/".into(),
            target: root_hash,
            updated_at: now_millis(),
        })?;

        // Synthetic root commit so `log` has something to show and
        // `checkout main` has a target before the user's first commit.
        let mut root_commit = Commit {
            hash: Hash::ZERO,
            tree_hash: root_hash,
            parents: Vec::new(),
            author: Author::new("atlas", "init@atlas"),
            timestamp: now_millis(),
            message: "init: empty store".into(),
            signature: None,
        };
        let (commit_hash, _) = seal(&mut root_commit)?;
        fs.meta.put_commit(&root_commit)?;
        fs.meta.put_branch(&Branch {
            name: cfg.default_branch.clone(),
            head: commit_hash,
            protection: BranchProtection::default(),
        })?;
        fs.meta
            .put_head(&HeadState::Branch(cfg.default_branch.clone()))?;

        Ok(fs)
    }

    /// Open an existing ATLAS store under `root_dir`.
    pub fn open(root_dir: impl AsRef<Path>) -> Result<Self> {
        let root = root_dir.as_ref().to_path_buf();
        if !root.join("config.bin").exists() {
            return Err(Error::NotFound(format!(
                "no ATLAS store at {}",
                root.display()
            )));
        }
        Self::open_paths(&root)
    }

    fn open_paths(root: &Path) -> Result<Self> {
        let chunks = LocalChunkStore::open(root.join("chunks"))?;
        let meta = SledStore::open(root.join("meta"))?;
        Ok(Self {
            chunks: Arc::new(chunks),
            meta: Arc::new(meta),
            root_dir: root.to_path_buf(),
            write_gate: Arc::new(Mutex::new(())),
            write_hook: None,
            auth_hook: None,
            principal: None,
            dir_cache: Arc::new(DashMap::new()),
        })
    }

    /// Attach a write hook (e.g. quota enforcer) to this `Fs` instance.
    /// Returns a new clone with the hook set; the original is unchanged.
    pub fn with_write_hook(mut self, hook: Arc<dyn WriteHook>) -> Self {
        self.write_hook = Some(hook);
        self
    }

    /// Attach an authorization hook. Every `read`, `write`, `delete`,
    /// `rename`, `mkdir`, `list`, and `stat` call will invoke it first.
    pub fn with_auth_hook(mut self, hook: Arc<dyn AuthHook>) -> Self {
        self.auth_hook = Some(hook);
        self
    }

    /// Set the principal (user identity) for this `Fs` handle.
    pub fn with_principal(mut self, principal: impl Into<String>) -> Self {
        self.principal = Some(principal.into());
        self
    }

    /// The directory this store lives in on disk.
    pub fn store_path(&self) -> &Path {
        &self.root_dir
    }

    // -- Authorization ------------------------------------------------

    /// Check authorization for `op` on `path`. No-ops when no hook is set.
    fn auth_check(&self, path: &str, op: OpKind) -> Result<()> {
        if let Some(hook) = &self.auth_hook {
            let principal = self.principal.as_deref().unwrap_or("anonymous");
            hook.authorize(principal, path, op)?;
        }
        Ok(())
    }

    // -- Reads --------------------------------------------------------

    /// Resolve an absolute path to its object hash and kind.
    pub fn stat(&self, path: &str) -> Result<Entry> {
        let path = normalize_path(path)?;
        self.auth_check(&path, OpKind::Stat)?;
        let (hash, kind) = self.resolve(&path)?;
        let size = match kind {
            ObjectKind::File => self.file_size(&hash)?,
            _ => 0,
        };
        Ok(Entry {
            path,
            kind,
            hash,
            size,
        })
    }

    /// List the immediate children of a directory.
    pub fn list(&self, path: &str) -> Result<Vec<Entry>> {
        let path = normalize_path(path)?;
        self.auth_check(&path, OpKind::List)?;
        let (hash, kind) = self.resolve(&path)?;
        if kind != ObjectKind::Dir {
            return Err(Error::Invalid(format!("not a directory: {path}")));
        }
        let dir = self.load_dir(&hash)?;
        let mut out = Vec::with_capacity(dir.entries.len());
        for e in &dir.entries {
            let child_path = join(&path, &e.name);
            let size = if e.kind == ObjectKind::File {
                self.file_size(&e.object_hash).unwrap_or(0)
            } else {
                0
            };
            out.push(Entry {
                path: child_path,
                kind: e.kind,
                hash: e.object_hash,
                size,
            });
        }
        Ok(out)
    }

    /// Read a file's raw bytes.
    pub fn read(&self, path: &str) -> Result<Vec<u8>> {
        let path = normalize_path(path)?;
        self.auth_check(&path, OpKind::Read)?;
        let (hash, kind) = self.resolve(&path)?;
        if kind != ObjectKind::File {
            return Err(Error::Invalid(format!("not a file: {path}")));
        }
        let file = self.load_file(&hash)?;
        let blob = self
            .meta
            .get_blob_manifest(&file.blob_hash)?
            .ok_or_else(|| Error::NotFound(format!("blob manifest {}", file.blob_hash.short())))?;
        let mut out = Vec::with_capacity(blob.total_size as usize);
        for c in &blob.chunks {
            let bytes = self.chunks.get(&c.hash)?;
            // Verify hash before trusting the bytes (P0-3: silent-corruption prevention).
            let actual = Hash::of(&bytes);
            if actual != c.hash {
                return Err(Error::Integrity {
                    expected: c.hash.to_hex(),
                    actual: actual.to_hex(),
                });
            }
            if bytes.len() as u32 != c.length {
                return Err(Error::Integrity {
                    expected: format!("chunk {} length {}", c.hash.short(), c.length),
                    actual: format!("{}", bytes.len()),
                });
            }
            out.extend_from_slice(&bytes);
        }
        Ok(out)
    }

    // -- Writes -------------------------------------------------------

    /// Create or overwrite a file at `path` with `bytes`.
    pub fn write(&self, path: &str, bytes: &[u8]) -> Result<Entry> {
        let path = normalize_path(path)?;
        self.auth_check(&path, OpKind::Write)?;
        let (parent, name) = parent_and_name(&path)?;
        if name.is_empty() {
            return Err(Error::BadPath(format!("cannot write root: {path}")));
        }

        // quota / rate-limit hook
        if let Some(hook) = &self.write_hook {
            hook.before_write(&path, bytes.len() as u64)?;
        }

        // P0-1: serialise all mutations through the write gate
        let _gate = self.write_gate.lock()
            .map_err(|_| Error::Internal("write-gate mutex poisoned".into()))?;

        let blob_hash = self.write_blob(bytes)?;

        let mut file = FileManifest {
            hash: Hash::ZERO,
            blob_hash,
            created_at: now_millis(),
            mode: 0o100644,
            xattrs: Vec::new(),
            embeddings: Vec::new(),
            schema_ref: None,
            lineage_ref: None,
            policy_ref: None,
            signatures: Vec::new(),
        };
        let (file_hash, _) = seal(&mut file)?;
        self.meta.put_file_manifest(&file)?;

        let entry = DirEntry {
            name: name.clone(),
            object_hash: file_hash,
            kind: ObjectKind::File,
        };
        self.mutate_at(&parent, |entries| {
            upsert_in_dir(entries, entry);
            Ok(())
        })?;

        Ok(Entry {
            path,
            kind: ObjectKind::File,
            hash: file_hash,
            size: bytes.len() as u64,
        })
    }

    /// Delete a file or empty directory at `path`.
    pub fn delete(&self, path: &str) -> Result<()> {
        let path = normalize_path(path)?;
        self.auth_check(&path, OpKind::Delete)?;
        if path == "/" {
            return Err(Error::Invalid("cannot delete root".into()));
        }
        let (parent, name) = parent_and_name(&path)?;
        let _gate = self.write_gate.lock()
            .map_err(|_| Error::Internal("write-gate mutex poisoned".into()))?;

        let (target_hash, target_kind) = self.resolve(&path)?;
        if target_kind == ObjectKind::Dir {
            let dir = self.load_dir(&target_hash)?;
            if !dir.entries.is_empty() {
                return Err(Error::Invalid(format!("directory not empty: {path}")));
            }
        }

        self.mutate_at(&parent, |entries| {
            let before = entries.len();
            entries.retain(|e| e.name != name);
            if entries.len() == before {
                return Err(Error::NotFound(format!("no such entry: {name}")));
            }
            Ok(())
        })
    }

    /// Rename `from` to `to`. Overwrites `to` if it already exists.
    pub fn rename(&self, from: &str, to: &str) -> Result<()> {
        let from = normalize_path(from)?;
        let to = normalize_path(to)?;
        self.auth_check(&from, OpKind::Rename)?;
        if from == to {
            return Ok(());
        }
        let _gate = self.write_gate.lock()
            .map_err(|_| Error::Internal("write-gate mutex poisoned".into()))?;
        let (from_parent, from_name) = parent_and_name(&from)?;
        let (to_parent, to_name) = parent_and_name(&to)?;

        let (target_hash, target_kind) = self.resolve(&from)?;

        // Remove from source.
        self.mutate_at(&from_parent, |entries| {
            let before = entries.len();
            entries.retain(|e| e.name != from_name);
            if entries.len() == before {
                return Err(Error::NotFound(format!("no such entry: {from_name}")));
            }
            Ok(())
        })?;

        // Insert at destination.
        let new_entry = DirEntry {
            name: to_name,
            object_hash: target_hash,
            kind: target_kind,
        };
        self.mutate_at(&to_parent, |entries| {
            upsert_in_dir(entries, new_entry);
            Ok(())
        })
    }

    /// Create an empty directory at `path`. Idempotent if it already exists.
    pub fn mkdir(&self, path: &str) -> Result<()> {
        let path = normalize_path(path)?;
        self.auth_check(&path, OpKind::Mkdir)?;
        if path == "/" {
            return Ok(());
        }
        let _gate = self.write_gate.lock()
            .map_err(|_| Error::Internal("write-gate mutex poisoned".into()))?;
        if let Ok((_h, k)) = self.resolve(&path) {
            if k == ObjectKind::Dir {
                return Ok(());
            }
            return Err(Error::AlreadyExists(format!(
                "path exists with kind {k}: {path}"
            )));
        }
        // mutate_at creates the dir as part of its walk, leaving it empty.
        self.mutate_at(&path, |_entries| Ok(()))
    }

    // -- Version-engine escape hatches -------------------------------

    /// Hash of the current working root (the ref at `/`).
    pub fn working_root(&self) -> Result<Hash> {
        Ok(self
            .meta
            .get_ref("/")?
            .ok_or_else(|| Error::NotFound("root ref".into()))?
            .target)
    }

    /// Replace the working root. Used by `checkout`.
    pub fn set_working_root(&self, target: Hash) -> Result<()> {
        self.meta.put_ref(&RefRecord {
            path: "/".into(),
            target,
            updated_at: now_millis(),
        })
    }

    /// Read-only access to the underlying chunk store.
    pub fn chunks(&self) -> &dyn ChunkStore {
        &*self.chunks
    }

    /// Read-only access to the underlying metadata store.
    pub fn meta(&self) -> &dyn MetaStore {
        &*self.meta
    }

    // -- Internals ----------------------------------------------------

    fn write_blob(&self, bytes: &[u8]) -> Result<Hash> {
        let mut chunks = Vec::new();
        for (off, len) in window(bytes.len(), DEFAULT_CHUNK_SIZE) {
            let slice = &bytes[off..off + len];
            let h = self.chunks.put(slice)?;
            chunks.push(ChunkRef {
                hash: h,
                length: len as u32,
            });
        }
        let mut blob = atlas_object::BlobManifest {
            hash: Hash::ZERO,
            total_size: bytes.len() as u64,
            format_hint: None,
            chunks,
        };
        let (h, _) = seal(&mut blob)?;
        self.meta.put_blob_manifest(&blob)?;
        Ok(h)
    }

    fn file_size(&self, file_hash: &Hash) -> Result<u64> {
        let file = self.load_file(file_hash)?;
        let blob = self
            .meta
            .get_blob_manifest(&file.blob_hash)?
            .ok_or_else(|| Error::NotFound(format!("blob {}", file.blob_hash.short())))?;
        Ok(blob.total_size)
    }

    fn load_dir(&self, h: &Hash) -> Result<DirectoryManifest> {
        if let Some(cached) = self.dir_cache.get(h) {
            return Ok(cached.clone());
        }
        let dir = self
            .meta
            .get_dir_manifest(h)?
            .ok_or_else(|| Error::NotFound(format!("dir manifest {}", h.short())))?;
        self.dir_cache.insert(*h, dir.clone());
        Ok(dir)
    }

    fn load_file(&self, h: &Hash) -> Result<FileManifest> {
        self.meta
            .get_file_manifest(h)?
            .ok_or_else(|| Error::NotFound(format!("file manifest {}", h.short())))
    }

    /// Walk from the working root to `path`, returning `(hash, kind)`.
    fn resolve(&self, path: &str) -> Result<(Hash, ObjectKind)> {
        let parts = split_path(path)?;
        let mut current_hash = self.working_root()?;
        let mut current_kind = ObjectKind::Dir;
        for part in &parts {
            let dir = self.load_dir(&current_hash)?;
            let entry = dir
                .entries
                .iter()
                .find(|e| &e.name == part)
                .ok_or_else(|| Error::NotFound(format!("path not found: {path}")))?;
            current_hash = entry.object_hash;
            current_kind = entry.kind;
        }
        Ok((current_hash, current_kind))
    }

    /// Recursively rebuild the tree so the directory at `dir_path` has
    /// `mutate` applied to its entries. Missing intermediate directories
    /// are auto-created as empty dirs.
    fn mutate_at<F>(&self, dir_path: &str, mutate: F) -> Result<()>
    where
        F: FnOnce(&mut Vec<DirEntry>) -> Result<()>,
    {
        let parts = split_path(dir_path)?;
        let root_hash = self.working_root()?;
        let new_root = self.rebuild(root_hash, &parts, mutate)?;
        self.set_working_root(new_root)
    }

    fn rebuild<F>(&self, dir_hash: Hash, parts: &[String], mutate: F) -> Result<Hash>
    where
        F: FnOnce(&mut Vec<DirEntry>) -> Result<()>,
    {
        let mut dir = self.load_dir(&dir_hash)?;
        if parts.is_empty() {
            mutate(&mut dir.entries)?;
            // Keep entries sorted-by-name invariant.
            dir.entries.sort_by(|a, b| a.name.cmp(&b.name));
            dir.hash = Hash::ZERO;
            let (h, _) = seal(&mut dir)?;
            self.meta.put_dir_manifest(&dir)?;
            self.dir_cache.insert(h, dir);
            return Ok(h);
        }
        let head = &parts[0];
        let tail = &parts[1..];
        let (child_hash, existing_idx) = match dir.entries.iter().position(|e| &e.name == head) {
            Some(i) => {
                let e = &dir.entries[i];
                if e.kind != ObjectKind::Dir {
                    return Err(Error::Invalid(format!(
                        "path component '{head}' is a {kind}, expected dir",
                        kind = e.kind
                    )));
                }
                (e.object_hash, Some(i))
            }
            None => (self.empty_dir_hash()?, None),
        };
        let new_child = self.rebuild(child_hash, tail, mutate)?;
        match existing_idx {
            Some(i) => {
                dir.entries[i].object_hash = new_child;
                dir.entries[i].kind = ObjectKind::Dir;
            }
            None => {
                let new_entry = DirEntry {
                    name: head.clone(),
                    object_hash: new_child,
                    kind: ObjectKind::Dir,
                };
                upsert_in_dir(&mut dir.entries, new_entry);
            }
        }
        dir.hash = Hash::ZERO;
        let (h, _) = seal(&mut dir)?;
        self.meta.put_dir_manifest(&dir)?;
        self.dir_cache.insert(h, dir);
        Ok(h)
    }

    fn empty_dir_hash(&self) -> Result<Hash> {
        let mut empty = DirectoryManifest {
            hash: Hash::ZERO,
            entries: Vec::new(),
            xattrs: Vec::new(),
            policy_ref: None,
        };
        let (h, _) = seal(&mut empty)?;
        self.meta.put_dir_manifest(&empty)?;
        self.dir_cache.insert(h, empty);
        Ok(h)
    }
}

/// Insert or replace an entry in a directory's `entries`, preserving
/// sort-by-name order.
fn upsert_in_dir(entries: &mut Vec<DirEntry>, new_entry: DirEntry) {
    match entries.binary_search_by(|e| e.name.cmp(&new_entry.name)) {
        Ok(idx) => entries[idx] = new_entry,
        Err(idx) => entries.insert(idx, new_entry),
    }
}

fn join(parent: &str, name: &str) -> String {
    if parent == "/" {
        format!("/{name}")
    } else {
        format!("{parent}/{name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn tmp_fs() -> (TempDir, Fs) {
        let dir = tempfile::tempdir().unwrap();
        let fs = Fs::init(dir.path()).unwrap();
        (dir, fs)
    }

    #[test]
    fn init_then_open() {
        let dir = tempfile::tempdir().unwrap();
        {
            let _ = Fs::init(dir.path()).unwrap();
        }
        let fs = Fs::open(dir.path()).unwrap();
        assert_eq!(fs.list("/").unwrap(), Vec::<Entry>::new());
    }

    #[test]
    fn init_twice_errors() {
        let dir = tempfile::tempdir().unwrap();
        let _ = Fs::init(dir.path()).unwrap();
        assert!(Fs::init(dir.path()).is_err());
    }

    #[test]
    fn write_and_read_file_at_root() {
        let (_d, fs) = tmp_fs();
        fs.write("/hello.txt", b"hi there").unwrap();
        assert_eq!(fs.read("/hello.txt").unwrap(), b"hi there".to_vec());
    }

    #[test]
    fn write_creates_intermediate_dirs() {
        let (_d, fs) = tmp_fs();
        fs.write("/a/b/c.txt", b"deep").unwrap();
        assert_eq!(fs.read("/a/b/c.txt").unwrap(), b"deep".to_vec());

        let root = fs.list("/").unwrap();
        assert_eq!(root.len(), 1);
        assert_eq!(root[0].path, "/a");
        assert_eq!(root[0].kind, ObjectKind::Dir);

        let a = fs.list("/a").unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].path, "/a/b");
        assert_eq!(a[0].kind, ObjectKind::Dir);
    }

    #[test]
    fn overwrite_same_path_changes_root_hash() {
        let (_d, fs) = tmp_fs();
        fs.write("/x", b"one").unwrap();
        let r1 = fs.working_root().unwrap();
        fs.write("/x", b"two").unwrap();
        let r2 = fs.working_root().unwrap();
        assert_ne!(r1, r2);
        assert_eq!(fs.read("/x").unwrap(), b"two".to_vec());
    }

    #[test]
    fn delete_file_removes_entry() {
        let (_d, fs) = tmp_fs();
        fs.write("/a/b.txt", b"x").unwrap();
        fs.delete("/a/b.txt").unwrap();
        assert!(fs.read("/a/b.txt").is_err());
        let a = fs.list("/a").unwrap();
        assert!(a.is_empty());
    }

    #[test]
    fn delete_non_empty_dir_fails() {
        let (_d, fs) = tmp_fs();
        fs.write("/d/f", b"x").unwrap();
        assert!(fs.delete("/d").is_err());
    }

    #[test]
    fn rename_moves_file() {
        let (_d, fs) = tmp_fs();
        fs.write("/a", b"content").unwrap();
        fs.rename("/a", "/b").unwrap();
        assert_eq!(fs.read("/b").unwrap(), b"content".to_vec());
        assert!(fs.read("/a").is_err());
    }

    #[test]
    fn rename_across_dirs() {
        let (_d, fs) = tmp_fs();
        fs.write("/src/a.txt", b"hi").unwrap();
        fs.rename("/src/a.txt", "/dst/b.txt").unwrap();
        assert_eq!(fs.read("/dst/b.txt").unwrap(), b"hi".to_vec());
    }

    #[test]
    fn mkdir_is_idempotent() {
        let (_d, fs) = tmp_fs();
        fs.mkdir("/x").unwrap();
        fs.mkdir("/x").unwrap();
        let entries = fs.list("/x").unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn dedup_at_chunk_layer() {
        let (_d, fs) = tmp_fs();
        let payload = vec![42u8; 1_000_000];
        fs.write("/a", &payload).unwrap();
        let n_after_first = fs.chunks.iter_hashes().count();
        fs.write("/b", &payload).unwrap();
        let n_after_second = fs.chunks.iter_hashes().count();
        assert_eq!(n_after_first, n_after_second, "identical bytes must dedup");
    }

    #[test]
    fn entries_are_sorted() {
        let (_d, fs) = tmp_fs();
        fs.write("/z", b"1").unwrap();
        fs.write("/a", b"2").unwrap();
        fs.write("/m", b"3").unwrap();
        let listing: Vec<String> = fs.list("/").unwrap().into_iter().map(|e| e.path).collect();
        assert_eq!(listing, vec!["/a", "/m", "/z"]);
    }

    #[test]
    fn concurrent_writes_all_visible() {
        let (_d, fs) = tmp_fs();
        let fs = Arc::new(fs);
        let mut handles = Vec::new();
        for i in 0..8u8 {
            let fs2 = Arc::clone(&fs);
            handles.push(std::thread::spawn(move || {
                fs2.write(&format!("/file-{i}"), &[i; 128]).unwrap();
            }));
        }
        for h in handles { h.join().unwrap(); }
        let entries = fs.list("/").unwrap();
        assert_eq!(entries.len(), 8, "all 8 concurrent writes must be visible");
    }

    #[test]
    fn write_hook_blocks_oversized_write() {
        struct SizeLimit(u64);
        impl WriteHook for SizeLimit {
            fn before_write(&self, _path: &str, bytes_len: u64) -> Result<()> {
                if bytes_len > self.0 {
                    Err(Error::PermissionDenied("quota exceeded".into()))
                } else {
                    Ok(())
                }
            }
        }
        let (_d, fs) = tmp_fs();
        let fs = fs.with_write_hook(Arc::new(SizeLimit(10)));
        assert!(fs.write("/big", &[0u8; 100]).is_err());
        assert!(fs.write("/small", &[0u8; 5]).is_ok());
    }

    #[test]
    fn large_blob_splits_into_multiple_chunks_but_reads_back_intact() {
        let (_d, fs) = tmp_fs();
        // Two chunks' worth. Use a varying pattern so both chunks are unique.
        let mut data = vec![0u8; DEFAULT_CHUNK_SIZE + 100];
        for (i, b) in data.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        fs.write("/big", &data).unwrap();
        assert_eq!(fs.read("/big").unwrap(), data);
    }
}
