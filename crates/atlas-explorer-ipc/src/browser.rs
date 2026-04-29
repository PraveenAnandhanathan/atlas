//! Browser tab IPC types (T6.6).

use atlas_core::{Hash, ObjectKind};
use serde::{Deserialize, Serialize};

/// A single row in the file browser pane.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserEntry {
    pub path: String,
    pub name: String,
    pub kind: ObjectKind,
    pub size: u64,
    pub hash_hex: String,
    pub modified_ms: u64,
    pub content_type: String,
}

impl BrowserEntry {
    pub fn from_fs_entry(e: &atlas_fs::Entry) -> Self {
        let name = e.path.rsplit('/').next().unwrap_or(&e.path).to_string();
        let hash_hex = hex::encode(e.hash.as_bytes());
        let content_type = infer_content_type(&e.path);
        Self {
            path: e.path.clone(),
            name,
            kind: e.kind,
            size: e.size,
            hash_hex,
            modified_ms: 0,
            content_type: content_type.into(),
        }
    }
}

fn infer_content_type(path: &str) -> &'static str {
    if path.ends_with(".safetensors") { return "safetensors"; }
    if path.ends_with(".parquet")     { return "parquet"; }
    if path.ends_with(".arrow")       { return "arrow"; }
    if path.ends_with(".zarr")        { return "zarr"; }
    if path.ends_with(".json")        { return "json"; }
    if path.ends_with(".jsonl")       { return "jsonl"; }
    if path.ends_with(".py")          { return "python"; }
    if path.ends_with(".rs")          { return "rust"; }
    "binary"
}

/// Request: list the children of a directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserRequest {
    pub path: String,
}

/// Response: list of entries + optional breadcrumb trail.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserResponse {
    pub path: String,
    pub entries: Vec<BrowserEntry>,
    pub breadcrumbs: Vec<String>,
    pub error: Option<String>,
}

impl BrowserResponse {
    pub fn ok(path: impl Into<String>, entries: Vec<BrowserEntry>) -> Self {
        let path = path.into();
        let breadcrumbs = path.split('/').filter(|s| !s.is_empty()).map(String::from).collect();
        Self { path, entries, breadcrumbs, error: None }
    }

    pub fn err(path: impl Into<String>, msg: impl Into<String>) -> Self {
        Self { path: path.into(), entries: Vec::new(), breadcrumbs: Vec::new(), error: Some(msg.into()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_response_breadcrumbs() {
        let resp = BrowserResponse::ok("/a/b/c", vec![]);
        assert_eq!(resp.breadcrumbs, vec!["a", "b", "c"]);
    }

    #[test]
    fn content_type_detection() {
        assert_eq!(infer_content_type("/x.safetensors"), "safetensors");
        assert_eq!(infer_content_type("/x.unknown"), "binary");
    }
}
