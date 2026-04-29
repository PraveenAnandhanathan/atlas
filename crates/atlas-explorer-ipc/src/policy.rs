//! Policy tab IPC types (T6.6).

use serde::{Deserialize, Serialize};

/// A human-readable view of a policy attached to a path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyView {
    pub path: String,
    pub rules: Vec<PolicyRule>,
    pub redaction_enabled: bool,
    pub capability_scope: Option<String>,
}

/// One policy rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    pub permission: String,
    pub principal: String,
    pub effect: PolicyEffect,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PolicyEffect {
    Allow,
    Deny,
}

/// Request policy info for a path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRequest {
    pub path: String,
}

/// Policy response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyResponse {
    pub view: Option<PolicyView>,
    pub error: Option<String>,
}

impl PolicyResponse {
    pub fn ok(view: PolicyView) -> Self {
        Self { view: Some(view), error: None }
    }
    pub fn err(msg: impl Into<String>) -> Self {
        Self { view: None, error: Some(msg.into()) }
    }
}
