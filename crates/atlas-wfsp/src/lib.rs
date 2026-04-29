//! ATLAS WinFsp-based Windows filesystem driver (T6.1).
//!
//! Exposes an ATLAS volume as a Windows drive letter (e.g. `A:\`) or an
//! NTFS reparse-point mount (`C:\atlas\vol`).  The driver is a thin
//! shim: every filesystem operation translates into an [`atlas_fs::Fs`]
//! call exactly the same way the FUSE driver does on Linux.
//!
//! # Architecture
//!
//! ```text
//! WinFsp kernel driver
//!        │  (ReadFile / WriteFile / CreateFile / FindFirstFile …)
//!        ▼
//! WinFsp user-mode DLL  ──►  atlas-wfsp (this crate)
//!                                 │
//!                          atlas_fs::Fs  (shared object model)
//!                                 │
//!                          atlas-storage / atlas-meta
//! ```
//!
//! On non-Windows hosts the crate compiles to a no-op shim so the
//! workspace build stays green everywhere; the real implementation
//! activates under `#[cfg(target_os = "windows")]`.
//!
//! # Usage
//!
//! ```no_run
//! use atlas_wfsp::{WfspMount, WfspConfig};
//! use atlas_fs::Fs;
//! use std::path::PathBuf;
//!
//! let fs = Fs::open(PathBuf::from(r"C:\atlas-store")).unwrap();
//! let cfg = WfspConfig {
//!     mount_point: "Z:".into(),
//!     volume_label: "ATLAS".into(),
//!     read_only: false,
//!     debug: false,
//! };
//! let mount = WfspMount::new(fs, cfg).unwrap();
//! mount.run(); // blocks until unmounted
//! ```

pub mod config;
pub mod driver;
pub mod ops;

pub use config::WfspConfig;
pub use driver::{WfspMount, WfspError};

#[cfg(test)]
mod tests;
