//! ATLAS long-form migration tools from S3, GCS, ext4, and git-LFS (T7.8).
//!
//! - [`source`]: parse and enumerate migration sources.
//! - [`pipeline`]: migration pipeline — transfers objects into ATLAS volumes.

pub mod pipeline;
pub mod source;

pub use pipeline::{run, MigrationConfig, MigrationStats, TransferResult};
pub use source::{enumerate, parse_source, MigrationSource, SourceObject};
