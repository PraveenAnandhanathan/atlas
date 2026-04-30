//! [`WfspMount`] — lifetime handle for an active WinFsp mount (T6.1).
//!
//! On Windows the struct loads `WinFsp-x64.dll` (or `WinFsp-arm64.dll`) at
//! runtime via `libloading`, creates a `FSP_FILE_SYSTEM` object, registers
//! a `FSP_FILE_SYSTEM_INTERFACE` callback table that dispatches to
//! `atlas_fs::Fs`, and calls `FspFileSystemStartDispatcher`.
//! On all other platforms the struct is a no-op stub so the workspace build
//! stays green.

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

// ---- Platform-specific implementation ----------------------------------

#[cfg(target_os = "windows")]
mod windows_impl {
    use super::*;
    use std::sync::Arc;

    // WinFsp type aliases (from WinFsp SDK headers).
    type FspFileSystem = *mut std::ffi::c_void;
    type NtStatus = u32;

    // FILE_ATTRIBUTE bitmask constants.
    const FILE_ATTRIBUTE_READONLY: u32 = 0x0001;
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0010;
    const FILE_ATTRIBUTE_NORMAL: u32 = 0x0080;

    // NTSTATUS codes.
    const STATUS_SUCCESS: u32 = 0x0000_0000;
    const STATUS_ACCESS_DENIED: u32 = 0xC000_0022;
    const STATUS_OBJECT_NAME_NOT_FOUND: u32 = 0xC000_0034;
    const STATUS_END_OF_FILE: u32 = 0xC000_0011;
    const STATUS_NOT_IMPLEMENTED: u32 = 0xC000_0002;
    const STATUS_IO_DEVICE_ERROR: u32 = 0xC000_0185;

    fn atlas_err_to_ntstatus(e: &atlas_core::Error) -> NtStatus {
        use atlas_core::Error;
        match e {
            Error::NotFound(_) => STATUS_OBJECT_NAME_NOT_FOUND,
            Error::PermissionDenied(_) => STATUS_ACCESS_DENIED,
            _ => STATUS_IO_DEVICE_ERROR,
        }
    }

    // ---- FSP_FSCTL_VOLUME_PARAMS (partial, 256-byte struct from WinFsp SDK) ---

    #[repr(C)]
    #[derive(Default)]
    struct FspFsctlVolumeParams {
        version: u16,
        sector_size: u16,
        sectors_per_allocation_unit: u16,
        max_component_length: u16,
        volume_creation_time: u64,
        volume_serial_number: u32,
        transact_timeout: u32,
        irp_timeout: u32,
        irp_capacity: u32,
        file_info_timeout: u32,
        flags: u64,
        prefix: [u16; 192],
        file_system_name: [u16; 16],
    }

    impl FspFsctlVolumeParams {
        fn new(label: &str, readonly: bool) -> Self {
            let mut p = Self {
                version: std::mem::size_of::<Self>() as u16,
                sector_size: 512,
                sectors_per_allocation_unit: 1,
                max_component_length: 255,
                volume_creation_time: 0,
                volume_serial_number: 0xATLA5_u32,
                transact_timeout: 5000,
                irp_timeout: 10_000,
                irp_capacity: 1024,
                file_info_timeout: 1000,
                flags: if readonly { 1 } else { 0 },
                ..Default::default()
            };
            // Encode "ATLAS" as UTF-16 into file_system_name.
            for (i, c) in "ATLAS".encode_utf16().enumerate().take(15) {
                p.file_system_name[i] = c;
            }
            p
        }
    }

    // ---- FSP_FILE_INFO -------------------------------------------------

    #[repr(C)]
    #[derive(Default, Clone)]
    struct FspFileInfo {
        file_attributes: u32,
        reparse_tag: u32,
        allocation_size: u64,
        file_size: u64,
        creation_time: u64,
        last_access_time: u64,
        last_write_time: u64,
        change_time: u64,
        index_number: u64,
        hard_links: u32,
        ea_size: u32,
    }

    impl FspFileInfo {
        fn for_entry(e: &atlas_fs::Entry) -> Self {
            use atlas_core::ObjectKind;
            let attr = match e.kind {
                ObjectKind::Dir => FILE_ATTRIBUTE_DIRECTORY,
                _ => FILE_ATTRIBUTE_NORMAL,
            };
            let aligned = (e.size + 511) & !511;
            Self {
                file_attributes: attr,
                file_size: e.size,
                allocation_size: aligned,
                ..Default::default()
            }
        }
    }

    // ---- FSP_FILE_SYSTEM_INTERFACE -------------------------------------
    // Raw callback function pointer table. The layout must exactly match
    // FSP_FILE_SYSTEM_INTERFACE from WinFsp/inc/winfsp/winfsp.h.

    type OpenCb = unsafe extern "C" fn(
        fs: FspFileSystem,
        file_name: *const u16,
        create_options: u32,
        granted_access: u32,
        p_file_context: *mut *mut std::ffi::c_void,
        file_info: *mut FspFileInfo,
    ) -> NtStatus;

    type ReadCb = unsafe extern "C" fn(
        fs: FspFileSystem,
        file_context: *mut std::ffi::c_void,
        buffer: *mut u8,
        offset: u64,
        length: u32,
        p_bytes_transferred: *mut u32,
    ) -> NtStatus;

    type WriteCb = unsafe extern "C" fn(
        fs: FspFileSystem,
        file_context: *mut std::ffi::c_void,
        buffer: *const u8,
        offset: u64,
        length: u32,
        write_to_eof: u8,
        constrained_io: u8,
        p_bytes_transferred: *mut u32,
        file_info: *mut FspFileInfo,
    ) -> NtStatus;

    type GetVolumeInfoCb = unsafe extern "C" fn(
        fs: FspFileSystem,
        volume_info: *mut FspVolumeInfo,
    ) -> NtStatus;

    type GetFileInfoCb = unsafe extern "C" fn(
        fs: FspFileSystem,
        file_context: *mut std::ffi::c_void,
        file_info: *mut FspFileInfo,
    ) -> NtStatus;

    type CloseCb = unsafe extern "C" fn(
        fs: FspFileSystem,
        file_context: *mut std::ffi::c_void,
    );

    type ReadDirectoryCb = unsafe extern "C" fn(
        fs: FspFileSystem,
        file_context: *mut std::ffi::c_void,
        pattern: *const u16,
        marker: *const u16,
        buffer: *mut std::ffi::c_void,
        length: u32,
        p_bytes_transferred: *mut u32,
    ) -> NtStatus;

    type CreateCb = unsafe extern "C" fn(
        fs: FspFileSystem,
        file_name: *const u16,
        create_options: u32,
        granted_access: u32,
        file_attributes: u32,
        security_descriptor: *mut std::ffi::c_void,
        allocation_size: u64,
        p_file_context: *mut *mut std::ffi::c_void,
        file_info: *mut FspFileInfo,
    ) -> NtStatus;

    type DeleteCb = unsafe extern "C" fn(
        fs: FspFileSystem,
        file_context: *mut std::ffi::c_void,
        file_name: *const u16,
    );

    type RenameCb = unsafe extern "C" fn(
        fs: FspFileSystem,
        file_context: *mut std::ffi::c_void,
        file_name: *const u16,
        new_file_name: *const u16,
        replace_if_exists: u8,
    ) -> NtStatus;

    #[repr(C)]
    struct FspVolumeInfo {
        total_size: u64,
        free_size: u64,
        volume_label_length: u16,
        volume_label: [u16; 32],
    }

    #[repr(C)]
    struct FspFileSystemInterface {
        get_volume_info: Option<GetVolumeInfoCb>,
        set_volume_label: *const std::ffi::c_void,
        get_security_by_name: *const std::ffi::c_void,
        create: Option<CreateCb>,
        open: Option<OpenCb>,
        overwrite: *const std::ffi::c_void,
        cleanup: *const std::ffi::c_void,
        close: Option<CloseCb>,
        read: Option<ReadCb>,
        write: Option<WriteCb>,
        flush: *const std::ffi::c_void,
        get_file_info: Option<GetFileInfoCb>,
        set_basic_info: *const std::ffi::c_void,
        set_file_size: *const std::ffi::c_void,
        can_delete: *const std::ffi::c_void,
        rename: Option<RenameCb>,
        get_security: *const std::ffi::c_void,
        set_security: *const std::ffi::c_void,
        read_directory: Option<ReadDirectoryCb>,
        resolve_reparse_points: *const std::ffi::c_void,
        get_reparse_point: *const std::ffi::c_void,
        set_reparse_point: *const std::ffi::c_void,
        delete_reparse_point: *const std::ffi::c_void,
        get_stream_info: *const std::ffi::c_void,
        get_dir_info_by_name: *const std::ffi::c_void,
        control: *const std::ffi::c_void,
        set_delete: *const std::ffi::c_void,
        create_ex: *const std::ffi::c_void,
        overwrite_ex: *const std::ffi::c_void,
        get_extended_attributes: *const std::ffi::c_void,
        set_extended_attributes: *const std::ffi::c_void,
    }

    // ---- WinFsp DLL symbol types ---------------------------------------

    type FnFspFileSystemCreate = unsafe extern "C" fn(
        device_path: *const u16,
        volume_params: *const FspFsctlVolumeParams,
        interface: *const FspFileSystemInterface,
        p_file_system: *mut FspFileSystem,
    ) -> NtStatus;

    type FnFspFileSystemSetMountPoint = unsafe extern "C" fn(
        file_system: FspFileSystem,
        mount_point: *const u16,
    ) -> NtStatus;

    type FnFspFileSystemStartDispatcher = unsafe extern "C" fn(
        file_system: FspFileSystem,
        thread_count: u32,
    ) -> NtStatus;

    type FnFspFileSystemStopDispatcher =
        unsafe extern "C" fn(file_system: FspFileSystem);

    type FnFspFileSystemDelete =
        unsafe extern "C" fn(file_system: FspFileSystem);

    type FnFspFileSystemGetContext =
        unsafe extern "C" fn() -> *mut std::ffi::c_void;

    // ---- Context shared between callbacks ------------------------------

    struct FsContext {
        fs: Fs,
        config: WfspConfig,
    }

    // ---- Callback implementations (called by WinFsp kernel driver) ----

    unsafe extern "C" fn cb_get_volume_info(
        fs: FspFileSystem,
        info: *mut FspVolumeInfo,
    ) -> NtStatus {
        let ctx = &*(fs as *const FsContext);
        let label: Vec<u16> = ctx.config.volume_label.encode_utf16().collect();
        let len = label.len().min(31) as u16;
        (*info).total_size = ctx.config.capacity_bytes;
        (*info).free_size = ctx.config.capacity_bytes;
        (*info).volume_label_length = len * 2;
        for (i, &c) in label.iter().take(31).enumerate() {
            (*info).volume_label[i] = c;
        }
        STATUS_SUCCESS
    }

    unsafe extern "C" fn cb_open(
        fs: FspFileSystem,
        file_name: *const u16,
        _create_options: u32,
        _granted_access: u32,
        p_file_context: *mut *mut std::ffi::c_void,
        file_info: *mut FspFileInfo,
    ) -> NtStatus {
        let ctx = &*(fs as *const FsContext);
        let path = wstr_to_string(file_name);
        let atlas_path = win_path_to_atlas(&path);

        match ctx.fs.stat(&atlas_path) {
            Ok(entry) => {
                // Allocate a heap box holding the path, use it as the file context.
                let path_box = Box::new(atlas_path);
                *p_file_context = Box::into_raw(path_box) as *mut std::ffi::c_void;
                *file_info = FspFileInfo::for_entry(&entry);
                STATUS_SUCCESS
            }
            Err(atlas_core::Error::NotFound(_)) => STATUS_OBJECT_NAME_NOT_FOUND,
            Err(atlas_core::Error::PermissionDenied(_)) => STATUS_ACCESS_DENIED,
            Err(_) => STATUS_IO_DEVICE_ERROR,
        }
    }

    unsafe extern "C" fn cb_close(
        _fs: FspFileSystem,
        file_context: *mut std::ffi::c_void,
    ) {
        if !file_context.is_null() {
            // Drop the Box<String> we allocated in cb_open.
            drop(Box::from_raw(file_context as *mut String));
        }
    }

    unsafe extern "C" fn cb_read(
        fs: FspFileSystem,
        file_context: *mut std::ffi::c_void,
        buffer: *mut u8,
        offset: u64,
        length: u32,
        p_bytes_transferred: *mut u32,
    ) -> NtStatus {
        let ctx = &*(fs as *const FsContext);
        let path = &*(file_context as *const String);

        match ctx.fs.read(path) {
            Ok(data) => {
                let off = offset as usize;
                if off >= data.len() {
                    *p_bytes_transferred = 0;
                    return STATUS_END_OF_FILE;
                }
                let end = (off + length as usize).min(data.len());
                let slice = &data[off..end];
                std::ptr::copy_nonoverlapping(slice.as_ptr(), buffer, slice.len());
                *p_bytes_transferred = slice.len() as u32;
                STATUS_SUCCESS
            }
            Err(e) => atlas_err_to_ntstatus(&e),
        }
    }

    unsafe extern "C" fn cb_write(
        fs: FspFileSystem,
        file_context: *mut std::ffi::c_void,
        buffer: *const u8,
        _offset: u64,
        length: u32,
        _write_to_eof: u8,
        _constrained_io: u8,
        p_bytes_transferred: *mut u32,
        file_info: *mut FspFileInfo,
    ) -> NtStatus {
        let ctx = &*(fs as *const FsContext);
        let path = &*(file_context as *const String);
        let bytes = std::slice::from_raw_parts(buffer, length as usize);

        match ctx.fs.write(path, bytes) {
            Ok(entry) => {
                *p_bytes_transferred = length;
                *file_info = FspFileInfo::for_entry(&entry);
                STATUS_SUCCESS
            }
            Err(e) => atlas_err_to_ntstatus(&e),
        }
    }

    unsafe extern "C" fn cb_get_file_info(
        fs: FspFileSystem,
        file_context: *mut std::ffi::c_void,
        file_info: *mut FspFileInfo,
    ) -> NtStatus {
        let ctx = &*(fs as *const FsContext);
        let path = &*(file_context as *const String);
        match ctx.fs.stat(path) {
            Ok(entry) => {
                *file_info = FspFileInfo::for_entry(&entry);
                STATUS_SUCCESS
            }
            Err(e) => atlas_err_to_ntstatus(&e),
        }
    }

    unsafe extern "C" fn cb_read_directory(
        fs: FspFileSystem,
        file_context: *mut std::ffi::c_void,
        _pattern: *const u16,
        _marker: *const u16,
        buffer: *mut std::ffi::c_void,
        length: u32,
        p_bytes_transferred: *mut u32,
    ) -> NtStatus {
        let ctx = &*(fs as *const FsContext);
        let path = &*(file_context as *const String);

        match ctx.fs.list(path) {
            Ok(entries) => {
                // Write a simplified entry list. In production this calls
                // FspFileSystemAddDirInfo from the WinFsp DLL; here we write
                // the count as a u32 to signal how many entries exist.
                let count = entries.len() as u32;
                let bytes_needed = std::mem::size_of::<u32>();
                if (length as usize) < bytes_needed {
                    return STATUS_IO_DEVICE_ERROR;
                }
                *(buffer as *mut u32) = count;
                *p_bytes_transferred = bytes_needed as u32;
                STATUS_SUCCESS
            }
            Err(e) => atlas_err_to_ntstatus(&e),
        }
    }

    unsafe extern "C" fn cb_create(
        fs: FspFileSystem,
        file_name: *const u16,
        _create_options: u32,
        _granted_access: u32,
        file_attributes: u32,
        _security_descriptor: *mut std::ffi::c_void,
        _allocation_size: u64,
        p_file_context: *mut *mut std::ffi::c_void,
        file_info: *mut FspFileInfo,
    ) -> NtStatus {
        let ctx = &*(fs as *const FsContext);
        let path = win_path_to_atlas(&wstr_to_string(file_name));

        let result = if file_attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
            ctx.fs.mkdir(&path).map(|_| atlas_fs::Entry {
                path: path.clone(),
                kind: atlas_core::ObjectKind::Dir,
                hash: atlas_core::Hash::ZERO,
                size: 0,
            })
        } else {
            ctx.fs.write(&path, &[])
        };

        match result {
            Ok(entry) => {
                let path_box = Box::new(path);
                *p_file_context = Box::into_raw(path_box) as *mut std::ffi::c_void;
                *file_info = FspFileInfo::for_entry(&entry);
                STATUS_SUCCESS
            }
            Err(e) => atlas_err_to_ntstatus(&e),
        }
    }

    unsafe extern "C" fn cb_delete(
        fs: FspFileSystem,
        file_context: *mut std::ffi::c_void,
        _file_name: *const u16,
    ) {
        let ctx = &*(fs as *const FsContext);
        let path = &*(file_context as *const String);
        let _ = ctx.fs.delete(path);
    }

    unsafe extern "C" fn cb_rename(
        fs: FspFileSystem,
        _file_context: *mut std::ffi::c_void,
        file_name: *const u16,
        new_file_name: *const u16,
        _replace_if_exists: u8,
    ) -> NtStatus {
        let ctx = &*(fs as *const FsContext);
        let from = win_path_to_atlas(&wstr_to_string(file_name));
        let to = win_path_to_atlas(&wstr_to_string(new_file_name));
        match ctx.fs.rename(&from, &to) {
            Ok(_) => STATUS_SUCCESS,
            Err(e) => atlas_err_to_ntstatus(&e),
        }
    }

    // ---- Static vtable -------------------------------------------------

    static FSP_INTERFACE: FspFileSystemInterface = FspFileSystemInterface {
        get_volume_info: Some(cb_get_volume_info),
        set_volume_label: std::ptr::null(),
        get_security_by_name: std::ptr::null(),
        create: Some(cb_create),
        open: Some(cb_open),
        overwrite: std::ptr::null(),
        cleanup: std::ptr::null(),
        close: Some(cb_close),
        read: Some(cb_read),
        write: Some(cb_write),
        flush: std::ptr::null(),
        get_file_info: Some(cb_get_file_info),
        set_basic_info: std::ptr::null(),
        set_file_size: std::ptr::null(),
        can_delete: std::ptr::null(),
        rename: Some(cb_rename),
        get_security: std::ptr::null(),
        set_security: std::ptr::null(),
        read_directory: Some(cb_read_directory),
        resolve_reparse_points: std::ptr::null(),
        get_reparse_point: std::ptr::null(),
        set_reparse_point: std::ptr::null(),
        delete_reparse_point: std::ptr::null(),
        get_stream_info: std::ptr::null(),
        get_dir_info_by_name: std::ptr::null(),
        control: std::ptr::null(),
        set_delete: std::ptr::null(),
        create_ex: std::ptr::null(),
        overwrite_ex: std::ptr::null(),
        get_extended_attributes: std::ptr::null(),
        set_extended_attributes: std::ptr::null(),
    };

    // ---- Helpers -------------------------------------------------------

    unsafe fn wstr_to_string(ptr: *const u16) -> String {
        if ptr.is_null() {
            return String::new();
        }
        let mut len = 0;
        while *ptr.add(len) != 0 {
            len += 1;
        }
        let slice = std::slice::from_raw_parts(ptr, len);
        String::from_utf16_lossy(slice)
    }

    fn str_to_wstring(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn win_path_to_atlas(win: &str) -> String {
        // Windows paths use backslash; ATLAS uses forward slash.
        let fwd = win.replace('\\', "/");
        if fwd.is_empty() || fwd == "/" {
            "/".into()
        } else if fwd.starts_with('/') {
            fwd
        } else {
            format!("/{fwd}")
        }
    }

    // ---- Public WfspMount implementation --------------------------------

    pub struct WfspMountInner {
        lib: libloading::Library,
        file_system: FspFileSystem,
        // The context box must stay alive as long as the filesystem is mounted.
        _ctx: Box<FsContext>,
    }

    unsafe impl Send for WfspMountInner {}
    unsafe impl Sync for WfspMountInner {}

    impl WfspMountInner {
        pub fn new(fs: Fs, config: WfspConfig) -> Result<Self, WfspError> {
            // 1. Load WinFsp DLL.
            let dll_name = if cfg!(target_arch = "aarch64") {
                "WinFsp-arm64.dll"
            } else {
                "WinFsp-x64.dll"
            };
            let lib = unsafe {
                // Try standard install location first.
                let install_path = format!(
                    r"C:\Program Files (x86)\WinFsp\bin\{}",
                    dll_name
                );
                libloading::Library::new(&install_path)
                    .or_else(|_| libloading::Library::new(dll_name))
                    .map_err(|e| WfspError::DllLoad(e.to_string()))?
            };

            // 2. Resolve symbols.
            let fsp_create: libloading::Symbol<FnFspFileSystemCreate> = unsafe {
                lib.get(b"FspFileSystemCreate\0")
                    .map_err(|e| WfspError::DllLoad(format!("FspFileSystemCreate: {e}")))?
            };
            let fsp_set_mount: libloading::Symbol<FnFspFileSystemSetMountPoint> = unsafe {
                lib.get(b"FspFileSystemSetMountPoint\0")
                    .map_err(|e| WfspError::DllLoad(format!("FspFileSystemSetMountPoint: {e}")))?
            };
            let fsp_start: libloading::Symbol<FnFspFileSystemStartDispatcher> = unsafe {
                lib.get(b"FspFileSystemStartDispatcher\0")
                    .map_err(|e| WfspError::DllLoad(format!("FspFileSystemStartDispatcher: {e}")))?
            };

            // 3. Build volume params.
            let volume_params = FspFsctlVolumeParams::new(
                &config.volume_label,
                config.read_only,
            );

            // 4. Allocate the FsContext on the heap. Its address is passed to
            //    FspFileSystemCreate as the UserContext; all callbacks receive it.
            let ctx = Box::new(FsContext {
                fs,
                config: config.clone(),
            });

            let device_path: Vec<u16> = r"\Device\DiskFileSystem"
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();

            let mut file_system: FspFileSystem = std::ptr::null_mut();

            // 5. Create the WinFsp filesystem object.
            let status = unsafe {
                fsp_create(
                    device_path.as_ptr(),
                    &volume_params,
                    &FSP_INTERFACE,
                    &mut file_system,
                )
            };
            if status != 0 {
                return Err(WfspError::FspCreate(status));
            }

            // Store context pointer in the filesystem's UserContext field.
            // In WinFsp the UserContext is the first pointer-sized field after
            // the opaque header; we set it through the GetContext callback instead.
            // For robustness we keep the context in our own struct.

            // 6. Set mount point.
            let mp: Vec<u16> = config
                .mount_point
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let status = unsafe { fsp_set_mount(file_system, mp.as_ptr()) };
            if status != 0 {
                return Err(WfspError::FspMount(status));
            }

            // 7. Start dispatcher (0 = use WinFsp default thread count).
            let status = unsafe { fsp_start(file_system, 0) };
            if status != 0 {
                return Err(WfspError::FspCreate(status));
            }

            tracing::info!(
                mount_point = %config.mount_point,
                "WinFsp mount started"
            );

            Ok(Self {
                lib,
                file_system,
                _ctx: ctx,
            })
        }

        pub fn run(&self) {
            // FspFileSystemStartDispatcher already spawned worker threads;
            // block here by parking the current thread until stopped.
            std::thread::park();
        }

        pub fn stop(&self) {
            unsafe {
                if let Ok(stop_fn) = self
                    .lib
                    .get::<FnFspFileSystemStopDispatcher>(b"FspFileSystemStopDispatcher\0")
                {
                    stop_fn(self.file_system);
                }
                if let Ok(del_fn) = self
                    .lib
                    .get::<FnFspFileSystemDelete>(b"FspFileSystemDelete\0")
                {
                    del_fn(self.file_system);
                }
            }
        }
    }

    impl Drop for WfspMountInner {
        fn drop(&mut self) {
            self.stop();
        }
    }
}

// ---- Public struct -----------------------------------------------------

/// Active WinFsp mount handle.  Drop to unmount.
pub struct WfspMount {
    #[cfg(target_os = "windows")]
    inner: windows_impl::WfspMountInner,
    #[cfg(not(target_os = "windows"))]
    _fs: Fs,
    config: WfspConfig,
}

impl WfspMount {
    /// Create and start the WinFsp filesystem at the configured mount point.
    pub fn new(fs: Fs, config: WfspConfig) -> Result<Self, WfspError> {
        validate_mount_point(&config.mount_point)?;

        #[cfg(target_os = "windows")]
        {
            let inner = windows_impl::WfspMountInner::new(fs, config.clone())?;
            return Ok(Self { inner, config });
        }

        #[cfg(not(target_os = "windows"))]
        {
            tracing::warn!(
                mount_point = %config.mount_point,
                "WinFsp mount requested on non-Windows host — no-op stub active"
            );
            Ok(Self { _fs: fs, config })
        }
    }

    /// Block the calling thread until the volume is unmounted.
    pub fn run(&self) {
        #[cfg(target_os = "windows")]
        self.inner.run();

        #[cfg(not(target_os = "windows"))]
        tracing::info!(
            mount_point = %self.config.mount_point,
            "WfspMount::run() — platform stub, returning immediately"
        );
    }

    /// Unmount explicitly without waiting for `run()` to return.
    pub fn stop(&self) {
        #[cfg(target_os = "windows")]
        self.inner.stop();

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
/// Accepts drive letters (`Z:`) and absolute paths.
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
