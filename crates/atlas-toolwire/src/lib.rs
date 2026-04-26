//! Anthropic tool-use and OpenAI function-calling adapters (T5.7).
//!
//! Both vendor wire formats describe a tool by `(name, description,
//! JSON-Schema)` and invoke it with `(name, arguments)` JSON.  This
//! crate re-projects the MCP descriptors into both shapes and ships
//! a [`run_tool_use`] function that swallows a model's tool-use block,
//! dispatches it through [`atlas_mcp::CapabilityCore`], and emits the
//! matching tool-result block.
//!
//! Anthropic and OpenAI both reject `.` in tool names, so the canonical
//! `atlas.fs.read` becomes `atlas_fs_read` on the wire.  The MCP
//! parser accepts either form (see [`atlas_mcp::parse_capability`]).

use atlas_mcp::{tool_descriptors, CapabilityCore};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

fn wire_name(canonical: &str) -> String {
    canonical.replace('.', "_")
}

/// Anthropic Messages API `tools` array entries.
pub fn anthropic_tools() -> Vec<Value> {
    tool_descriptors()
        .into_iter()
        .map(|d| {
            json!({
                "name": wire_name(d.name),
                "description": d.description,
                "input_schema": d.input_schema,
            })
        })
        .collect()
}

/// OpenAI Chat Completions `tools` array entries (`type=function`).
pub fn openai_tools() -> Vec<Value> {
    tool_descriptors()
        .into_iter()
        .map(|d| {
            json!({
                "type": "function",
                "function": {
                    "name": wire_name(d.name),
                    "description": d.description,
                    "parameters": d.input_schema,
                }
            })
        })
        .collect()
}

/// Vendor-neutral tool-use block consumed by [`run_tool_use`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolUse {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub input: Value,
}

/// Run a model-emitted tool-use block.  Returns Anthropic-style
/// `tool_result` content as a JSON value.
pub fn run_tool_use(core: &CapabilityCore, principal: &str, t: &ToolUse) -> Value {
    match core.invoke(principal, &t.name, &t.input) {
        Ok(v) => json!({
            "type": "tool_result",
            "tool_use_id": t.id,
            "content": [{"type": "text", "text": v.to_string()}],
            "is_error": false,
        }),
        Err(e) => json!({
            "type": "tool_result",
            "tool_use_id": t.id,
            "content": [{"type": "text", "text": e.message}],
            "is_error": true,
        }),
    }
}

/// OpenAI tool-call structure: `{name, arguments: stringified-json}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiToolCall {
    pub id: String,
    pub function: OpenAiFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiFunction {
    pub name: String,
    pub arguments: String,
}

/// Run an OpenAI tool-call.  Returns the `tool` role message body.
pub fn run_openai_tool_call(
    core: &CapabilityCore,
    principal: &str,
    call: &OpenAiToolCall,
) -> Value {
    let args: Value = serde_json::from_str(&call.function.arguments).unwrap_or(json!({}));
    let result = core.invoke(principal, &call.function.name, &args);
    let (content, is_error) = match result {
        Ok(v) => (v.to_string(), false),
        Err(e) => (json!({"error": e.message}).to_string(), true),
    };
    json!({
        "role": "tool",
        "tool_call_id": call.id,
        "content": content,
        "is_error": is_error,
    })
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
    fn anthropic_tool_names_have_no_dots() {
        for t in anthropic_tools() {
            let name = t["name"].as_str().unwrap();
            assert!(!name.contains('.'), "{name}");
        }
    }

    #[test]
    fn openai_tools_have_function_wrapper() {
        for t in openai_tools() {
            assert_eq!(t["type"], "function");
            assert!(t["function"]["name"].is_string());
        }
    }

    #[test]
    fn run_tool_use_round_trip() {
        let c = core();
        let r = run_tool_use(
            &c,
            "u",
            &ToolUse {
                id: "tu-1".into(),
                name: "atlas_fs_write".into(),
                input: json!({"path": "/p", "content": "x"}),
            },
        );
        assert_eq!(r["is_error"], false);
        assert_eq!(r["tool_use_id"], "tu-1");
    }

    #[test]
    fn run_openai_tool_call_round_trip() {
        let c = core();
        let r = run_openai_tool_call(
            &c,
            "u",
            &OpenAiToolCall {
                id: "call-1".into(),
                function: OpenAiFunction {
                    name: "atlas_fs_write".into(),
                    arguments: r#"{"path":"/q","content":"y"}"#.into(),
                },
            },
        );
        assert_eq!(r["is_error"], false);
    }

    #[test]
    fn openai_unknown_capability_is_error() {
        let c = core();
        let r = run_openai_tool_call(
            &c,
            "u",
            &OpenAiToolCall {
                id: "x".into(),
                function: OpenAiFunction {
                    name: "atlas_nope".into(),
                    arguments: "{}".into(),
                },
            },
        );
        assert_eq!(r["is_error"], true);
    }
}
