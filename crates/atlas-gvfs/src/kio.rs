//! KDE I/O (KIO) worker for the `atlas://` URL scheme (T6.5).
//!
//! The KIO worker is a shared library (`kio_atlas.so`) installed into
//! `$KDE_INSTALL_PLUGINDIR/kio/`.  Dolphin spawns `kioworker atlas`
//! as a subprocess; the subprocess links against this Rust code and
//! receives ipc packets from `kiod6`.
//!
//! The C++ `WorkerBase` subclass lives in `desktop/kio-worker/`; this
//! module provides the Rust symbols it calls.

use serde::{Deserialize, Serialize};

/// Minimal KIO `UDSEntry` fields ATLAS populates for every item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UdsEntry {
    /// UDS_NAME — filename without path.
    pub name: String,
    /// UDS_FILE_TYPE — `S_IFDIR` (16384) or `S_IFREG` (32768).
    pub file_type: u32,
    /// UDS_SIZE in bytes.
    pub size: u64,
    /// UDS_MIME_TYPE.
    pub mime_type: String,
    /// UDS_URL — full `atlas://` URI.
    pub url: String,
    /// UDS_ICON_NAME — themed icon.
    pub icon_name: String,
}

pub mod file_type {
    pub const DIR:     u32 = 16384; // S_IFDIR
    pub const REGULAR: u32 = 32768; // S_IFREG
}

impl UdsEntry {
    pub fn from_atlas_entry(entry: &atlas_fs::Entry, volume: &str, host: &str) -> Self {
        let name = entry.path.rsplit('/').next().unwrap_or(&entry.path).to_string();
        let is_dir = matches!(entry.kind, atlas_core::ObjectKind::Dir);
        let file_type = if is_dir { file_type::DIR } else { file_type::REGULAR };
        let mime = mime_for_path(&entry.path, is_dir);
        let icon = if is_dir { "folder" } else { "text-x-generic" };
        let uri = format!("atlas://{host}/{volume}{}", entry.path);
        Self {
            name,
            file_type,
            size: entry.size,
            mime_type: mime.into(),
            url: uri,
            icon_name: icon.into(),
        }
    }
}

fn mime_for_path(path: &str, is_dir: bool) -> &'static str {
    if is_dir { return "inode/directory"; }
    if path.ends_with(".safetensors") { return "application/x-safetensors"; }
    if path.ends_with(".parquet")     { return "application/x-parquet"; }
    if path.ends_with(".json")        { return "application/json"; }
    if path.ends_with(".jsonl")       { return "application/jsonlines"; }
    if path.ends_with(".arrow")       { return "application/x-arrow"; }
    "application/octet-stream"
}

/// KIO worker protocol constants.
pub mod protocol {
    pub const SCHEME: &str = "atlas";
    pub const DESKTOP_FILE: &str = "kio_atlas.desktop";
    pub const SERVICE_TYPE: &str = "KIO/SlaveBase";
}

/// KIO `.desktop` metadata for the worker.
pub fn worker_desktop_entry() -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Service\n\
         Name=ATLAS KIO Worker\n\
         Exec=kioworker {}\n\
         X-KDE-ServiceTypes={}\n\
         X-KDE-Protocol={}\n\
         X-KDE-Protocols={}\n",
        protocol::SCHEME,
        protocol::SERVICE_TYPE,
        protocol::SCHEME,
        protocol::SCHEME,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uds_entry_directory() {
        use atlas_core::{Hash, ObjectKind};
        let entry = atlas_fs::Entry {
            path: "/datasets".into(),
            kind: ObjectKind::Dir,
            hash: Hash::ZERO,
            size: 0,
        };
        let uds = UdsEntry::from_atlas_entry(&entry, "myvol", "localhost");
        assert_eq!(uds.file_type, file_type::DIR);
        assert_eq!(uds.name, "datasets");
        assert!(uds.url.contains("atlas://"));
    }

    #[test]
    fn uds_entry_file_mime() {
        use atlas_core::{Hash, ObjectKind};
        let entry = atlas_fs::Entry {
            path: "/data/model.safetensors".into(),
            kind: ObjectKind::File,
            hash: Hash::ZERO,
            size: 1024,
        };
        let uds = UdsEntry::from_atlas_entry(&entry, "vol", "host");
        assert_eq!(uds.mime_type, "application/x-safetensors");
    }

    #[test]
    fn worker_desktop_entry_has_scheme() {
        let de = worker_desktop_entry();
        assert!(de.contains("atlas"));
    }
}
