//! Wire messages. One enum per service so the request/response surface
//! stays explicit and easy to translate to a gRPC `.proto` later.

use atlas_core::Hash;
use atlas_object::{
    BlobManifest, Branch, Commit, DirectoryManifest, FileManifest, HeadState, RefRecord,
    StoreConfig,
};
use serde::{Deserialize, Serialize};

/// Bumped on any breaking wire change. Server rejects requests whose
/// version does not match.
pub const SERVICE_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Request {
    Hello { client_version: u32 },
    Chunk(Box<ChunkRequest>),
    Meta(Box<MetaRequest>),
    Replicate(Box<ReplicateRequest>),
    Goodbye,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Response {
    Hello {
        server_version: u32,
    },
    Chunk(Box<ChunkResponse>),
    Meta(Box<MetaResponse>),
    Replicate(Box<ReplicateResponse>),
    /// Surfaces an error from the server, encoded as a string. Clients
    /// re-wrap into [`atlas_core::Error::Backend`].
    Error {
        message: String,
    },
}

// ---------- ChunkStore ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChunkRequest {
    Put { bytes: Vec<u8> },
    Get { hash: Hash },
    Delete { hash: Hash },
    Has { hash: Hash },
    Verify { hash: Hash },
    Size { hash: Hash },
    IterHashes,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChunkResponse {
    Put { hash: Hash },
    Get { bytes: Vec<u8> },
    Delete,
    Has { exists: bool },
    Verify,
    Size { bytes: u64 },
    IterHashes { hashes: Vec<Hash> },
}

// ---------- MetaStore ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MetaRequest {
    GetRaw { key: String },
    PutRaw { key: String, value: Vec<u8> },
    Delete { key: String },
    ScanPrefix { prefix: String },
    ApplyBatch { ops: Vec<BatchOp> },

    GetBlobManifest { hash: Hash },
    PutBlobManifest { manifest: BlobManifest },
    GetFileManifest { hash: Hash },
    PutFileManifest { manifest: FileManifest },
    GetDirManifest { hash: Hash },
    PutDirManifest { manifest: DirectoryManifest },
    GetCommit { hash: Hash },
    PutCommit { commit: Commit },
    GetRef { path: String },
    PutRef { record: RefRecord },
    DeleteRef { path: String },
    GetBranch { name: String },
    PutBranch { branch: Branch },
    ListBranches,
    GetHead,
    PutHead { head: HeadState },
    GetConfig,
    PutConfig { config: StoreConfig },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BatchOp {
    Put { key: String, value: Vec<u8> },
    Delete { key: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MetaResponse {
    OptBytes { value: Option<Vec<u8>> },
    Bool { value: bool },
    Empty,
    Pairs { pairs: Vec<(String, Vec<u8>)> },

    OptBlobManifest { manifest: Option<BlobManifest> },
    OptFileManifest { manifest: Option<FileManifest> },
    OptDirManifest { manifest: Option<DirectoryManifest> },
    OptCommit { commit: Option<Commit> },
    OptRef { record: Option<RefRecord> },
    OptBranch { branch: Option<Branch> },
    Branches { branches: Vec<Branch> },
    OptHead { head: Option<HeadState> },
    OptConfig { config: Option<StoreConfig> },
}

// ---------- Chain replication (CRAQ) ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReplicateRequest {
    /// Forwarded along the chain. The receiver stores the chunk locally
    /// and forwards to its successor (or acks if it is the tail).
    PropagateChunk { bytes: Vec<u8>, sequence: u64 },
    /// Tail-style read: caller is asking the tail node which copy is
    /// considered "clean" for `hash`.
    ReadCommitted { hash: Hash },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReplicateResponse {
    Ack { hash: Hash, sequence: u64 },
    ReadCommitted { bytes: Vec<u8> },
}
