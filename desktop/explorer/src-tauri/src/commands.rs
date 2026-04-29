//! Tauri command handlers — one per UI tab (T6.6).
//!
//! The TypeScript side calls these via `invoke("browse", { path: "/" })`.
//! Each command borrows the Tauri-managed `AppState`, drives the ATLAS
//! engine, and returns a serialisable IPC response type.

use crate::state::AppState;
use atlas_explorer_ipc::{
    BrowserEntry, BrowserRequest, BrowserResponse,
    LineageRequest, LineageResponse,
    PolicyRequest, PolicyResponse, PolicyView,
    SearchRequest, SearchResponse, SearchResult,
    VersionRequest, VersionResponse,
};
use std::path::PathBuf;

/// Open (or switch to) an ATLAS store on disk.
pub fn open_store(state: &AppState, path: PathBuf) -> Result<String, String> {
    state.open_store(path.clone())?;
    Ok(format!("Opened store: {}", path.display()))
}

/// List the children of `request.path` for the Browser tab.
pub fn browse(state: &AppState, request: BrowserRequest) -> BrowserResponse {
    state
        .with_fs(|fs| {
            let entries = fs.list(&request.path).map_err(|e| e.to_string())?;
            let browser_entries: Vec<BrowserEntry> = entries
                .iter()
                .map(BrowserEntry::from_fs_entry)
                .collect();
            Ok(BrowserResponse::ok(&request.path, browser_entries))
        })
        .unwrap_or_else(|e| BrowserResponse::err(&request.path, e))
}

/// Run a hybrid search query for the Search tab.
pub fn search(_state: &AppState, request: SearchRequest) -> SearchResponse {
    // Production: route through atlas_indexer::AtlasIndex::hybrid_search.
    // Stub returns an informative placeholder.
    SearchResponse::ok(
        &request.query,
        vec![SearchResult {
            path: "/example/result.safetensors".into(),
            snippet: format!("Matched «{}» — connect the indexer to get real results.", request.query),
            score: 1.0,
            kind: "file".into(),
        }],
        0,
    )
}

/// Query the lineage graph for the Lineage tab.
pub fn lineage(_state: &AppState, request: LineageRequest) -> LineageResponse {
    LineageResponse::ok(&request.path, vec![])
}

/// Return commit log and branch list for the Version tab.
pub fn version_log(_state: &AppState, _request: VersionRequest) -> VersionResponse {
    VersionResponse::ok(vec![], vec![], "main")
}

/// Return policy view for a path.
pub fn policy_view(_state: &AppState, request: PolicyRequest) -> PolicyResponse {
    PolicyResponse::ok(PolicyView {
        path: request.path,
        rules: vec![],
        redaction_enabled: false,
        capability_scope: None,
    })
}
