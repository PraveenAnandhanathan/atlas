//! ATLAS Model-Context-Protocol server (T5.1) and the shared capability
//! core every Phase 5 adapter funnels through.
//!
//! Every protocol crate (MCP, A2A, REST, gRPC, S3, tool-use JSON) is a
//! thin translator that converts its native wire format into a
//! [`CapabilityCore::invoke`] call and back.  This is what design §8.1
//! calls *one capability model, N wire formats* — it is what keeps the
//! governance plane authoritative regardless of how an agent connects.
//!
//! # Layout
//!
//! - [`capability`] — enumerates the catalog from design §7.1 and parses
//!   tool names into typed [`Capability`] values.
//! - [`core`] — the [`CapabilityCore`] runtime: a single
//!   [`CapabilityCore::invoke`] entry point with policy + redaction +
//!   audit hooks fired around every call.
//! - [`tools`] — the JSON-Schema descriptor table consumed by MCP tool
//!   discovery and reused by the OpenAI/Anthropic adapters in
//!   [`atlas-toolwire`].
//! - [`wire`] — the JSON-RPC envelope used by `atlasctl mcp serve`
//!   (T5.2) and the conformance harness (T5.8).

pub mod capability;
pub mod core;
pub mod tools;
pub mod wire;

pub use capability::{parse_capability, Capability};
pub use core::{ApiError, CapabilityCore, InvokeResult};
pub use tools::{tool_descriptors, ToolDescriptor};
pub use wire::{handle_line, McpRequest, McpResponse, RpcError};
