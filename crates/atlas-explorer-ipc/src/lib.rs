//! IPC types for the ATLAS Explorer Tauri application (T6.6).
//!
//! The Tauri backend exposes these types as JSON-serialised Tauri
//! commands.  The TypeScript side mirrors them in `src/types/ipc.ts`.
//!
//! Design rule: **no business logic here** — this crate is purely a
//! shared type definition layer.  Real logic lives in `atlas_fs`,
//! `atlas_indexer`, `atlas_lineage`, and `atlas_governor`.

pub mod browser;
pub mod lineage;
pub mod policy;
pub mod search;
pub mod version;

pub use browser::{BrowserEntry, BrowserRequest, BrowserResponse};
pub use lineage::{LineageEdgeView, LineageRequest, LineageResponse};
pub use policy::{PolicyRequest, PolicyResponse, PolicyView};
pub use search::{SearchRequest, SearchResponse, SearchResult};
pub use version::{BranchView, CommitView, VersionRequest, VersionResponse};
