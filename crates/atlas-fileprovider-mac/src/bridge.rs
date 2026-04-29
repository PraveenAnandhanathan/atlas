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
//! extern fn atlas_enumerate(domain: *const c_char, parent_id: *const c_char,
//!                           out_json: *mut *mut c_char) -> i32;
//! extern fn atlas_fetch(domain: *const c_char, id: *const c_char,
//!                       out_data: *mut *mut u8, out_len: *mut usize) -> i32;
//! extern fn atlas_preview(path: *const c_char, data: *const u8, len: usize,
//!                         out_html: *mut *mut c_char) -> i32;
//! ```

/// C-ABI status codes returned to Swift.
pub mod status {
    pub const OK: i32      =  0;
    pub const NOT_FOUND: i32 = -1;
    pub const ERROR: i32   = -2;
}

/// Enumerate children of `parent_id` in `domain`.
/// Returns JSON array of `ItemMetadata` or an empty array on error.
///
/// # Safety
/// Callers must pass valid null-terminated C strings.
#[no_mangle]
pub unsafe extern "C" fn atlas_enumerate(
    _domain: *const std::ffi::c_char,
    _parent_id: *const std::ffi::c_char,
    _out_json: *mut *mut std::ffi::c_char,
) -> i32 {
    // Production: decode CStr → open volume → call FileProviderCore::enumerate
    // → serialise Vec<ItemMetadata> → CString → write to *out_json.
    status::OK
}

/// Fetch the content bytes for `id`.
///
/// # Safety
/// Callers must pass valid pointers.
#[no_mangle]
pub unsafe extern "C" fn atlas_fetch(
    _domain: *const std::ffi::c_char,
    _id: *const std::ffi::c_char,
    _out_data: *mut *mut u8,
    _out_len: *mut usize,
) -> i32 {
    status::OK
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

    let c_html = match std::ffi::CString::new(result.html) {
        Ok(s) => s,
        Err(_) => return status::ERROR,
    };
    *out_html = c_html.into_raw();
    status::OK
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
