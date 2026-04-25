//! Manifest types. Binary layout is frozen by spec v0.1.

use atlas_core::{Author, FormatVersion, Hash, ObjectKind};
use serde::{Deserialize, Serialize};

/// Reference to a chunk inside a blob.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkRef {
    pub hash: Hash,
    /// Must not exceed the volume's chunk size. Last chunk may be shorter.
    pub length: u32,
}

/// Reference to an embedding vector in the index plane (unused in v0.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbeddingRef {
    pub model_id: String,
    pub model_version: String,
    pub vector_ref: Hash,
}

/// A detached signature bundle (unused in v0.1, reserved for Phase 4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signature {
    pub signer_id: String,
    pub algorithm: String,
    pub bytes: Vec<u8>,
}

/// Blob manifest: the ordered chunk list that makes up a file's bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobManifest {
    pub hash: Hash,
    pub total_size: u64,
    pub format_hint: Option<String>,
    pub chunks: Vec<ChunkRef>,
}

/// File manifest: pairs a blob with rich metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileManifest {
    pub hash: Hash,
    pub blob_hash: Hash,
    pub created_at: i64,
    /// POSIX-style mode bits. Default 0o100644 for regular files.
    pub mode: u32,
    /// Sorted by key.
    pub xattrs: Vec<(String, Vec<u8>)>,
    pub embeddings: Vec<EmbeddingRef>,
    pub schema_ref: Option<Hash>,
    pub lineage_ref: Option<Hash>,
    pub policy_ref: Option<Hash>,
    pub signatures: Vec<Signature>,
}

/// One entry in a directory manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirEntry {
    pub name: String,
    pub object_hash: Hash,
    pub kind: ObjectKind,
}

/// Directory manifest: sorted list of entries plus directory-level metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectoryManifest {
    pub hash: Hash,
    /// Sorted by `name`.
    pub entries: Vec<DirEntry>,
    pub xattrs: Vec<(String, Vec<u8>)>,
    pub policy_ref: Option<Hash>,
}

/// Mutable pointer from a logical path to an object hash.
///
/// Unlike the manifests, refs are not content-addressed — they're the
/// single mutable element in the system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefRecord {
    pub path: String,
    pub target: Hash,
    pub updated_at: i64,
}

/// A commit ties a tree snapshot to its parents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Commit {
    pub hash: Hash,
    pub tree_hash: Hash,
    pub parents: Vec<Hash>,
    pub author: Author,
    pub timestamp: i64,
    pub message: String,
    pub signature: Option<Signature>,
}

/// Per-branch protection policy. All fields default false in v0.1.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchProtection {
    pub require_signed: bool,
    pub require_reviewed: bool,
}

/// A named mutable pointer to a commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Branch {
    pub name: String,
    pub head: Hash,
    pub protection: BranchProtection,
}

/// Current HEAD of a store: either a named branch or a detached commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HeadState {
    Branch(String),
    DetachedCommit(Hash),
}

/// Store-level configuration, persisted once at init.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreConfig {
    pub format_version: FormatVersion,
    pub chunk_size: u32,
    pub default_branch: String,
    pub created_at: i64,
}

impl StoreConfig {
    pub fn new() -> Self {
        Self {
            format_version: FormatVersion::CURRENT,
            chunk_size: 4 * 1024 * 1024,
            default_branch: "main".to_string(),
            created_at: atlas_core::now_millis(),
        }
    }
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// Opaque handle used by callers to refer to a blob that hasn't been
/// turned into a file manifest yet (e.g. in streaming writes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blob {
    pub manifest: BlobManifest,
}
