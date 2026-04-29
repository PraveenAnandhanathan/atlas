//! Version tab IPC types (T6.6).

use serde::{Deserialize, Serialize};

/// Summary view of a single commit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitView {
    pub hash_hex: String,
    pub short_hash: String,
    pub message: String,
    pub author: String,
    pub timestamp_ms: u64,
    pub parent_hashes: Vec<String>,
}

/// Summary view of a branch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchView {
    pub name: String,
    pub head_hash: String,
    pub is_current: bool,
}

/// Request version history for a path or the whole volume.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionRequest {
    /// Empty = volume-level log; otherwise path-specific history.
    pub path: String,
    /// Number of commits to return.
    pub limit: usize,
    /// Commit hash to start from (for pagination).
    pub after: Option<String>,
}

/// Version response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionResponse {
    pub commits: Vec<CommitView>,
    pub branches: Vec<BranchView>,
    pub current_branch: String,
    pub error: Option<String>,
}

impl VersionResponse {
    pub fn ok(commits: Vec<CommitView>, branches: Vec<BranchView>, current: impl Into<String>) -> Self {
        Self { commits, branches, current_branch: current.into(), error: None }
    }

    pub fn err(msg: impl Into<String>) -> Self {
        Self { commits: Vec::new(), branches: Vec::new(), current_branch: String::new(), error: Some(msg.into()) }
    }
}
