//! JSON-Schema descriptors for every capability (T5.1 tool catalog).
//!
//! These descriptors are consumed by:
//!
//! - the MCP `tools/list` handshake in [`crate::wire`],
//! - the OpenAI / Anthropic translators in `atlas-toolwire` (T5.7),
//! - the OpenAPI generator in `atlas-rest` (T5.4),
//! - the gRPC reflection table in `atlas-grpc` (T5.5).
//!
//! Keeping them in one place is what makes the conformance harness
//! (T5.8) cheap: the wire formats differ but the schema is identical.

use crate::Capability;
use serde::Serialize;
use serde_json::{json, Value};

/// One MCP tool descriptor.
#[derive(Debug, Clone, Serialize)]
pub struct ToolDescriptor {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
    /// The capability requires the caller to be able to mutate state.
    pub mutates: bool,
}

fn obj(props: Vec<(&str, Value)>, required: &[&str]) -> Value {
    let mut p = serde_json::Map::new();
    for (k, v) in props {
        p.insert(k.to_string(), v);
    }
    json!({
        "type": "object",
        "properties": Value::Object(p),
        "required": required,
    })
}

fn s(t: &str, desc: &str) -> Value {
    json!({"type": t, "description": desc})
}

/// All descriptors in catalog order.
pub fn tool_descriptors() -> Vec<ToolDescriptor> {
    use Capability::*;
    let mut out = Vec::new();
    let path = || s("string", "Absolute path inside the volume.");
    let hash = || s("string", "64-hex content hash.");

    out.push(ToolDescriptor {
        name: fs_stat.name(),
        description: "Metadata for a path: kind, hash, size.",
        input_schema: obj(vec![("path", path())], &["path"]),
        mutates: false,
    });
    out.push(ToolDescriptor {
        name: fs_list.name(),
        description: "Directory listing with rich metadata.",
        input_schema: obj(
            vec![
                ("path", path()),
                ("recursive", s("boolean", "Recurse into subdirectories.")),
            ],
            &["path"],
        ),
        mutates: false,
    });
    out.push(ToolDescriptor {
        name: fs_read.name(),
        description: "Raw bytes (returned base16-encoded).",
        input_schema: obj(
            vec![
                ("path", path()),
                ("offset", s("integer", "Starting byte offset.")),
                ("length", s("integer", "Bytes to read; -1 = all.")),
            ],
            &["path"],
        ),
        mutates: false,
    });
    out.push(ToolDescriptor {
        name: fs_read_text.name(),
        description: "Format-aware text extraction (UTF-8 lossy fallback).",
        input_schema: obj(vec![("path", path())], &["path"]),
        mutates: false,
    });
    out.push(ToolDescriptor {
        name: fs_read_tensor.name(),
        description: "Format-aware tensor slice (safetensors/zarr).",
        input_schema: obj(
            vec![("path", path()), ("tensor_name", s("string", "Tensor."))],
            &["path", "tensor_name"],
        ),
        mutates: false,
    });
    out.push(ToolDescriptor {
        name: fs_read_schema.name(),
        description: "Extract schema from Parquet/JSON/CSV.",
        input_schema: obj(vec![("path", path())], &["path"]),
        mutates: false,
    });
    out.push(ToolDescriptor {
        name: fs_write.name(),
        description: "Create or overwrite a file with `content` (UTF-8).",
        input_schema: obj(
            vec![("path", path()), ("content", s("string", "File content."))],
            &["path", "content"],
        ),
        mutates: true,
    });
    out.push(ToolDescriptor {
        name: fs_append.name(),
        description: "Append text to a file (creates if absent).",
        input_schema: obj(
            vec![("path", path()), ("content", s("string", "Bytes to append."))],
            &["path", "content"],
        ),
        mutates: true,
    });
    out.push(ToolDescriptor {
        name: fs_delete.name(),
        description: "Delete a file or empty directory.",
        input_schema: obj(vec![("path", path())], &["path"]),
        mutates: true,
    });

    // Semantic
    out.push(ToolDescriptor {
        name: semantic_query.name(),
        description: "Hybrid keyword + vector search returning ranked paths.",
        input_schema: obj(
            vec![
                ("q", s("string", "Free-text query.")),
                ("limit", s("integer", "Max results.")),
                ("vector_weight", s("number", "Vector weight in [0,1].")),
            ],
            &["q"],
        ),
        mutates: false,
    });
    out.push(ToolDescriptor {
        name: semantic_similar.name(),
        description: "Find files similar to a given path.",
        input_schema: obj(
            vec![("path", path()), ("limit", s("integer", "Max results."))],
            &["path"],
        ),
        mutates: false,
    });
    out.push(ToolDescriptor {
        name: semantic_embed.name(),
        description: "Force re-embed a path; returns model-version tag.",
        input_schema: obj(
            vec![("path", path()), ("model", s("string", "Model id."))],
            &["path"],
        ),
        mutates: true,
    });
    out.push(ToolDescriptor {
        name: semantic_describe.name(),
        description: "LLM-generated summary (cached).",
        input_schema: obj(vec![("path", path())], &["path"]),
        mutates: false,
    });

    // Versioning
    out.push(ToolDescriptor {
        name: version_log.name(),
        description: "Commit history walking back from HEAD.",
        input_schema: obj(vec![("limit", s("integer", "Max commits."))], &[]),
        mutates: false,
    });
    out.push(ToolDescriptor {
        name: version_diff.name(),
        description: "Diff two commits (or HEAD~1 vs HEAD when omitted).",
        input_schema: obj(
            vec![
                ("from", s("string", "Branch/commit hash.")),
                ("to", s("string", "Branch/commit hash.")),
            ],
            &[],
        ),
        mutates: false,
    });
    out.push(ToolDescriptor {
        name: version_branch_create.name(),
        description: "Create a new branch (O(1)).",
        input_schema: obj(vec![("name", s("string", "Branch name."))], &["name"]),
        mutates: true,
    });
    out.push(ToolDescriptor {
        name: version_branch_list.name(),
        description: "List branches with their head commits.",
        input_schema: obj(vec![], &[]),
        mutates: false,
    });
    out.push(ToolDescriptor {
        name: version_checkout.name(),
        description: "Switch HEAD to a branch or detached commit.",
        input_schema: obj(
            vec![("target", s("string", "Branch name or commit hash."))],
            &["target"],
        ),
        mutates: true,
    });
    out.push(ToolDescriptor {
        name: version_commit.name(),
        description: "Snapshot working root onto current branch.",
        input_schema: obj(
            vec![
                ("message", s("string", "Commit message.")),
                ("author_name", s("string", "")),
                ("author_email", s("string", "")),
            ],
            &["message"],
        ),
        mutates: true,
    });
    out.push(ToolDescriptor {
        name: version_tag.name(),
        description: "Attach a tag to a commit (stored as a branch ref).",
        input_schema: obj(
            vec![("commit", hash()), ("name", s("string", "Tag name."))],
            &["commit", "name"],
        ),
        mutates: true,
    });

    // Lineage
    out.push(ToolDescriptor {
        name: lineage_upstream.name(),
        description: "Producers (ancestors) of a content hash.",
        input_schema: obj(
            vec![("hash", hash()), ("depth", s("integer", "BFS depth."))],
            &["hash"],
        ),
        mutates: false,
    });
    out.push(ToolDescriptor {
        name: lineage_downstream.name(),
        description: "Consumers (descendants) of a content hash.",
        input_schema: obj(
            vec![("hash", hash()), ("depth", s("integer", "BFS depth."))],
            &["hash"],
        ),
        mutates: false,
    });
    out.push(ToolDescriptor {
        name: lineage_provenance.name(),
        description: "Signed provenance statement for a hash.",
        input_schema: obj(vec![("hash", hash())], &["hash"]),
        mutates: false,
    });
    out.push(ToolDescriptor {
        name: lineage_record.name(),
        description: "Explicit edge: declare an input→output relationship.",
        input_schema: obj(
            vec![
                ("source", hash()),
                ("sink", hash()),
                ("kind", s("string", "read|write|derive|copy|transform")),
            ],
            &["source", "sink"],
        ),
        mutates: true,
    });

    // Governance
    out.push(ToolDescriptor {
        name: policy_show.name(),
        description: "Effective policy at a path.",
        input_schema: obj(vec![("path", path())], &["path"]),
        mutates: false,
    });
    out.push(ToolDescriptor {
        name: policy_check.name(),
        description: "Can `principal` perform `operation` at `path`?",
        input_schema: obj(
            vec![
                ("path", path()),
                ("principal", s("string", "Principal id.")),
                ("operation", s("string", "read|write|delete|list")),
            ],
            &["path", "principal", "operation"],
        ),
        mutates: false,
    });
    out.push(ToolDescriptor {
        name: policy_audit.name(),
        description: "Audit log entries (optionally filtered by `since` seq).",
        input_schema: obj(
            vec![("since", s("integer", "Start sequence (inclusive)."))],
            &[],
        ),
        mutates: false,
    });
    out.push(ToolDescriptor {
        name: policy_set.name(),
        description: "Set policy at path (requires elevated capability).",
        input_schema: obj(
            vec![("path", path()), ("policy_yaml", s("string", "YAML body."))],
            &["path", "policy_yaml"],
        ),
        mutates: true,
    });

    // Agent / workflow
    out.push(ToolDescriptor {
        name: agent_scratchpad_create.name(),
        description: "Isolated working area under /scratch/<uuid>.",
        input_schema: obj(vec![("name", s("string", "Optional name hint."))], &[]),
        mutates: true,
    });
    out.push(ToolDescriptor {
        name: agent_checkpoint.name(),
        description: "Snapshot agent state into the FS.",
        input_schema: obj(
            vec![("path", path()), ("state", s("string", "JSON state blob."))],
            &["path", "state"],
        ),
        mutates: true,
    });
    out.push(ToolDescriptor {
        name: agent_fork.name(),
        description: "Copy-on-write fork of a path for parallel exploration.",
        input_schema: obj(
            vec![("from_path", path()), ("to_path", path())],
            &["from_path", "to_path"],
        ),
        mutates: true,
    });

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptors_cover_every_capability() {
        let names: std::collections::HashSet<_> =
            tool_descriptors().iter().map(|d| d.name).collect();
        for c in Capability::all() {
            assert!(names.contains(c.name()), "missing descriptor for {}", c.name());
        }
    }
}
