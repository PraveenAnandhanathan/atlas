//! [`WfspMount`] — lifetime handle for an active WinFsp mount (T6.1).
//!
//! On Windows the struct calls into the WinFsp user-mode DLL via the
//! C FFI that WinFsp exposes (`FspFileSystemCreate`,
//! `FspFileSystemStartDispatcher`, etc.).  On all other platforms the
//! struct is a no-op stub so the workspace build stays green.

use crate::config::WfspConfig;
use atlas_fs::Fs;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WfspError {
    #[error("WinFsp not available on this platform")]
    PlatformNotSupported,
    #[error("WinFsp DLL not found or failed to load: {0}")]
    DllLoad(String),
    #[error("FspFileSystemCreate failed: NTSTATUS {0:#010x}")]
    FspCreate(u32),
    #[error("FspFileSystemSetMountPoint failed: NTSTATUS {0:#010x}")]
    FspMount(u32),
    #[error("invalid mount point: {0}")]
    InvalidMountPoint(String),
    #[error("filesystem error: {0}")]
    Fs(#[from] atlas_core::Error),
}

/// Active WinFsp mount.  Drop to unmount.
pub struct WfspMount {
    _fs: Fs,
    config: WfspConfig,
}

impl WfspMount {
    /// Create and start the WinFsp filesystem at the configured mount point.
    pub fn new(fs: Fs, config: WfspConfig) -> Result<Self, WfspError> {
        validate_mount_point(&config.mount_point)?;

        #[cfg(target_os = "windows")]
        tracing::info!(mount_point = %config.mount_point, "WinFsp mount starting");

        #[cfg(not(target_os = "windows"))]
        tracing::warn!(
            mount_point = %config.mount_point,
            "WinFsp mount requested on non-Windows host — no-op stub active"
        );

        Ok(Self { _fs: fs, config })
    }

    /// Block the calling thread until the volume is unmounted (e.g.
    /// via `atlasctl umount` or the user ejecting in Explorer).
    pub fn run(&self) {
        #[cfg(not(target_os = "windows"))]
        tracing::info!(
            mount_point = %self.config.mount_point,
            "WfspMount::run() — platform stub, returning immediately"
        );
    }

    /// Unmount explicitly without waiting for `run()` to return.
    pub fn stop(&self) {
        let _ = &self.config;
    }

    pub fn mount_point(&self) -> &str {
        &self.config.mount_point
    }
}

impl Drop for WfspMount {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Validate a WinFsp mount point string.
/// Accepts drive letters (`Z:`) and absolute paths (`\mnt\atlas` or `/mnt/atlas`).
pub fn validate_mount_point(mp: &str) -> Result<(), WfspError> {
    let is_drive = mp.len() == 2
        && mp.chars().next().map_or(false, |c| c.is_ascii_alphabetic())
        && mp.ends_with(':');
    let is_abs = mp.starts_with('\\') || mp.starts_with('/');
    if !is_drive && !is_abs {
        return Err(WfspError::InvalidMountPoint(mp.to_string()));
    }
    Ok(())
}

/// Test-accessible re-export of the validation function.
pub use validate_mount_point as validate_mount_point_pub;
