//! Metadata plane — a typed view over a pluggable KV backend.
//!
//! Phase 0 ships a single backend, [`SledStore`], per ADR-0003. The
//! trait [`MetaStore`] exists so Phase 2 can introduce a RocksDB
//! backend for single-node throughput and a FoundationDB backend for
//! clusters without touching call sites.
//!
//! Keys follow the schema in spec v0.1 §10. Values are bincode-encoded.

pub mod keys;
pub mod sled_store;
pub mod store;

pub use sled_store::SledStore;
pub use store::{MetaStore, Transaction};
