//! ATLAS high-level filesystem engine.
//!
//! Ties [`atlas_chunk`], [`atlas_object`], and [`atlas_meta`] into a
//! single API: open a store, put/get files, list directories, rename,
//! delete. Versioning (commits, branches) lives one layer up in
//! `atlas-version`.
//!
//! This crate is the engine that the CLI ([`atlasctl`]) and the FUSE
//! adapter ([`atlas-fuse`]) both wrap.

pub mod engine;
pub mod path;

pub use engine::{Entry, Fs};
pub use path::{normalize_path, split_path};
