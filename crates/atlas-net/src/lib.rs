//! Network adapters that implement [`atlas_chunk::ChunkStore`] and
//! [`atlas_meta::MetaStore`] over the wire protocol defined in
//! [`atlas_proto`].
//!
//! Phase 2 ships a single TCP transport backed by tokio. The traits are
//! synchronous (because that's what `atlas-fs` calls), so the clients
//! own a multi-threaded tokio runtime internally and `block_on` for each
//! request. A purely-async surface lands in Phase 3 once the engine is
//! ported.

pub mod chunk_client;
pub mod meta_client;
pub mod runtime;

pub use chunk_client::RemoteChunkStore;
pub use meta_client::RemoteMetaStore;
pub use runtime::ClientRuntime;
