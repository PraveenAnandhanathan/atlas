//! ATLAS Agent-to-Agent reference adapter (T5.3).
//!
//! Implements the bare A2A surface every spec variant agrees on:
//!
//! - an **agent card** (`/.well-known/agent.json` style) describing the
//!   ATLAS instance as an A2A peer with a list of skills,
//! - a `tasks/send` invocation that translates an A2A message into a
//!   [`atlas_mcp::CapabilityCore::invoke`] call.
//!
//! Per design §8.1 this stays a thin translator: every request funnels
//! through the same capability core, governance is unchanged.

use atlas_mcp::{tool_descriptors, CapabilityCore};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// One advertised skill in the agent card.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub input_modes: Vec<String>,
    pub output_modes: Vec<String>,
}

/// Public agent card; serialised at `/.well-known/agent.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCard {
    pub name: String,
    pub description: String,
    pub url: String,
    pub version: String,
    pub skills: Vec<Skill>,
    pub capabilities: AgentCapabilities,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCapabilities {
    pub streaming: bool,
    pub push_notifications: bool,
}

/// Build the standard ATLAS agent card.  Each MCP capability becomes one
/// A2A skill with stable `atlas.<ns>.<verb>` ID.
pub fn agent_card(url: impl Into<String>) -> AgentCard {
    let skills = tool_descriptors()
        .into_iter()
        .map(|d| Skill {
            id: d.name.to_string(),
            name: d.name.to_string(),
            description: d.description.to_string(),
            input_modes: vec!["application/json".into()],
            output_modes: vec!["application/json".into()],
        })
        .collect();
    AgentCard {
        name: "atlas".into(),
        description: "ATLAS filesystem exposed as an A2A peer.".into(),
        url: url.into(),
        version: env!("CARGO_PKG_VERSION").into(),
        skills,
        capabilities: AgentCapabilities {
            streaming: false,
            push_notifications: false,
        },
    }
}

/// Incoming A2A `tasks/send` message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2aRequest {
    /// Skill id (e.g. `atlas.fs.read`).
    pub skill: String,
    #[serde(default)]
    pub input: Value,
    #[serde(default)]
    pub principal: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2aResponse {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub fn handle(core: &CapabilityCore, req: &A2aRequest) -> A2aResponse {
    let principal = req.principal.as_deref().unwrap_or("a2a-peer");
    match core.invoke(principal, &req.skill, &req.input) {
        Ok(v) => A2aResponse {
            status: "completed".into(),
            output: Some(v),
            error: None,
        },
        Err(e) => A2aResponse {
            status: "failed".into(),
            output: None,
            error: Some(e.message),
        },
    }
}

/// Convenience: render the well-known JSON.
pub fn agent_card_json(url: &str) -> Value {
    serde_json::to_value(agent_card(url)).unwrap_or(json!({}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_fs::Fs;
    use std::sync::Arc;

    fn core() -> CapabilityCore {
        let d = tempfile::tempdir().unwrap();
        let fs = Fs::init(d.path()).unwrap();
        std::mem::forget(d);
        CapabilityCore::new(Arc::new(fs))
    }

    #[test]
    fn card_contains_every_skill() {
        let card = agent_card("https://atlas.local/a2a");
        let names: Vec<&str> = card.skills.iter().map(|s| s.id.as_str()).collect();
        assert!(names.contains(&"atlas.fs.read"));
        assert!(names.contains(&"atlas.semantic.query"));
    }

    #[test]
    fn handle_dispatches_to_core() {
        let c = core();
        let req = A2aRequest {
            skill: "atlas.fs.write".into(),
            input: json!({"path": "/p", "content": "hi"}),
            principal: Some("peer-1".into()),
        };
        let r = handle(&c, &req);
        assert_eq!(r.status, "completed");
    }

    #[test]
    fn unknown_skill_fails() {
        let c = core();
        let r = handle(
            &c,
            &A2aRequest { skill: "atlas.nope".into(), input: json!({}), principal: None },
        );
        assert_eq!(r.status, "failed");
    }
}
