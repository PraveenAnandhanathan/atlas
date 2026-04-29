//! ATLAS Windows Shell Extension (T6.2).
//!
//! Provides two COM in-process extension points that Windows Explorer
//! loads from the `atlas-shellext-win.dll`:
//!
//! - **Column provider** ([`columns`]): adds ATLAS-specific columns to
//!   Explorer's Details view — hash, version, lineage depth, policy
//!   tags, branch name, and content-type.
//!
//! - **Context-menu handler** ([`context_menu`]): right-click items on
//!   any file under an ATLAS mount — *Open in ATLAS Explorer*, *Copy
//!   hash*, *Show lineage*, *Commit now*, *Branch from here*.
//!
//! # Registration
//!
//! ```text
//! atlasctl shell register   # writes registry keys, requires admin
//! atlasctl shell unregister # removes them
//! ```
//!
//! On non-Windows hosts the crate compiles to a pure-Rust stub used by
//! integration tests that exercise column formatting logic without a COM
//! runtime.

pub mod columns;
pub mod context_menu;
pub mod registry;

pub use columns::{AtlasColumn, ColumnProvider, ColumnValue};
pub use context_menu::{ContextAction, ContextMenuHandler};
