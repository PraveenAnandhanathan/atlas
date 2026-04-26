//! ATLAS REST adapter (T5.4).
//!
//! Per design §8 every adapter is a *thin translator*. This crate
//! ships the route table, the request/response handler, and an OpenAPI
//! 3.1 spec generator built on top of [`atlas_mcp::tool_descriptors`].
//!
//! Wire framing (axum/hyper/etc.) is intentionally left to whoever
//! embeds the adapter — the conformance harness (T5.8) drives
//! [`handle_request`] directly without needing a TCP listener.

use atlas_mcp::{tool_descriptors, ApiError, CapabilityCore};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestRequest {
    pub method: String,
    pub path: String,
    pub principal: Option<String>,
    pub body: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestResponse {
    pub status: u16,
    pub body: Value,
}

impl RestResponse {
    pub fn ok(body: Value) -> Self {
        Self { status: 200, body }
    }
    pub fn err(e: ApiError) -> Self {
        let status = match e.code {
            atlas_mcp::core::ErrorCode::NotFound => 404,
            atlas_mcp::core::ErrorCode::Forbidden => 403,
            atlas_mcp::core::ErrorCode::InvalidArgument => 400,
            atlas_mcp::core::ErrorCode::NotImplemented => 501,
            atlas_mcp::core::ErrorCode::Internal => 500,
        };
        Self {
            status,
            body: json!({"error": e.message, "code": format!("{:?}", e.code)}),
        }
    }
}

/// Translate `(method, path)` to a capability and dispatch via [`CapabilityCore`].
///
/// Path conventions:
/// - `POST /v1/tools/{capability}` — body is the tool argument JSON.
/// - `GET  /v1/tools` — list descriptors.
/// - `GET  /v1/openapi.json` — OpenAPI spec.
pub fn handle_request(core: &CapabilityCore, req: &RestRequest) -> RestResponse {
    let principal = req.principal.as_deref().unwrap_or("anonymous");
    let path = req.path.trim_end_matches('/');

    if req.method.eq_ignore_ascii_case("GET") && path == "/v1/openapi.json" {
        return RestResponse::ok(openapi_spec());
    }
    if req.method.eq_ignore_ascii_case("GET") && path == "/v1/tools" {
        let tools: Vec<Value> = tool_descriptors()
            .into_iter()
            .map(|d| json!({"name": d.name, "description": d.description}))
            .collect();
        return RestResponse::ok(json!({"tools": tools}));
    }
    if req.method.eq_ignore_ascii_case("POST") {
        if let Some(name) = path.strip_prefix("/v1/tools/") {
            return match core.invoke(principal, name, &req.body) {
                Ok(v) => RestResponse::ok(v),
                Err(e) => RestResponse::err(e),
            };
        }
    }
    RestResponse {
        status: 404,
        body: json!({"error": format!("no route for {} {}", req.method, req.path)}),
    }
}

/// Generate an OpenAPI 3.1 spec covering every capability.
pub fn openapi_spec() -> Value {
    let mut paths = serde_json::Map::new();
    for d in tool_descriptors() {
        let route = format!("/v1/tools/{}", d.name);
        let op = json!({
            "post": {
                "operationId": d.name,
                "summary": d.description,
                "x-atlas-mutates": d.mutates,
                "requestBody": {
                    "required": true,
                    "content": {
                        "application/json": {"schema": d.input_schema}
                    }
                },
                "responses": {
                    "200": {
                        "description": "OK",
                        "content": {"application/json": {"schema": {"type": "object"}}}
                    },
                    "403": {"description": "Forbidden by policy"},
                    "404": {"description": "Not found"},
                    "501": {"description": "Not implemented"}
                }
            }
        });
        paths.insert(route, op);
    }
    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "ATLAS REST API",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Auto-generated from the MCP capability catalog (design §7.1)."
        },
        "paths": Value::Object(paths),
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
    fn list_tools_route() {
        let c = core();
        let r = handle_request(
            &c,
            &RestRequest {
                method: "GET".into(),
                path: "/v1/tools".into(),
                principal: None,
                body: json!({}),
            },
        );
        assert_eq!(r.status, 200);
    }

    #[test]
    fn openapi_lists_every_capability() {
        let spec = openapi_spec();
        let paths = spec["paths"].as_object().unwrap();
        for d in tool_descriptors() {
            assert!(paths.contains_key(&format!("/v1/tools/{}", d.name)));
        }
    }

    #[test]
    fn capability_dispatch_returns_200() {
        let c = core();
        let r = handle_request(
            &c,
            &RestRequest {
                method: "POST".into(),
                path: "/v1/tools/atlas.fs.write".into(),
                principal: Some("u".into()),
                body: json!({"path": "/a", "content": "hi"}),
            },
        );
        assert_eq!(r.status, 200);
    }

    #[test]
    fn unknown_capability_is_400() {
        let c = core();
        let r = handle_request(
            &c,
            &RestRequest {
                method: "POST".into(),
                path: "/v1/tools/atlas.bogus".into(),
                principal: None,
                body: json!({}),
            },
        );
        assert_eq!(r.status, 400);
    }

    #[test]
    fn unknown_route_is_404() {
        let c = core();
        let r = handle_request(
            &c,
            &RestRequest {
                method: "GET".into(),
                path: "/nope".into(),
                principal: None,
                body: json!({}),
            },
        );
        assert_eq!(r.status, 404);
    }
}
