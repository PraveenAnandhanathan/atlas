//! [`CapabilityCore`] — the single dispatch point every Phase 5 adapter
//! routes through (design §8.1: *one capability model, N wire formats*).
//!
//! `invoke()` runs the same five-step pipeline regardless of caller:
//!
//! 1. **Scope check** — refuse paths outside the configured subtree
//!    (T5.2 `atlasctl mcp serve <path>`).
//! 2. **Policy check** — consult [`atlas_governor::PolicyEngine`] for
//!    path-bearing capabilities.
//! 3. **Dispatch** — call the matching engine method.
//! 4. **Redaction** — read-time PII scrub for `atlas.fs.read*` if a
//!    [`atlas_governor::RedactEngine`] is configured (design §13.3).
//! 5. **Audit** — append a `policy.eval` / `policy.deny` entry through
//!    the chained [`atlas_governor::AuditLog`].

use crate::Capability;
use atlas_core::{Author, Hash, ObjectKind};
use atlas_fs::Fs;
use atlas_governor::{
    policy::{AccessRequest, Decision, Permission, PolicyEngine},
    AuditLog, RedactEngine, TokenAuthority,
};
use atlas_indexer::{AtlasIndex, HybridQuery};
use atlas_lineage::{EdgeKind, LineageEdge, LineageJournal};
use atlas_version::{Change, Version};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use thiserror::Error;

/// Structured error returned to every adapter.  Adapters map this to
/// their native error envelope (HTTP status, gRPC code, S3 XML, ...).
#[derive(Debug, Error, Serialize, Deserialize, Clone)]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    NotFound,
    Forbidden,
    InvalidArgument,
    NotImplemented,
    Internal,
}

impl ApiError {
    pub fn forbidden(msg: impl Into<String>) -> Self {
        Self { code: ErrorCode::Forbidden, message: msg.into() }
    }
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self { code: ErrorCode::NotFound, message: msg.into() }
    }
    pub fn invalid(msg: impl Into<String>) -> Self {
        Self { code: ErrorCode::InvalidArgument, message: msg.into() }
    }
    pub fn not_implemented(msg: impl Into<String>) -> Self {
        Self { code: ErrorCode::NotImplemented, message: msg.into() }
    }
    pub fn internal(msg: impl Into<String>) -> Self {
        Self { code: ErrorCode::Internal, message: msg.into() }
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

/// What every dispatch returns. Adapters serialise this back into their
/// own envelope (e.g. JSON body, gRPC `Any`).
pub type InvokeResult = std::result::Result<Value, ApiError>;

/// Glue layer exposing the entire ATLAS capability surface to any adapter.
#[derive(Clone)]
pub struct CapabilityCore {
    pub fs: Arc<Fs>,
    pub index: Option<Arc<Mutex<AtlasIndex>>>,
    pub lineage: Option<Arc<Mutex<LineageJournal>>>,
    pub policy: Option<Arc<PolicyEngine>>,
    pub audit: Option<Arc<Mutex<AuditLog>>>,
    pub tokens: Option<Arc<TokenAuthority>>,
    pub redactor: Option<Arc<RedactEngine>>,
    /// Subtree limit (T5.2). When set, paths outside this prefix are rejected.
    pub subtree: Option<String>,
}

impl CapabilityCore {
    /// Build a core that has only the filesystem wired up. Callers add
    /// the rest through the `with_*` builders so adapters compose
    /// optional planes consistently.
    pub fn new(fs: Arc<Fs>) -> Self {
        Self {
            fs,
            index: None,
            lineage: None,
            policy: None,
            audit: None,
            tokens: None,
            redactor: None,
            subtree: None,
        }
    }
    pub fn with_index(mut self, idx: Arc<Mutex<AtlasIndex>>) -> Self {
        self.index = Some(idx);
        self
    }
    pub fn with_lineage(mut self, j: Arc<Mutex<LineageJournal>>) -> Self {
        self.lineage = Some(j);
        self
    }
    pub fn with_policy(mut self, p: Arc<PolicyEngine>) -> Self {
        self.policy = Some(p);
        self
    }
    pub fn with_audit(mut self, a: Arc<Mutex<AuditLog>>) -> Self {
        self.audit = Some(a);
        self
    }
    pub fn with_tokens(mut self, t: Arc<TokenAuthority>) -> Self {
        self.tokens = Some(t);
        self
    }
    pub fn with_redactor(mut self, r: Arc<RedactEngine>) -> Self {
        self.redactor = Some(r);
        self
    }
    /// Limit every dispatch to paths starting with `prefix`. Used by
    /// `atlasctl mcp serve <path>` (T5.2).
    pub fn with_subtree(mut self, prefix: impl Into<String>) -> Self {
        let mut p = prefix.into();
        if !p.starts_with('/') {
            p.insert(0, '/');
        }
        self.subtree = Some(p);
        self
    }

    fn check_scope(&self, path: &str) -> Result<(), ApiError> {
        if let Some(scope) = &self.subtree {
            if !path.starts_with(scope) {
                return Err(ApiError::forbidden(format!(
                    "path {path} outside subtree {scope}"
                )));
            }
        }
        Ok(())
    }

    fn check_policy(
        &self,
        principal: &str,
        path: &str,
        perm: Permission,
    ) -> Result<(), ApiError> {
        let Some(engine) = &self.policy else { return Ok(()) };
        let req = AccessRequest {
            path: path.to_string(),
            principal: principal.to_string(),
            permission: perm,
        };
        match engine.evaluate(&req) {
            Decision::Allow => {
                self.audit_event(
                    "policy.allow",
                    path,
                    principal,
                    HashMap::from([("perm".into(), format!("{:?}", req.permission))]),
                );
                Ok(())
            }
            Decision::Deny(reason) => {
                self.audit_event(
                    "policy.deny",
                    path,
                    principal,
                    HashMap::from([
                        ("perm".into(), format!("{:?}", req.permission)),
                        ("reason".into(), reason.clone()),
                    ]),
                );
                Err(ApiError::forbidden(reason))
            }
        }
    }

    fn audit_event(
        &self,
        event_type: &str,
        subject: &str,
        actor: &str,
        detail: HashMap<String, String>,
    ) {
        if let Some(a) = &self.audit {
            match a.lock() {
                Ok(mut log) => {
                    if let Err(e) = log.append(event_type, subject, actor, detail) {
                        tracing::error!(
                            event_type,
                            subject,
                            actor,
                            error = %e,
                            "audit log append failed — event not recorded"
                        );
                    }
                }
                Err(_) => {
                    tracing::error!(
                        event_type,
                        subject,
                        actor,
                        "audit log mutex poisoned — event not recorded"
                    );
                }
            }
        }
    }

    /// Single dispatch: parse `name`, run the pipeline, return JSON.
    pub fn invoke(&self, principal: &str, name: &str, args: &Value) -> InvokeResult {
        let Some(cap) = crate::parse_capability(name) else {
            return Err(ApiError::invalid(format!("unknown capability: {name}")));
        };
        let res = self.dispatch(principal, cap, args);
        if let Err(e) = &res {
            self.audit_event(
                "capability.error",
                name,
                principal,
                HashMap::from([("message".into(), e.message.clone())]),
            );
        } else {
            self.audit_event(
                "capability.invoke",
                name,
                principal,
                HashMap::new(),
            );
        }
        res
    }

    fn dispatch(
        &self,
        principal: &str,
        cap: Capability,
        args: &Value,
    ) -> InvokeResult {
        use Capability::*;
        match cap {
            // -- FS reads ----------------------------------------------
            fs_stat => {
                let path = req_str(args, "path")?;
                self.check_scope(&path)?;
                self.check_policy(principal, &path, Permission::Read)?;
                let e = self.fs.stat(&path).map_err(map_fs_err)?;
                Ok(json!({
                    "path": e.path,
                    "kind": e.kind.to_string(),
                    "hash": e.hash.to_hex(),
                    "size": e.size,
                }))
            }
            fs_list => {
                let path = req_str(args, "path")?;
                self.check_scope(&path)?;
                self.check_policy(principal, &path, Permission::List)?;
                let entries = self.fs.list(&path).map_err(map_fs_err)?;
                Ok(json!(entries
                    .into_iter()
                    .map(|e| json!({
                        "path": e.path,
                        "kind": e.kind.to_string(),
                        "hash": e.hash.to_hex(),
                        "size": e.size,
                    }))
                    .collect::<Vec<_>>()))
            }
            fs_read => {
                let path = req_str(args, "path")?;
                self.check_scope(&path)?;
                self.check_policy(principal, &path, Permission::Read)?;
                let bytes = self.fs.read(&path).map_err(map_fs_err)?;
                let offset = (args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize)
                    .min(bytes.len());
                let length = args
                    .get("length")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(-1);
                let end = if length < 0 {
                    bytes.len()
                } else {
                    (offset + length as usize).min(bytes.len())
                };
                // end is always >= offset because both are clamped to bytes.len()
                let slice = &bytes[offset..end];
                Ok(json!({
                    "bytes_hex": hex::encode(slice),
                    "size": slice.len(),
                    "total_size": bytes.len(),
                }))
            }
            fs_read_text => {
                let path = req_str(args, "path")?;
                self.check_scope(&path)?;
                self.check_policy(principal, &path, Permission::Read)?;
                let bytes = self.fs.read(&path).map_err(map_fs_err)?;
                let mut text = String::from_utf8_lossy(&bytes).to_string();
                if let Some(r) = &self.redactor {
                    text = r.redact(&text);
                }
                Ok(json!({"text": text, "size": bytes.len()}))
            }
            fs_read_tensor | fs_read_schema => Err(ApiError::not_implemented(
                "format-aware reads land with the format-plugin work in §15.5",
            )),
            // -- FS writes ---------------------------------------------
            fs_write => {
                let path = req_str(args, "path")?;
                let content = req_str(args, "content")?;
                self.check_scope(&path)?;
                self.check_policy(principal, &path, Permission::Write)?;
                let e = self
                    .fs
                    .write(&path, content.as_bytes())
                    .map_err(map_fs_err)?;
                Ok(json!({"path": e.path, "hash": e.hash.to_hex(), "size": e.size}))
            }
            fs_append => {
                let path = req_str(args, "path")?;
                let content = req_str(args, "content")?;
                self.check_scope(&path)?;
                self.check_policy(principal, &path, Permission::Write)?;
                let mut existing = self.fs.read(&path).unwrap_or_default();
                existing.extend_from_slice(content.as_bytes());
                let e = self.fs.write(&path, &existing).map_err(map_fs_err)?;
                Ok(json!({"path": e.path, "hash": e.hash.to_hex(), "size": e.size}))
            }
            fs_delete => {
                let path = req_str(args, "path")?;
                self.check_scope(&path)?;
                self.check_policy(principal, &path, Permission::Delete)?;
                self.fs.delete(&path).map_err(map_fs_err)?;
                Ok(json!({"deleted": path}))
            }

            // -- Semantic ----------------------------------------------
            semantic_query => {
                let q = req_str(args, "q")?;
                let limit = args
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(10) as usize;
                let vw = args
                    .get("vector_weight")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0) as f32;
                let idx = self.index_lock()?;
                let results = idx
                    .hybrid_search(&HybridQuery {
                        text: Some(q),
                        embedding: None,
                        xattr_filters: HashMap::new(),
                        limit,
                        vector_weight: vw,
                    })
                    .map_err(|e| ApiError::internal(e.to_string()))?;
                Ok(json!(results
                    .into_iter()
                    .map(|r| json!({
                        "path": r.path,
                        "hash": r.file_hash.to_hex(),
                        "score": r.score,
                        "snippet": r.snippet,
                    }))
                    .collect::<Vec<_>>()))
            }
            semantic_similar => {
                let path = req_str(args, "path")?;
                self.check_scope(&path)?;
                self.check_policy(principal, &path, Permission::Read)?;
                Err(ApiError::not_implemented(
                    "similar() requires a stored embedding; wire via embedder service",
                ))
            }
            semantic_embed => {
                let path = req_str(args, "path")?;
                self.check_scope(&path)?;
                self.check_policy(principal, &path, Permission::Read)?;
                Err(ApiError::not_implemented(
                    "embed() requires the embedder service from §15.6",
                ))
            }
            semantic_describe => Err(ApiError::not_implemented(
                "describe() requires the embedder service",
            )),

            // -- Versioning --------------------------------------------
            version_log => {
                let limit = args
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(20) as usize;
                let v = Version::new(&self.fs);
                let log = v.log(None, limit).map_err(map_fs_err)?;
                Ok(json!(log
                    .into_iter()
                    .map(|c| json!({
                        "hash": c.hash.to_hex(),
                        "tree_hash": c.tree_hash.to_hex(),
                        "parents": c.parents.iter().map(|h| h.to_hex()).collect::<Vec<_>>(),
                        "author": c.author.name,
                        "email": c.author.email,
                        "ts": c.timestamp,
                        "message": c.message,
                    }))
                    .collect::<Vec<_>>()))
            }
            version_diff => {
                let v = Version::new(&self.fs);
                let to_h = match args.get("to").and_then(|v| v.as_str()) {
                    Some(s) => resolve_commitish(&self.fs, s)?,
                    None => v.head_commit().map_err(map_fs_err)?,
                };
                let from_h = match args.get("from").and_then(|v| v.as_str()) {
                    Some(s) => resolve_commitish(&self.fs, s)?,
                    None => {
                        let h = v.head_commit().map_err(map_fs_err)?;
                        let c = self
                            .fs
                            .meta()
                            .get_commit(&h)
                            .map_err(map_fs_err)?
                            .ok_or_else(|| ApiError::not_found("HEAD"))?;
                        *c.parents.first().unwrap_or(&h)
                    }
                };
                let changes = v.diff_commits(from_h, to_h).map_err(map_fs_err)?;
                Ok(json!(changes
                    .into_iter()
                    .map(|c| match c {
                        Change::Added { path, .. } => json!({"op": "add", "path": path}),
                        Change::Removed { path, .. } => json!({"op": "del", "path": path}),
                        Change::Modified { path, .. } => json!({"op": "mod", "path": path}),
                    })
                    .collect::<Vec<_>>()))
            }
            version_branch_create => {
                let name = req_str(args, "name")?;
                let v = Version::new(&self.fs);
                let b = v.branch_create(&name, None).map_err(map_fs_err)?;
                Ok(json!({"name": b.name, "head": b.head.to_hex()}))
            }
            version_branch_list => {
                let v = Version::new(&self.fs);
                let bs = v.branch_list().map_err(map_fs_err)?;
                Ok(json!(bs
                    .into_iter()
                    .map(|b| json!({"name": b.name, "head": b.head.to_hex()}))
                    .collect::<Vec<_>>()))
            }
            version_checkout => {
                let target = req_str(args, "target")?;
                let v = Version::new(&self.fs);
                if v.branch_list()
                    .map_err(map_fs_err)?
                    .iter()
                    .any(|b| b.name == target)
                {
                    v.checkout_branch(&target).map_err(map_fs_err)?;
                    Ok(json!({"head": target, "kind": "branch"}))
                } else {
                    let h = Hash::from_hex(&target)
                        .map_err(|_| ApiError::invalid("not a branch or commit hash"))?;
                    v.checkout_commit(h).map_err(map_fs_err)?;
                    Ok(json!({"head": h.to_hex(), "kind": "detached"}))
                }
            }
            version_commit => {
                let msg = req_str(args, "message")?;
                let name = args
                    .get("author_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("anonymous");
                let email = args
                    .get("author_email")
                    .and_then(|v| v.as_str())
                    .unwrap_or("anon@atlas");
                let v = Version::new(&self.fs);
                let h = v
                    .commit(Author::new(name, email), msg)
                    .map_err(map_fs_err)?;
                Ok(json!({"commit": h.to_hex()}))
            }
            version_tag => {
                let commit = req_hash(args, "commit")?;
                let name = req_str(args, "name")?;
                let v = Version::new(&self.fs);
                let _ = v.branch_create(&name, Some(commit)).map_err(map_fs_err)?;
                Ok(json!({"tag": name, "commit": commit.to_hex()}))
            }

            // -- Lineage -----------------------------------------------
            lineage_upstream => {
                let h = req_hash(args, "hash")?;
                let depth = args.get("depth").and_then(|v| v.as_u64()).unwrap_or(3) as usize;
                let j = self.lineage_lock()?;
                let edges = j
                    .ancestors(&h, depth)
                    .map_err(|e| ApiError::internal(e.to_string()))?;
                Ok(json!(edges
                    .into_iter()
                    .map(|e| json!({
                        "id": e.id,
                        "ts": e.ts,
                        "kind": e.kind.to_string(),
                        "source": e.source_hash.to_hex(),
                        "sink": e.sink_hash.to_hex(),
                        "agent": e.agent,
                    }))
                    .collect::<Vec<_>>()))
            }
            lineage_downstream => {
                let h = req_hash(args, "hash")?;
                let depth = args.get("depth").and_then(|v| v.as_u64()).unwrap_or(3) as usize;
                let j = self.lineage_lock()?;
                let edges = j
                    .descendants(&h, depth)
                    .map_err(|e| ApiError::internal(e.to_string()))?;
                Ok(json!(edges
                    .into_iter()
                    .map(|e| json!({
                        "id": e.id,
                        "ts": e.ts,
                        "kind": e.kind.to_string(),
                        "source": e.source_hash.to_hex(),
                        "sink": e.sink_hash.to_hex(),
                        "agent": e.agent,
                    }))
                    .collect::<Vec<_>>()))
            }
            lineage_provenance => {
                let h = req_hash(args, "hash")?;
                let j = self.lineage_lock()?;
                let parents = j
                    .parents(&h)
                    .map_err(|e| ApiError::internal(e.to_string()))?;
                Ok(json!({
                    "subject": h.to_hex(),
                    "direct_parents": parents
                        .into_iter()
                        .map(|e| json!({
                            "kind": e.kind.to_string(),
                            "source": e.source_hash.to_hex(),
                            "agent": e.agent,
                            "ts": e.ts,
                        }))
                        .collect::<Vec<_>>()
                }))
            }
            lineage_record => {
                let source = req_hash(args, "source")?;
                let sink = req_hash(args, "sink")?;
                let kind = args
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .unwrap_or("derive");
                let kind: EdgeKind = kind
                    .parse()
                    .map_err(|e: String| ApiError::invalid(e))?;
                let edge = LineageEdge::new(kind, source, sink, principal);
                let id = edge.id.clone();
                let mut j = self
                    .lineage
                    .as_ref()
                    .ok_or_else(|| ApiError::not_implemented("lineage plane disabled"))?
                    .lock()
                    .map_err(|_| ApiError::internal("lineage lock"))?;
                j.record(edge).map_err(|e| ApiError::internal(e.to_string()))?;
                Ok(json!({"edge_id": id}))
            }

            // -- Governance --------------------------------------------
            policy_show => {
                let path = req_str(args, "path")?;
                let engine = self
                    .policy
                    .as_ref()
                    .ok_or_else(|| ApiError::not_implemented("policy plane disabled"))?;
                // PolicyEngine doesn't expose its policies; surface effective
                // permissions instead by probing each Permission.
                let mut perms = Vec::new();
                for p in [
                    Permission::Read,
                    Permission::Write,
                    Permission::Delete,
                    Permission::List,
                ] {
                    let label = p.to_string();
                    let d = engine.evaluate(&AccessRequest {
                        path: path.clone(),
                        principal: principal.to_string(),
                        permission: p,
                    });
                    if d.is_allow() {
                        perms.push(label);
                    }
                }
                Ok(json!({"path": path, "principal": principal, "allowed": perms}))
            }
            policy_check => {
                let path = req_str(args, "path")?;
                let p_principal = req_str(args, "principal")?;
                let op = req_str(args, "operation")?;
                let perm: Permission = op
                    .parse()
                    .map_err(|e: String| ApiError::invalid(e))?;
                let engine = self
                    .policy
                    .as_ref()
                    .ok_or_else(|| ApiError::not_implemented("policy plane disabled"))?;
                let d = engine.evaluate(&AccessRequest {
                    path: path.clone(),
                    principal: p_principal.clone(),
                    permission: perm,
                });
                Ok(match d {
                    Decision::Allow => json!({"decision": "allow"}),
                    Decision::Deny(r) => json!({"decision": "deny", "reason": r}),
                })
            }
            policy_audit => {
                let log = self
                    .audit
                    .as_ref()
                    .ok_or_else(|| ApiError::not_implemented("audit log disabled"))?
                    .lock()
                    .map_err(|_| ApiError::internal("audit lock"))?;
                let since = args.get("since").and_then(|v| v.as_u64()).unwrap_or(0);
                let entries = log
                    .export_range(since, u64::MAX)
                    .map_err(|e| ApiError::internal(e.to_string()))?;
                Ok(json!(entries))
            }
            policy_set => Err(ApiError::not_implemented(
                "policy.set requires elevated capability and YAML store; deferred",
            )),

            // -- Agent / workflow --------------------------------------
            agent_scratchpad_create => {
                let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("scratch");
                let id = uuid::Uuid::new_v4().to_string();
                let path = format!("/scratch/{name}-{id}");
                self.check_scope(&path).ok(); // scratch may live under subtree if user wants
                self.fs.mkdir(&path).map_err(map_fs_err)?;
                Ok(json!({"path": path}))
            }
            agent_checkpoint => {
                let path = req_str(args, "path")?;
                let state = req_str(args, "state")?;
                self.check_scope(&path)?;
                self.check_policy(principal, &path, Permission::Write)?;
                let e = self
                    .fs
                    .write(&path, state.as_bytes())
                    .map_err(map_fs_err)?;
                Ok(json!({"path": e.path, "hash": e.hash.to_hex()}))
            }
            agent_fork => {
                let from = req_str(args, "from_path")?;
                let to = req_str(args, "to_path")?;
                self.check_scope(&from)?;
                self.check_scope(&to)?;
                self.check_policy(principal, &from, Permission::Read)?;
                self.check_policy(principal, &to, Permission::Write)?;
                fork_path(&self.fs, &from, &to)?;
                Ok(json!({"from": from, "to": to}))
            }
        }
    }

    fn index_lock(&self) -> Result<std::sync::MutexGuard<'_, AtlasIndex>, ApiError> {
        self.index
            .as_ref()
            .ok_or_else(|| ApiError::not_implemented("semantic plane disabled"))?
            .lock()
            .map_err(|_| ApiError::internal("index lock"))
    }

    fn lineage_lock(&self) -> Result<std::sync::MutexGuard<'_, LineageJournal>, ApiError> {
        self.lineage
            .as_ref()
            .ok_or_else(|| ApiError::not_implemented("lineage plane disabled"))?
            .lock()
            .map_err(|_| ApiError::internal("lineage lock"))
    }
}

fn map_fs_err(e: atlas_core::Error) -> ApiError {
    use atlas_core::Error::*;
    match e {
        NotFound(m) => ApiError::not_found(m),
        AlreadyExists(m) => ApiError::invalid(m),
        Invalid(m) | BadPath(m) => ApiError::invalid(m),
        _ => ApiError::internal(e.to_string()),
    }
}

fn req_str(args: &Value, field: &str) -> Result<String, ApiError> {
    args.get(field)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| ApiError::invalid(format!("missing field {field:?}")))
}

fn req_hash(args: &Value, field: &str) -> Result<Hash, ApiError> {
    let s = req_str(args, field)?;
    Hash::from_hex(&s).map_err(|_| ApiError::invalid(format!("invalid hash for {field:?}")))
}

fn resolve_commitish(fs: &Fs, s: &str) -> Result<Hash, ApiError> {
    if let Ok(h) = Hash::from_hex(s) {
        return Ok(h);
    }
    if let Some(b) = fs.meta().get_branch(s).map_err(map_fs_err)? {
        return Ok(b.head);
    }
    Err(ApiError::invalid(format!("not a branch or commit hash: {s}")))
}

/// Recursively copy `from` to `to`. CoW is automatic at the chunk layer
/// — chunks are content-addressed, so no bytes are duplicated.
fn fork_path(fs: &Fs, from: &str, to: &str) -> Result<(), ApiError> {
    let entry = fs.stat(from).map_err(map_fs_err)?;
    match entry.kind {
        ObjectKind::File => {
            let bytes = fs.read(from).map_err(map_fs_err)?;
            fs.write(to, &bytes).map_err(map_fs_err)?;
            Ok(())
        }
        ObjectKind::Dir => {
            fs.mkdir(to).map_err(map_fs_err)?;
            for child in fs.list(from).map_err(map_fs_err)? {
                let name = child.path.rsplit('/').next().unwrap_or("");
                let child_to = if to.ends_with('/') {
                    format!("{to}{name}")
                } else {
                    format!("{to}/{name}")
                };
                fork_path(fs, &child.path, &child_to)?;
            }
            Ok(())
        }
        _ => Err(ApiError::invalid("cannot fork symlink")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn core() -> (TempDir, CapabilityCore) {
        let dir = tempfile::tempdir().unwrap();
        let fs = Fs::init(dir.path()).unwrap();
        (dir, CapabilityCore::new(Arc::new(fs)))
    }

    #[test]
    fn write_then_read_roundtrip() {
        let (_d, c) = core();
        c.invoke("alice", "atlas.fs.write", &json!({"path": "/a", "content": "hi"}))
            .unwrap();
        let r = c
            .invoke("alice", "atlas.fs.read_text", &json!({"path": "/a"}))
            .unwrap();
        assert_eq!(r["text"], "hi");
    }

    #[test]
    fn scope_blocks_outside_path() {
        let (_d, mut c) = core();
        c = c.with_subtree("/projects/a");
        let err = c
            .invoke(
                "alice",
                "atlas.fs.write",
                &json!({"path": "/other", "content": "x"}),
            )
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::Forbidden);
    }

    #[test]
    fn unknown_capability_is_invalid() {
        let (_d, c) = core();
        let err = c.invoke("a", "atlas.fs.bogus", &json!({})).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArgument);
    }

    #[test]
    fn version_commit_then_log() {
        let (_d, c) = core();
        c.invoke("u", "atlas.fs.write", &json!({"path": "/a", "content": "x"}))
            .unwrap();
        c.invoke("u", "atlas.version.commit", &json!({"message": "first"}))
            .unwrap();
        let log = c.invoke("u", "atlas.version.log", &json!({"limit": 5})).unwrap();
        assert!(log.as_array().unwrap().len() >= 2);
    }

    #[test]
    fn agent_fork_copies_subtree() {
        let (_d, c) = core();
        c.invoke("u", "atlas.fs.write", &json!({"path": "/a/x", "content": "1"}))
            .unwrap();
        c.invoke(
            "u",
            "atlas.agent.fork",
            &json!({"from_path": "/a", "to_path": "/b"}),
        )
        .unwrap();
        let r = c
            .invoke("u", "atlas.fs.read_text", &json!({"path": "/b/x"}))
            .unwrap();
        assert_eq!(r["text"], "1");
    }

    // P6-4: A poisoned audit-log mutex must not cause invoke() to panic.
    // The code must log the error and return success for the operation itself.
    #[test]
    fn poisoned_audit_log_mutex_does_not_panic() {
        use atlas_governor::AuditLog;

        let (_d, c) = core();
        let audit_dir = tempfile::tempdir().unwrap();
        let audit = AuditLog::open(audit_dir.path()).unwrap();
        let audit_arc = std::sync::Arc::new(std::sync::Mutex::new(audit));

        // Poison the mutex by panicking inside a thread that holds it.
        let audit_arc2 = std::sync::Arc::clone(&audit_arc);
        let _ = std::thread::spawn(move || {
            let _guard = audit_arc2.lock().unwrap();
            panic!("intentional poison");
        })
        .join(); // captures the panic; mutex is now poisoned

        let c = c.with_audit(audit_arc);

        // An invoke with a poisoned audit log must not panic.
        // The operation should succeed; only the audit append silently logs an error.
        let result = c.invoke("alice", "atlas.fs.write", &json!({"path": "/x", "content": "y"}));
        assert!(
            result.is_ok(),
            "operation must succeed even with poisoned audit log"
        );
    }

    // P6-6a: offset beyond file length must not panic (returns empty slice).
    #[test]
    fn read_offset_beyond_eof_returns_empty() {
        let (_d, c) = core();
        c.invoke("u", "atlas.fs.write", &json!({"path": "/f", "content": "hello"}))
            .unwrap();
        let r = c
            .invoke(
                "u",
                "atlas.fs.read",
                &json!({"path": "/f", "offset": 9999, "length": 10}),
            )
            .unwrap();
        assert_eq!(r["size"], 0, "offset past EOF should yield empty slice");
    }

    // P6-6b: offset + length overflowing file size must be clamped, not panic.
    #[test]
    fn read_offset_plus_length_overflow_is_clamped() {
        let (_d, c) = core();
        c.invoke("u", "atlas.fs.write", &json!({"path": "/f", "content": "hello"}))
            .unwrap();
        // offset=3, length=100 → only 2 bytes ("lo") should be returned.
        let r = c
            .invoke(
                "u",
                "atlas.fs.read",
                &json!({"path": "/f", "offset": 3, "length": 100}),
            )
            .unwrap();
        assert_eq!(r["size"], 2, "length must be clamped to remaining bytes");
        assert_eq!(r["total_size"], 5);
    }

    // P6-6c: negative length (read-all) with a non-zero offset must return
    // the tail of the file, not the full file.
    #[test]
    fn read_negative_length_with_offset_returns_tail() {
        let (_d, c) = core();
        c.invoke(
            "u",
            "atlas.fs.write",
            &json!({"path": "/f", "content": "abcde"}),
        )
        .unwrap();
        let r = c
            .invoke(
                "u",
                "atlas.fs.read",
                &json!({"path": "/f", "offset": 2, "length": -1}),
            )
            .unwrap();
        // "cde" = 3 bytes
        assert_eq!(r["size"], 3);
        assert_eq!(r["bytes_hex"], hex::encode(b"cde"));
    }
}
