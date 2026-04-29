//! Search tab IPC types (T6.6).

use serde::{Deserialize, Serialize};

/// Hybrid search request (vector + keyword + attribute filter).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchRequest {
    pub query: String,
    /// Constrain results to this subtree (empty = entire volume).
    pub path_prefix: String,
    /// Maximum results to return.
    pub limit: usize,
    /// Enable vector search (requires embedder).
    pub vector: bool,
    /// Enable full-text BM25 search.
    pub keyword: bool,
}

impl Default for SearchRequest {
    fn default() -> Self {
        Self {
            query: String::new(),
            path_prefix: String::new(),
            limit: 50,
            vector: true,
            keyword: true,
        }
    }
}

/// A single search hit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub path: String,
    pub snippet: String,
    pub score: f32,
    pub kind: String,
}

/// Search response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    pub query: String,
    pub results: Vec<SearchResult>,
    pub total: usize,
    pub took_ms: u64,
    pub error: Option<String>,
}

impl SearchResponse {
    pub fn ok(query: impl Into<String>, results: Vec<SearchResult>, took_ms: u64) -> Self {
        let total = results.len();
        Self { query: query.into(), results, total, took_ms, error: None }
    }

    pub fn err(query: impl Into<String>, msg: impl Into<String>) -> Self {
        Self { query: query.into(), results: Vec::new(), total: 0, took_ms: 0, error: Some(msg.into()) }
    }
}
