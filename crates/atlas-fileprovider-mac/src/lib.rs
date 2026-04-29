//! ATLAS macOS FileProvider extension (T6.3) and Quick Look / Finder Sync
//! generators (T6.4).
//!
//! # FileProvider (T6.3)
//!
//! Implements the `NSFileProviderExtension` / `NSFileProviderReplicatedExtension`
//! (macOS 12+) protocol via a Rust core library that a thin Swift wrapper
//! calls into through the C-FFI bridge (`bridge.rs`).
//!
//! The extension lives inside an `.appex` bundle embedded in the main
//! `ATLAS.app`.  When the user enables the extension in System Settings →
//! Privacy & Security → Files and Folders, Finder transparently shows
//! the ATLAS volume alongside iCloud Drive.
//!
//! # Finder Sync (T6.4)
//!
//! `FinderSync.appex` observes the mounted volume path via the macOS
//! Finder Sync API and decorates entries with:
//! - Badge icons (synced / syncing / error).
//! - Toolbar items (*Open in ATLAS Explorer*, *Branch from here*).
//! - Context-menu items mirroring [`atlas_shellext_win::ContextAction`].
//!
//! # Quick Look (T6.4)
//!
//! `ATLASQuickLook.appex` generates rich previews for ATLAS-native formats:
//! - `*.safetensors` → tensor shape table + first-layer histogram.
//! - `*.parquet` → column schema + sample rows.
//! - `*.arrow` → schema + batch summary.
//! - `*.zarr` → hierarchy tree.
//! - `*.embeddings` → 2-D t-SNE scatter (pre-computed).

pub mod bridge;
pub mod fileprovider;
pub mod finder_sync;
pub mod quicklook;

pub use fileprovider::{FileProviderCore, ItemIdentifier, ItemMetadata};
pub use finder_sync::{BadgeKind, FinderSyncCore, ToolbarAction};
pub use quicklook::{preview_bytes, Format, PreviewResult};
