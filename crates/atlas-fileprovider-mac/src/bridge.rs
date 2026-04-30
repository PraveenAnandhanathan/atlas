//! C-FFI bridge for the Swift FileProvider and Finder Sync extensions (T6.3, T6.4).
//!
//! The Swift side calls these `extern "C"` functions; the Rust side
//! holds the real logic.  On non-macOS builds the symbols compile to
//! stubs so the workspace remains green.
//!
//! # Swift side
//!
//! ```swift
//! // Generated header: atlas_fileprovider_mac.h
//! extern fn atlas_register_domain(domain: *const c_char, root_path: *const c_char) -> i32;
//! extern fn atlas_unregister_domain(domain: *const c_char) -> i32;
//! extern fn atlas_enumerate(domain: *const c_char, parent_id: *const c_char,
//!                           out_json: *mut *mut c_char) -> i32;
//! extern fn atlas_fetch(domain: *const c_char, id: *const c_char,
//!                       out_data: *mut *mut u8, out_len: *mut usize) -> i32;
//! extern fn atlas_preview(path: *const c_char, data: *const u8, len: usize,
//!                         out_html: *mut *mut c_char) -> i32;
//! extern fn atlas_free_string(ptr: *mut c_char);
//! extern fn atlas_free_bytes(ptr: *mut u8);
//! ```

use crate::fileprovider::{FileProviderCore, ItemIdentifier};
use atlas_fs::Fs;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::sync::Mutex;

/// C-ABI status codes returned to Swift.
pub mod status {
    pub const OK: i32 = 0;
    pub const NOT_FOUND: i32 = -1;
    pub const ERROR: i32 = -2;
}

static DOMAINS: Mutex<Option<HashMap<String, FileProviderCore>>> = Mutex::new(None);

fn with_domains<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut HashMap<String, FileProviderCore>) -> R,
{
    let mut guard = DOMAINS.lock().ok()?;
    let map = guard.get_or_insert_with(HashMap::new);
    Some(f(map))
}

/// Register an ATLAS volume for a FileProvider domain.
/// `root_path` is the on-disk path to the ATLAS store root.
///
/// # Safety
/// Callers must pass valid null-terminated C strings.
#[no_mangle]
pub unsafe extern "C" fn atlas_register_domain(
    domain: *const std::ffi::c_char,
    root_path: *const std::ffi::c_char,
) -> i32 {
    if domain.is_null() || root_path.is_null() {
        return status::ERROR;
    }
    let domain_str = match CStr::from_ptr(domain).to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return status::ERROR,
    };
    let path_str = match CStr::from_ptr(root_path).to_str() {
        Ok(s) => s,
        Err(_) => return status::ERROR,
    };
    let fs = match Fs::init(std::path::Path::new(path_str)) {
        Ok(f) => f,
        Err(_) => return status::ERROR,
    };
    match with_domains(|m| m.insert(domain_str, FileProviderCore::new(fs))) {
        Some(_) => status::OK,
        None => status::ERROR,
    }
}

/// Unregister a previously registered domain.
///
/// # Safety
/// Callers must pass a valid null-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn atlas_unregister_domain(
    domain: *const std::ffi::c_char,
) -> i32 {
    if domain.is_null() {
        return status::ERROR;
    }
    let domain_str = match CStr::from_ptr(domain).to_str() {
        Ok(s) => s,
        Err(_) => return status::ERROR,
    };
    match with_domains(|m| m.remove(domain_str)) {
        Some(_) => status::OK,
        None => status::ERROR,
    }
}

/// Enumerate children of `parent_id` in `domain`.
/// Writes a JSON array of `ItemMetadata` objects to `*out_json`.
/// The caller must free with `atlas_free_string`.
///
/// # Safety
/// Callers must pass valid null-terminated C strings and a non-null `out_json`.
#[no_mangle]
pub unsafe extern "C" fn atlas_enumerate(
    domain: *const std::ffi::c_char,
    parent_id: *const std::ffi::c_char,
    out_json: *mut *mut std::ffi::c_char,
) -> i32 {
    if domain.is_null() || parent_id.is_null() || out_json.is_null() {
        return status::ERROR;
    }
    let domain_str = match CStr::from_ptr(domain).to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return status::ERROR,
    };
    let parent_str = match CStr::from_ptr(parent_id).to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return status::ERROR,
    };

    let result = with_domains(|m| {
        let core = m.get(&domain_str)?;
        let parent = ItemIdentifier::from_path(&parent_str);
        let items = core.enumerate(&parent).ok()?;
        serde_json::to_string(&items).ok()
    });

    match result.flatten() {
        Some(json) => match CString::new(json) {
            Ok(cs) => {
                *out_json = cs.into_raw();
                status::OK
            }
            Err(_) => status::ERROR,
        },
        None => status::NOT_FOUND,
    }
}

/// Fetch the content bytes for `id` in `domain`.
/// Writes the byte pointer and length to `*out_data`/`*out_len`.
/// The caller must free the buffer with `atlas_free_bytes`.
///
/// # Safety
/// Callers must pass valid pointers.
#[no_mangle]
pub unsafe extern "C" fn atlas_fetch(
    domain: *const std::ffi::c_char,
    id: *const std::ffi::c_char,
    out_data: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    if domain.is_null() || id.is_null() || out_data.is_null() || out_len.is_null() {
        return status::ERROR;
    }
    let domain_str = match CStr::from_ptr(domain).to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return status::ERROR,
    };
    let id_str = match CStr::from_ptr(id).to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return status::ERROR,
    };

    let result = with_domains(|m| {
        let core = m.get(&domain_str)?;
        let item_id = ItemIdentifier::from_path(&id_str);
        core.fetch_content(&item_id).ok()
    });

    match result.flatten() {
        Some(bytes) => {
            let mut boxed = bytes.into_boxed_slice();
            *out_len = boxed.len();
            *out_data = boxed.as_mut_ptr();
            std::mem::forget(boxed);
            status::OK
        }
        None => status::NOT_FOUND,
    }
}

/// Generate an HTML Quick Look preview for the file at `path`.
///
/// # Safety
/// Callers must pass valid pointers.
#[no_mangle]
pub unsafe extern "C" fn atlas_preview(
    path: *const std::ffi::c_char,
    data: *const u8,
    len: usize,
    out_html: *mut *mut std::ffi::c_char,
) -> i32 {
    if path.is_null() || data.is_null() || out_html.is_null() {
        return status::ERROR;
    }
    let path_str = match std::ffi::CStr::from_ptr(path).to_str() {
        Ok(s) => s,
        Err(_) => return status::ERROR,
    };
    let bytes = std::slice::from_raw_parts(data, len);
    let result = crate::quicklook::preview_bytes(path_str, bytes);

    match std::ffi::CString::new(result.html) {
        Ok(s) => {
            *out_html = s.into_raw();
            status::OK
        }
        Err(_) => status::ERROR,
    }
}

/// Free a C string previously allocated by this bridge.
///
/// # Safety
/// Must only be called with pointers returned by this module.
#[no_mangle]
pub unsafe extern "C" fn atlas_free_string(ptr: *mut std::ffi::c_char) {
    if !ptr.is_null() {
        drop(std::ffi::CString::from_raw(ptr));
    }
}

/// Free a byte buffer previously allocated by `atlas_fetch`.
///
/// # Safety
/// `ptr` must have been written by `atlas_fetch` and `len` must match.
#[no_mangle]
pub unsafe extern "C" fn atlas_free_bytes(ptr: *mut u8, len: usize) {
    if !ptr.is_null() && len > 0 {
        drop(Vec::from_raw_parts(ptr, len, len));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_pointers_return_error() {
        unsafe {
            assert_eq!(
                atlas_enumerate(std::ptr::null(), std::ptr::null(), std::ptr::null_mut()),
                status::ERROR
            );
            assert_eq!(
                atlas_fetch(
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut()
                ),
                status::ERROR
            );
        }
    }

    #[test]
    fn unregistered_domain_returns_not_found() {
        use std::ffi::CString;
        let domain = CString::new("nonexistent.domain").unwrap();
        let parent = CString::new("/").unwrap();
        let mut out_json: *mut std::ffi::c_char = std::ptr::null_mut();
        unsafe {
            let rc = atlas_enumerate(domain.as_ptr(), parent.as_ptr(), &mut out_json);
            assert_eq!(rc, status::NOT_FOUND);
        }
    }
}
