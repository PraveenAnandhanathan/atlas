//! ATLAS FUSE adapter — **stub for Phase 0**.
//!
//! Plan task **T0.7**: implement the minimum viable FUSE client on
//! Linux with `getattr`, `readdir`, `open`, `read`, `write`, `create`,
//! `unlink`, `rename`, `setxattr`, `getxattr` so a mounted ATLAS
//! volume passes a meaningful subset of pjdfstest. The full impl
//! lives behind the `linux-fuse` cargo feature once `fuser` is wired.
//!
//! Why a stub now: the workspace must build on macOS and Windows for
//! every contributor, even though FUSE itself is Linux-only. Keeping
//! the crate present (with the surface declared) lets dependent code
//! reference the API stably; the body fills in during Phase 0.

use atlas_fs::Fs;
use std::path::Path;

/// Mount an ATLAS store at `mountpoint` as a FUSE filesystem.
///
/// **Not yet implemented.** This entry point exists so the API
/// surface is stable across the workspace; callers can reference it
/// today and the body lands when the `linux-fuse` feature is enabled
/// (Phase 0 task T0.7).
pub fn mount(_fs: Fs, _mountpoint: impl AsRef<Path>) -> Result<(), MountError> {
    Err(MountError::NotImplemented)
}

#[derive(Debug, thiserror::Error)]
pub enum MountError {
    #[error(
        "FUSE mount is not yet implemented (T0.7); enable the `linux-fuse` feature when it lands"
    )]
    NotImplemented,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_returns_not_implemented() {
        let dir = tempfile::tempdir().unwrap();
        let fs = Fs::init(dir.path()).unwrap();
        let mountpoint = dir.path().join("mnt");
        std::fs::create_dir_all(&mountpoint).unwrap();
        assert!(matches!(
            mount(fs, &mountpoint),
            Err(MountError::NotImplemented)
        ));
    }
}
