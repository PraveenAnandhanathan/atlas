//! Lineage tab IPC types (T6.6).

use serde::{Deserialize, Serialize};

/// A directed edge in the lineage graph shown in the GUI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineageEdgeView {
    pub from_path: String,
    pub to_path: String,
    pub kind: String,
    pub timestamp_ms: u64,
    pub actor: String,
}

/// Request lineage graph for a path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineageRequest {
    pub path: String,
    pub depth: usize,
}

/// Lineage response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineageResponse {
    pub path: String,
    pub edges: Vec<LineageEdgeView>,
    pub error: Option<String>,
}

impl LineageResponse {
    pub fn ok(path: impl Into<String>, edges: Vec<LineageEdgeView>) -> Self {
        Self { path: path.into(), edges, error: None }
    }
    pub fn err(path: impl Into<String>, msg: impl Into<String>) -> Self {
        Self { path: path.into(), edges: Vec::new(), error: Some(msg.into()) }
    }
}
