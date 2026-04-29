//! ATLAS Explorer column provider (T6.2).
//!
//! Maps onto the Windows `IColumnProvider` (legacy) / `IPropertyHandler`
//! (Vista+) COM interfaces.  The pure-Rust layer here computes the
//! column values from an [`atlas_fs::Entry`]; the COM glue that actually
//! registers them in Explorer lives in a thin C++ shim compiled only on
//! Windows.

use atlas_fs::{Entry, Fs};
use serde::{Deserialize, Serialize};

/// One ATLAS-specific column shown in Explorer Details view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AtlasColumn {
    /// BLAKE3 hash of the file content (hex, 64 chars).
    ContentHash,
    /// Current branch the mounted volume is on.
    Branch,
    /// Number of commits in the lineage chain that touch this file.
    LineageDepth,
    /// Comma-separated policy tags attached via xattr.
    PolicyTags,
    /// Detected content-type (e.g. `safetensors`, `parquet`, `json`).
    ContentType,
    /// Short commit ref (`HEAD~3`, `abc12345`).
    CommitRef,
}

impl AtlasColumn {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::ContentHash   => "ATLAS Hash",
            Self::Branch        => "ATLAS Branch",
            Self::LineageDepth  => "Lineage Depth",
            Self::PolicyTags    => "Policy Tags",
            Self::ContentType   => "Content Type",
            Self::CommitRef     => "Commit",
        }
    }

    pub fn width_chars(&self) -> u32 {
        match self {
            Self::ContentHash  => 20,
            Self::Branch       => 12,
            Self::LineageDepth => 8,
            Self::PolicyTags   => 20,
            Self::ContentType  => 14,
            Self::CommitRef    => 10,
        }
    }

    /// All columns in display order.
    pub fn all() -> &'static [AtlasColumn] {
        &[
            Self::ContentHash,
            Self::Branch,
            Self::LineageDepth,
            Self::PolicyTags,
            Self::ContentType,
            Self::CommitRef,
        ]
    }
}

/// A resolved column value for a particular file path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnValue {
    pub column: AtlasColumn,
    pub value: String,
}

/// Computes column values for a single filesystem entry.
pub struct ColumnProvider<'a> {
    fs: &'a Fs,
}

impl<'a> ColumnProvider<'a> {
    pub fn new(fs: &'a Fs) -> Self {
        Self { fs }
    }

    /// Return all column values for `path`.  Returns an empty vec if
    /// the path is not inside an ATLAS mount.
    pub fn values_for(&self, path: &str) -> Vec<ColumnValue> {
        let entry = match self.fs.stat(path) {
            Ok(e) => e,
            Err(_) => return Vec::new(),
        };
        let hash_hex = hex::encode(entry.hash.as_bytes());
        let short_hash = &hash_hex[..12];

        vec![
            ColumnValue { column: AtlasColumn::ContentHash, value: short_hash.to_string() },
            ColumnValue { column: AtlasColumn::Branch, value: "main".to_string() },
            ColumnValue { column: AtlasColumn::LineageDepth, value: "0".to_string() },
            ColumnValue { column: AtlasColumn::PolicyTags, value: String::new() },
            ColumnValue {
                column: AtlasColumn::ContentType,
                value: detect_content_type(&entry).to_string(),
            },
            ColumnValue { column: AtlasColumn::CommitRef, value: "HEAD".to_string() },
        ]
    }
}

fn detect_content_type(entry: &Entry) -> &'static str {
    let p = entry.path.as_str();
    if p.ends_with(".safetensors") { return "safetensors"; }
    if p.ends_with(".parquet")     { return "parquet"; }
    if p.ends_with(".json") || p.ends_with(".jsonl") { return "json"; }
    if p.ends_with(".arrow")       { return "arrow"; }
    if p.ends_with(".zarr")        { return "zarr"; }
    if p.ends_with(".pdf")         { return "pdf"; }
    "binary"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_display_names_unique() {
        let names: Vec<_> = AtlasColumn::all().iter().map(|c| c.display_name()).collect();
        let unique: std::collections::HashSet<_> = names.iter().copied().collect();
        assert_eq!(names.len(), unique.len());
    }

    #[test]
    fn all_columns_have_positive_width() {
        for col in AtlasColumn::all() {
            assert!(col.width_chars() > 0, "{:?} has zero width", col);
        }
    }
}
