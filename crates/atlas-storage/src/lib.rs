//! ATLAS storage server library — accepts TCP connections and serves
//! [`ChunkStore`] and [`MetaStore`] requests against local backends.
//!
//! Two modules:
//! - [`server`] owns the listener and per-connection loop.
//! - [`handlers`] dispatches one request to the local stores.
//!
//! Used by both the `atlas-storage` binary and integration tests in
//! `atlas-net` that spin a server up in-process.

pub mod handlers;
pub mod server;

pub use server::{serve, ServerConfig};
