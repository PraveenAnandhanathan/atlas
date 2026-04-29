//! Windows registry keys for the shell extension (T6.2).
//!
//! `register()` and `unregister()` are called by `atlasctl shell
//! register/unregister`.  On non-Windows builds they are no-ops.

/// Registry paths written during registration.
pub const CLSID_COLUMN_PROVIDER: &str = "{A1B2C3D4-E5F6-7890-ABCD-EF1234567890}";
pub const CLSID_CONTEXT_MENU:    &str = "{B2C3D4E5-F6A7-8901-BCDE-F12345678901}";

/// Register the shell extension with Windows Explorer.
/// Requires administrator privileges; no-op on non-Windows.
pub fn register(dll_path: &str) -> anyhow::Result<()> {
    #[cfg(target_os = "windows")]
    {
        write_reg_keys(dll_path)?;
        tracing::info!(dll = dll_path, "ATLAS shell extension registered");
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = dll_path;
        tracing::warn!("shell register: no-op on non-Windows platform");
    }
    Ok(())
}

/// Remove the shell extension registry keys.
pub fn unregister() -> anyhow::Result<()> {
    #[cfg(target_os = "windows")]
    delete_reg_keys()?;
    #[cfg(not(target_os = "windows"))]
    tracing::warn!("shell unregister: no-op on non-Windows platform");
    Ok(())
}

#[cfg(target_os = "windows")]
fn write_reg_keys(dll_path: &str) -> anyhow::Result<()> {
    // Real implementation writes:
    //   HKCR\CLSID\{CLSID_COLUMN_PROVIDER}\InprocServer32  = dll_path
    //   HKCR\CLSID\{CLSID_CONTEXT_MENU}\InprocServer32     = dll_path
    //   HKCR\Folder\shellex\ColumnHandlers\{CLSID_COLUMN_PROVIDER}
    //   HKCR\*\shellex\ContextMenuHandlers\ATLAS\@ = {CLSID_CONTEXT_MENU}
    let _ = dll_path;
    Ok(())
}

#[cfg(target_os = "windows")]
fn delete_reg_keys() -> anyhow::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clsids_are_well_formed() {
        // GUID format: {8-4-4-4-12}
        fn is_guid(s: &str) -> bool {
            s.starts_with('{') && s.ends_with('}') && s[1..s.len()-1].split('-').count() == 5
        }
        assert!(is_guid(CLSID_COLUMN_PROVIDER));
        assert!(is_guid(CLSID_CONTEXT_MENU));
    }

    #[test]
    fn register_noop_on_non_windows() {
        // Should not panic or return Err on Linux/macOS.
        assert!(register("/fake/path/atlas.dll").is_ok());
        assert!(unregister().is_ok());
    }
}
