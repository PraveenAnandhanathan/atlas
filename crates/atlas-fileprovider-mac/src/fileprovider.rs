//! FileProvider core logic (T6.3).
//!
//! Implements the *replicated extension* model introduced in macOS 12:
//! the system owns the local copy; the extension answers fetch/upload
//! requests asynchronously.  Every domain maps to one ATLAS volume.

use atlas_core::Hash;
use atlas_fs::{Entry, Fs};
use serde::{Deserialize, Serialize};

/// Stable opaque identifier for a file-provider item.
/// ATLAS maps this 1:1 to the ATLAS path within the volume.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ItemIdentifier(pub String);

impl ItemIdentifier {
    pub fn root() -> Self { Self("/".into()) }
    pub fn from_path(path: &str) -> Self { Self(path.to_string()) }
    pub fn as_path(&self) -> &str { &self.0 }
}

/// Metadata about a single file-provider item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemMetadata {
    pub identifier: ItemIdentifier,
    pub parent: ItemIdentifier,
    pub filename: String,
    pub is_directory: bool,
    pub size: u64,
    pub content_hash: Hash,
    /// UTI string (e.g. `"org.atlas.safetensors"`).
    pub type_identifier: String,
}

impl ItemMetadata {
    pub fn from_entry(entry: &Entry, parent: ItemIdentifier) -> Self {
        let filename = entry.path.rsplit('/').next().unwrap_or(&entry.path).to_string();
        let uti = uti_for_path(&entry.path);
        Self {
            identifier: ItemIdentifier::from_path(&entry.path),
            parent,
            filename,
            is_directory: matches!(entry.kind, atlas_core::ObjectKind::Dir),
            size: entry.size,
            content_hash: entry.hash,
            type_identifier: uti.to_string(),
        }
    }
}

fn uti_for_path(path: &str) -> &'static str {
    if path.ends_with(".safetensors") { return "org.atlas.safetensors"; }
    if path.ends_with(".parquet")     { return "org.atlas.parquet"; }
    if path.ends_with(".arrow")       { return "org.atlas.arrow"; }
    if path.ends_with(".zarr")        { return "org.atlas.zarr"; }
    if path.ends_with(".json")        { return "public.json"; }
    if path.ends_with(".jsonl")       { return "public.json"; }
    "public.data"
}

/// Core logic wired into the Swift `NSFileProviderReplicatedExtension`.
pub struct FileProviderCore {
    pub fs: Fs,
}

impl FileProviderCore {
    pub fn new(fs: Fs) -> Self {
        Self { fs }
    }

    /// Enumerate children of `parent` — called by `enumerator(for:)`.
    pub fn enumerate(&self, parent: &ItemIdentifier) -> atlas_core::Result<Vec<ItemMetadata>> {
        let entries = self.fs.list(parent.as_path())?;
        Ok(entries
            .iter()
            .map(|e| ItemMetadata::from_entry(e, parent.clone()))
            .collect())
    }

    /// Fetch the content of an item — called by `fetchContents(for:version:)`.
    pub fn fetch_content(&self, id: &ItemIdentifier) -> atlas_core::Result<Vec<u8>> {
        self.fs.read(id.as_path())
    }

    /// Upload new content — called by `createItem(basedOn:fields:contents:)`.
    pub fn upload_content(&self, id: &ItemIdentifier, data: &[u8]) -> atlas_core::Result<()> {
        self.fs.write(id.as_path(), data)?;
        Ok(())
    }

    /// Delete an item — called by `deleteItem(identifier:baseVersion:)`.
    pub fn delete_item(&self, id: &ItemIdentifier) -> atlas_core::Result<()> {
        self.fs.delete(id.as_path())
    }

    /// Move/rename — called by `modifyItem(identifier:baseVersion:fields:)`.
    pub fn move_item(&self, from: &ItemIdentifier, to: &ItemIdentifier) -> atlas_core::Result<()> {
        self.fs.rename(from.as_path(), to.as_path())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_identifier_root() {
        let root = ItemIdentifier::root();
        assert_eq!(root.as_path(), "/");
    }

    #[test]
    fn uti_detection() {
        assert_eq!(uti_for_path("/data/model.safetensors"), "org.atlas.safetensors");
        assert_eq!(uti_for_path("/data/table.parquet"), "org.atlas.parquet");
        assert_eq!(uti_for_path("/data/blob.bin"), "public.data");
    }

    #[test]
    fn item_metadata_from_entry() {
        use atlas_core::{Hash, ObjectKind};
        let entry = atlas_fs::Entry {
            path: "/models/gpt2.safetensors".into(),
            kind: ObjectKind::File,
            hash: Hash::ZERO,
            size: 512,
        };
        let meta = ItemMetadata::from_entry(&entry, ItemIdentifier::root());
        assert_eq!(meta.filename, "gpt2.safetensors");
        assert_eq!(meta.type_identifier, "org.atlas.safetensors");
        assert!(!meta.is_directory);
    }
}
