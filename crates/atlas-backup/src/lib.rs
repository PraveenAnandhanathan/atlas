//! ATLAS backup, snapshot export, and cross-region replication (T7.2).
//!
//! Three capabilities:
//!
//! - **Snapshot export** ([`export`]): stream a point-in-time snapshot of
//!   an ATLAS volume to a portable archive format (`.atlas-bundle`), which
//!   can be imported on any ATLAS instance.
//!
//! - **Incremental backup** ([`incremental`]): build a chain of
//!   content-addressed delta bundles; only new chunks since the last backup
//!   are transferred, achieving near-optimal dedup.
//!
//! - **Cross-region replication** ([`replication`]): async, policy-driven
//!   replication of committed snapshots to a remote ATLAS cluster or an S3
//!   compatible bucket; designed for RPO ≤ 15 min and RTO ≤ 30 min.

pub mod export;
pub mod incremental;
pub mod replication;
pub mod schedule;

pub use export::{BundleWriter, ExportConfig, ExportStats};
pub use incremental::{BackupChain, BackupManifest, IncrementalBackup};
pub use replication::{ReplicationConfig, ReplicationTarget, Replicator};
pub use schedule::{BackupSchedule, RetentionPolicy};
