//! Mount configuration for the WinFsp driver (T6.1).

use serde::{Deserialize, Serialize};

/// Configuration passed to [`super::WfspMount::new`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WfspConfig {
    /// Windows mount point — either a drive letter (`Z:`) or an
    /// absolute NTFS path (`C:\mnt\atlas`).
    pub mount_point: String,

    /// Volume label shown in Explorer and `vol` output.
    pub volume_label: String,

    /// Expose the volume as read-only to all callers.
    pub read_only: bool,

    /// Emit WinFsp debug spew to stderr.
    pub debug: bool,

    /// Maximum number of concurrent file-system threads.
    /// `0` lets WinFsp choose (recommended).
    #[serde(default)]
    pub worker_threads: u32,

    /// Sector size reported to the kernel (must be a power of two ≥ 512).
    #[serde(default = "default_sector_size")]
    pub sector_size: u32,

    /// Total volume capacity reported via GetDiskFreeSpace.
    /// `0` = auto-derive from the backing store size.
    #[serde(default)]
    pub capacity_bytes: u64,
}

fn default_sector_size() -> u32 { 512 }

impl Default for WfspConfig {
    fn default() -> Self {
        Self {
            mount_point: "Z:".into(),
            volume_label: "ATLAS".into(),
            read_only: false,
            debug: false,
            worker_threads: 0,
            sector_size: 512,
            capacity_bytes: 0,
        }
    }
}
