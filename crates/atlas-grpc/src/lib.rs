//! ATLAS gRPC adapter — service definitions + reflection (T5.5).
//!
//! This crate is deliberately *protobuf-free*: it ships the equivalent
//! of a `.proto` file as a Rust descriptor table and provides a
//! reflection endpoint compatible with the gRPC Server Reflection
//! Protocol (`ServerReflectionInfo`). When the workspace acquires a
//! tonic build step the descriptors here become the source of truth
//! for the generated `atlas.v1.Atlas` service stubs.
//!
//! # Service shape
//!
//! ```proto
//! service Atlas {
//!   rpc Invoke(InvokeRequest) returns (InvokeResponse);
//!   rpc ListTools(google.protobuf.Empty) returns (ToolList);
//! }
//! message InvokeRequest { string capability = 1; string principal = 2;
//!                         string arguments_json = 3; }
//! message InvokeResponse { bool ok = 1; string result_json = 2;
//!                          string error = 3; int32 code = 4; }
//! ```
//!
//! Until the codegen lands, [`invoke`] is the in-process trampoline and
//! [`reflection_descriptor`] returns the proto schema bytes-equivalent
//! as JSON for the conformance harness (T5.8).

use atlas_mcp::{tool_descriptors, ApiError, CapabilityCore};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Mirrors the `InvokeRequest` proto message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvokeRequest {
    pub capability: String,
    #[serde(default)]
    pub principal: String,
    /// JSON-encoded argument struct (gRPC clients carry it as bytes/string
    /// because protobuf's `Any` is awkward to round-trip from agents).
    pub arguments_json: String,
}

/// Mirrors the `InvokeResponse` proto message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvokeResponse {
    pub ok: bool,
    pub result_json: String,
    pub error: String,
    pub code: i32,
}

pub fn invoke(core: &CapabilityCore, req: &InvokeRequest) -> InvokeResponse {
    let principal = if req.principal.is_empty() {
        "anonymous"
    } else {
        &req.principal
    };
    let args: Value =
        serde_json::from_str(if req.arguments_json.is_empty() { "{}" } else { &req.arguments_json })
            .unwrap_or(json!({}));
    match core.invoke(principal, &req.capability, &args) {
        Ok(v) => InvokeResponse {
            ok: true,
            result_json: v.to_string(),
            error: String::new(),
            code: 0,
        },
        Err(e) => {
            let code = grpc_status(&e);
            InvokeResponse {
                ok: false,
                result_json: String::new(),
                error: e.message,
                code,
            }
        }
    }
}

fn grpc_status(e: &ApiError) -> i32 {
    // Mapped to canonical gRPC status codes.
    match e.code {
        atlas_mcp::core::ErrorCode::NotFound => 5,
        atlas_mcp::core::ErrorCode::Forbidden => 7,
        atlas_mcp::core::ErrorCode::InvalidArgument => 3,
        atlas_mcp::core::ErrorCode::NotImplemented => 12,
        atlas_mcp::core::ErrorCode::Internal => 13,
    }
}

/// Reflection metadata returned to a `ServerReflectionInfo` caller.
pub fn reflection_descriptor() -> Value {
    let methods: Vec<Value> = tool_descriptors()
        .into_iter()
        .map(|d| {
            json!({
                "name": d.name,
                "description": d.description,
                "input_schema": d.input_schema,
                "mutates": d.mutates,
            })
        })
        .collect();
    json!({
        "package": "atlas.v1",
        "service": "Atlas",
        "rpcs": [
            {"name": "Invoke", "input": "InvokeRequest", "output": "InvokeResponse"},
            {"name": "ListTools", "input": "Empty", "output": "ToolList"}
        ],
        "tools": methods,
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
    fn invoke_round_trip() {
        let c = core();
        let r = invoke(
            &c,
            &InvokeRequest {
                capability: "atlas.fs.write".into(),
                principal: "u".into(),
                arguments_json: r#"{"path":"/a","content":"hi"}"#.into(),
            },
        );
        assert!(r.ok);
        let parsed: Value = serde_json::from_str(&r.result_json).unwrap();
        assert!(parsed.get("hash").is_some());
    }

    #[test]
    fn forbidden_maps_to_grpc_7() {
        let c = core().with_subtree("/sub");
        let r = invoke(
            &c,
            &InvokeRequest {
                capability: "atlas.fs.write".into(),
                principal: "u".into(),
                arguments_json: r#"{"path":"/other","content":"x"}"#.into(),
            },
        );
        assert!(!r.ok);
        assert_eq!(r.code, 7);
    }

    #[test]
    fn reflection_lists_full_catalog() {
        let d = reflection_descriptor();
        assert_eq!(d["service"], "Atlas");
        assert!(d["tools"].as_array().unwrap().len() >= 30);
    }
}
