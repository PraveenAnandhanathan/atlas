//! GNOME Virtual File System (GVfs) backend (T6.5).
//!
//! The GVfs daemon loads `libgvfsbackend-atlas.so` at startup.  The
//! `GVfsJobXxx` callbacks translate GIO operations to `VfsCore` calls.
//!
//! The actual C code that subclasses `GVfsBackend` lives in
//! `desktop/gvfs-backend/` and is compiled separately; this module
//! provides the Rust symbols it links against via the C-FFI bridge.

use crate::core::AtlasUri;
use serde::{Deserialize, Serialize};

/// GVfs mount record surfaced to GNOME Shell.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GvfsMount {
    pub display_name: String,
    pub icon_name: String,
    pub symbolic_icon: String,
    pub uri: String,
}

impl GvfsMount {
    pub fn for_volume(volume: &str, host: &str) -> Self {
        Self {
            display_name: format!("ATLAS — {volume}"),
            icon_name: "folder-remote-symbolic".into(),
            symbolic_icon: "folder-remote-symbolic".into(),
            uri: format!("atlas://{host}/{volume}/"),
        }
    }
}

/// GVfs backend info block (used by the `.mount` D-Bus service).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendInfo {
    pub scheme: String,
    pub dbus_name: String,
    pub exec: String,
}

impl BackendInfo {
    pub fn default_info() -> Self {
        Self {
            scheme: "atlas".into(),
            dbus_name: "org.gnome.VfsBackend.Atlas".into(),
            exec: "/usr/lib/gvfs/gvfsd-atlas".into(),
        }
    }
}

/// C-ABI entry points called from `libgvfsbackend-atlas.so`.

/// Return a JSON-serialised `GvfsMount` for `uri`.
///
/// # Safety
/// Must receive valid null-terminated strings; `out` must point to writable memory.
#[no_mangle]
pub unsafe extern "C" fn atlas_gvfs_mount_info(
    uri: *const std::ffi::c_char,
    out: *mut *mut std::ffi::c_char,
) -> i32 {
    if uri.is_null() || out.is_null() { return -1; }
    let uri_str = match std::ffi::CStr::from_ptr(uri).to_str() {
        Ok(s) => s,
        Err(_) => return -1,
    };
    let parsed = match AtlasUri::parse(uri_str) {
        Ok(u) => u,
        Err(_) => return -1,
    };
    let mount = GvfsMount::for_volume(&parsed.volume, &parsed.host);
    let json = match serde_json::to_string(&mount) {
        Ok(j) => j,
        Err(_) => return -1,
    };
    let c_str = match std::ffi::CString::new(json) {
        Ok(s) => s,
        Err(_) => return -1,
    };
    *out = c_str.into_raw();
    0
}

/// Free a string allocated by `atlas_gvfs_mount_info`.
///
/// # Safety
/// Must only be called with pointers returned by this module.
#[no_mangle]
pub unsafe extern "C" fn atlas_gvfs_free_string(ptr: *mut std::ffi::c_char) {
    if !ptr.is_null() {
        drop(std::ffi::CString::from_raw(ptr));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gvfs_mount_display_name() {
        let m = GvfsMount::for_volume("research", "mlbox");
        assert!(m.display_name.contains("research"));
        assert!(m.uri.starts_with("atlas://"));
    }

    #[test]
    fn backend_info_scheme() {
        let info = BackendInfo::default_info();
        assert_eq!(info.scheme, "atlas");
    }
}
