//! Newline-delimited JSON-RPC envelope used by `atlasctl mcp serve` (T5.2).
//!
//! The official MCP spec layers a few framing options on top — stdio,
//! Unix-socket, WebSocket — but every transport sees the same JSON
//! request/response pair, so we keep them separate from the framing.
//! The conformance harness (T5.8) feeds requests through `handle_line`
//! directly without touching any socket.
//!
//! Supported methods:
//!
//! - `initialize` — handshake; returns server name + protocol version.
//! - `tools/list` — descriptors from [`crate::tools`].
//! - `tools/call` — `params: { name, arguments, principal? }`.
//! - `ping` — liveness.

use crate::{tool_descriptors, ApiError, CapabilityCore};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpRequest {
    /// JSON-RPC 2.0 marker (we accept missing).
    #[serde(default)]
    pub jsonrpc: Option<String>,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpResponse {
    pub jsonrpc: &'static str,
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
}

impl McpResponse {
    pub fn ok(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }
    pub fn err(id: Option<Value>, e: ApiError) -> Self {
        let code = match e.code {
            crate::core::ErrorCode::NotFound => -32004,
            crate::core::ErrorCode::Forbidden => -32003,
            crate::core::ErrorCode::InvalidArgument => -32602,
            crate::core::ErrorCode::NotImplemented => -32601,
            crate::core::ErrorCode::Internal => -32603,
        };
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(RpcError { code, message: e.message }),
        }
    }
}

/// Process one JSON-RPC line and return its serialised response.
pub fn handle_line(core: &CapabilityCore, line: &str) -> String {
    let req: McpRequest = match serde_json::from_str(line) {
        Ok(r) => r,
        Err(e) => {
            let resp = McpResponse {
                jsonrpc: "2.0",
                id: None,
                result: None,
                error: Some(RpcError { code: -32700, message: format!("parse: {e}") }),
            };
            return serde_json::to_string(&resp).unwrap_or_default();
        }
    };
    let resp = handle(core, req);
    serde_json::to_string(&resp).unwrap_or_default()
}

fn handle(core: &CapabilityCore, req: McpRequest) -> McpResponse {
    match req.method.as_str() {
        "initialize" => McpResponse::ok(
            req.id,
            json!({
                "protocolVersion": "2024-11-05",
                "serverInfo": {"name": "atlas-mcp", "version": env!("CARGO_PKG_VERSION")},
                "capabilities": {"tools": {"listChanged": false}}
            }),
        ),
        "ping" => McpResponse::ok(req.id, json!({"pong": true})),
        "tools/list" => {
            let tools: Vec<Value> = tool_descriptors()
                .into_iter()
                .map(|d| {
                    json!({
                        "name": d.name,
                        "description": d.description,
                        "inputSchema": d.input_schema,
                    })
                })
                .collect();
            McpResponse::ok(req.id, json!({"tools": tools}))
        }
        "tools/call" => {
            let name = match req.params.get("name").and_then(|v| v.as_str()) {
                Some(s) => s.to_string(),
                None => {
                    return McpResponse::err(
                        req.id,
                        ApiError::invalid("missing tool name"),
                    )
                }
            };
            let args = req.params.get("arguments").cloned().unwrap_or(json!({}));
            let principal = req
                .params
                .get("principal")
                .and_then(|v| v.as_str())
                .unwrap_or("anonymous");
            match core.invoke(principal, &name, &args) {
                Ok(v) => McpResponse::ok(
                    req.id,
                    json!({
                        "content": [{"type": "text", "text": v.to_string()}],
                        "isError": false,
                        "structuredContent": v,
                    }),
                ),
                Err(e) => McpResponse::err(req.id, e),
            }
        }
        other => McpResponse::err(
            req.id,
            ApiError::not_implemented(format!("method {other} unknown")),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_fs::Fs;
    use std::sync::Arc;

    fn core() -> CapabilityCore {
        let dir = tempfile::tempdir().unwrap();
        let fs = Fs::init(dir.path()).unwrap();
        // Leak the temp dir so the test scope owns the store.
        std::mem::forget(dir);
        CapabilityCore::new(Arc::new(fs))
    }

    #[test]
    fn initialize_returns_server_info() {
        let c = core();
        let resp =
            handle_line(&c, r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#);
        assert!(resp.contains("atlas-mcp"));
    }

    #[test]
    fn tools_list_includes_full_catalog() {
        let c = core();
        let resp =
            handle_line(&c, r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#);
        assert!(resp.contains("atlas.fs.read"));
        assert!(resp.contains("atlas.semantic.query"));
        assert!(resp.contains("atlas.lineage.upstream"));
    }

    #[test]
    fn tools_call_dispatches_to_core() {
        let c = core();
        let line = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"atlas.fs.write","arguments":{"path":"/x","content":"hi"}}}"#;
        let resp = handle_line(&c, line);
        assert!(resp.contains("\"hash\""));
    }

    #[test]
    fn parse_error_returns_rpc_error() {
        let c = core();
        let resp = handle_line(&c, "not json");
        assert!(resp.contains("-32700"));
    }
}
