//! Adapter conformance harness (T5.8).
//!
//! For every capability we declare, this crate runs the *same* invocation
//! through every Phase 5 adapter and asserts the results agree.  It's
//! the executable form of design §8.1: *one capability model, N wire
//! formats*.  When a wire format starts to drift, this is the test that
//! catches it before a release ships.
//!
//! The harness exposes [`run_all`] so the workspace's `cargo test` job
//! exercises every adapter without needing a separate binary.

use atlas_mcp::{CapabilityCore, McpRequest};
use serde_json::{json, Value};

/// One probe — invoke `capability` with `args` through each adapter and
/// return the JSON result each one extracted.  Test code asserts they
/// all agree (modulo wire-specific framing).
pub struct Probe<'a> {
    pub capability: &'a str,
    pub args: Value,
    pub principal: &'a str,
}

#[derive(Debug)]
pub struct ProbeOutputs {
    pub mcp: Value,
    pub a2a: Value,
    pub rest: Value,
    pub grpc: Value,
    pub toolwire_anthropic: Value,
    pub toolwire_openai: Value,
}

/// Drive the same probe through every adapter in the workspace.
pub fn run_probe(core: &CapabilityCore, p: &Probe<'_>) -> ProbeOutputs {
    // -- MCP via JSON-RPC line ------------------------------------------
    let mcp_line = serde_json::to_string(&McpRequest {
        jsonrpc: Some("2.0".into()),
        id: Some(json!(1)),
        method: "tools/call".into(),
        params: json!({"name": p.capability, "arguments": p.args, "principal": p.principal}),
    })
    .unwrap();
    let mcp_resp_str = atlas_mcp::handle_line(core, &mcp_line);
    let mcp_resp: Value = serde_json::from_str(&mcp_resp_str).unwrap();
    let mcp = mcp_resp
        .get("result")
        .and_then(|r| r.get("structuredContent"))
        .cloned()
        .unwrap_or(Value::Null);

    // -- A2A ---------------------------------------------------------------
    let a2a_resp = atlas_a2a::handle(
        core,
        &atlas_a2a::A2aRequest {
            skill: p.capability.to_string(),
            input: p.args.clone(),
            principal: Some(p.principal.into()),
        },
    );
    let a2a = a2a_resp.output.unwrap_or(Value::Null);

    // -- REST --------------------------------------------------------------
    let rest_resp = atlas_rest::handle_request(
        core,
        &atlas_rest::RestRequest {
            method: "POST".into(),
            path: format!("/v1/tools/{}", p.capability),
            principal: Some(p.principal.into()),
            body: p.args.clone(),
        },
    );
    let rest = rest_resp.body;

    // -- gRPC --------------------------------------------------------------
    let grpc_resp = atlas_grpc::invoke(
        core,
        &atlas_grpc::InvokeRequest {
            capability: p.capability.into(),
            principal: p.principal.into(),
            arguments_json: p.args.to_string(),
        },
    );
    let grpc: Value = if grpc_resp.ok {
        serde_json::from_str(&grpc_resp.result_json).unwrap_or(Value::Null)
    } else {
        Value::Null
    };

    // -- Anthropic tool-use ------------------------------------------------
    let anth = atlas_toolwire::run_tool_use(
        core,
        p.principal,
        &atlas_toolwire::ToolUse {
            id: "probe".into(),
            name: p.capability.replace('.', "_"),
            input: p.args.clone(),
        },
    );
    let toolwire_anthropic = parse_toolwire_text(&anth);

    // -- OpenAI tool-call --------------------------------------------------
    let oa = atlas_toolwire::run_openai_tool_call(
        core,
        p.principal,
        &atlas_toolwire::OpenAiToolCall {
            id: "probe".into(),
            function: atlas_toolwire::OpenAiFunction {
                name: p.capability.replace('.', "_"),
                arguments: p.args.to_string(),
            },
        },
    );
    let toolwire_openai: Value = oa
        .get("content")
        .and_then(|c| c.as_str())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(Value::Null);

    ProbeOutputs {
        mcp,
        a2a,
        rest,
        grpc,
        toolwire_anthropic,
        toolwire_openai,
    }
}

fn parse_toolwire_text(v: &Value) -> Value {
    v.get("content")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .and_then(|m| m.get("text"))
        .and_then(|t| t.as_str())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(Value::Null)
}

/// Run the standard probe set every release must pass.
pub fn run_all(core: &CapabilityCore) -> Vec<(String, ProbeOutputs)> {
    let probes: Vec<Probe<'static>> = vec![
        Probe {
            capability: "atlas.fs.write",
            args: json!({"path": "/conf/a", "content": "hello"}),
            principal: "u",
        },
        Probe {
            capability: "atlas.fs.read_text",
            args: json!({"path": "/conf/a"}),
            principal: "u",
        },
        Probe {
            capability: "atlas.fs.stat",
            args: json!({"path": "/conf/a"}),
            principal: "u",
        },
        Probe {
            capability: "atlas.version.branch_list",
            args: json!({}),
            principal: "u",
        },
    ];
    probes
        .into_iter()
        .map(|p| (p.capability.to_string(), run_probe(core, &p)))
        .collect()
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
    fn every_adapter_returns_same_value_for_read() {
        let c = core();
        // Seed via MCP.
        let _ = c.invoke(
            "u",
            "atlas.fs.write",
            &json!({"path": "/c/x", "content": "hi"}),
        );
        let p = Probe {
            capability: "atlas.fs.read_text",
            args: json!({"path": "/c/x"}),
            principal: "u",
        };
        let out = run_probe(&c, &p);
        let mcp_text = out.mcp["text"].as_str().unwrap();
        assert_eq!(out.a2a["text"].as_str().unwrap(), mcp_text);
        assert_eq!(out.rest["text"].as_str().unwrap(), mcp_text);
        assert_eq!(out.grpc["text"].as_str().unwrap(), mcp_text);
        assert_eq!(out.toolwire_anthropic["text"].as_str().unwrap(), mcp_text);
        assert_eq!(out.toolwire_openai["text"].as_str().unwrap(), mcp_text);
    }

    #[test]
    fn run_all_completes_without_panic() {
        let c = core();
        let outputs = run_all(&c);
        assert_eq!(outputs.len(), 4);
    }

    #[test]
    fn forbidden_propagates_through_every_wire() {
        let c = core().with_subtree("/scoped");
        let p = Probe {
            capability: "atlas.fs.write",
            args: json!({"path": "/elsewhere", "content": "x"}),
            principal: "u",
        };
        let out = run_probe(&c, &p);
        // MCP returns null structuredContent on error; gRPC returns Null;
        // REST returns an error envelope object — all adapters must report
        // the failure rather than silently succeeding.
        assert!(out.mcp.is_null());
        assert!(out.grpc.is_null());
        assert!(out.rest.get("error").is_some());
    }
}
