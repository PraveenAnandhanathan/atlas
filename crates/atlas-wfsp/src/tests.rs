//! Unit tests for the WinFsp crate (T6.1).

use crate::config::WfspConfig;
use crate::driver::validate_mount_point_pub;
use crate::ops::{fs_err_to_ntstatus, ntstatus};
use atlas_core::Error as FsError;

#[test]
fn default_config_is_sane() {
    let cfg = WfspConfig::default();
    assert_eq!(cfg.mount_point, "Z:");
    assert_eq!(cfg.volume_label, "ATLAS");
    assert_eq!(cfg.sector_size, 512);
    assert!(!cfg.read_only);
}

#[test]
fn mount_point_validation_drive_letter() {
    assert!(validate_mount_point_pub("Z:").is_ok());
    assert!(validate_mount_point_pub("C:").is_ok());
}

#[test]
fn mount_point_validation_backslash_path() {
    assert!(validate_mount_point_pub(r"\mnt\atlas").is_ok());
}

#[test]
fn mount_point_validation_forward_slash_path() {
    assert!(validate_mount_point_pub("/mnt/atlas").is_ok());
}

#[test]
fn mount_point_validation_rejects_bare_name() {
    assert!(validate_mount_point_pub("noprefix").is_err());
    assert!(validate_mount_point_pub("").is_err());
}

#[test]
fn ntstatus_mapping_not_found() {
    let e = FsError::NotFound("/x".into());
    assert_eq!(fs_err_to_ntstatus(&e), ntstatus::OBJECT_NOT_FOUND);
}

#[test]
fn ntstatus_mapping_already_exists() {
    let e = FsError::AlreadyExists("/x".into());
    assert_eq!(fs_err_to_ntstatus(&e), ntstatus::OBJECT_NAME_COLLISION);
}

#[test]
fn volume_info_default_capacity() {
    use crate::ops::VolumeInfo;
    let vi = VolumeInfo::new("ATLAS", 0);
    assert!(vi.total_bytes > 0);
    assert_eq!(vi.label, "ATLAS");
}

#[test]
fn volume_info_explicit_capacity() {
    use crate::ops::VolumeInfo;
    let vi = VolumeInfo::new("MYLAB", 10 * 1024 * 1024 * 1024);
    assert_eq!(vi.total_bytes, 10 * 1024 * 1024 * 1024);
}
