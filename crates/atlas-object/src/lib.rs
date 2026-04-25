//! ATLAS object model — blob / file / directory manifests, commits, branches.
//!
//! See the authoritative on-disk format in [`docs/spec/v0.1.md`](../../../docs/spec/v0.1.md).
//!
//! All manifests are content-addressed. A manifest's `hash` field is
//! computed from its own canonical serialization with the `hash` slot
//! replaced by `Hash::ZERO`. This lets anyone re-derive and verify the
//! hash without trusting the sender.

pub mod manifest;
pub mod codec;

pub use manifest::{
    Blob, BlobManifest, Branch, BranchProtection, ChunkRef, Commit, DirEntry,
    DirectoryManifest, EmbeddingRef, FileManifest, HeadState, RefRecord, Signature,
    StoreConfig,
};
pub use codec::{hash_manifest, encode, decode, encode_with_zero_hash};
