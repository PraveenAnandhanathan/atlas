//! YAML-driven policy engine (T4.4) with lineage-constraint enforcement (T4.7).
//!
//! Policy files use a simple YAML format:
//! ```yaml
//! version: "1"
//! rules:
//!   - path_pattern: "/public/**"
//!     principals: ["*"]
//!     permissions: [read, list]
//!     effect: allow
//!   - path_pattern: "/secret/**"
//!     principals: ["admin"]
//!     permissions: [read, write, delete, list]
//!     effect: allow
//! ```
//!
//! Rules are evaluated in order. The first matching rule wins.
//! If no rule matches, access is **denied** (default-deny).

use crate::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    Read,
    Write,
    Delete,
    List,
}

impl std::fmt::Display for Permission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Delete => "delete",
            Self::List => "list",
        };
        write!(f, "{s}")
    }
}

impl std::str::FromStr for Permission {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        match s {
            "read" => Ok(Self::Read),
            "write" => Ok(Self::Write),
            "delete" => Ok(Self::Delete),
            "list" => Ok(Self::List),
            _ => Err(format!("unknown permission: {s:?}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    Allow,
    Deny,
}

/// One policy rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    /// Glob-like path pattern. `*` matches one segment, `**` matches any subtree.
    pub path_pattern: String,
    /// Principals this rule applies to. `["*"]` = everyone.
    pub principals: Vec<String>,
    /// Permissions covered by this rule.
    pub permissions: Vec<Permission>,
    /// Whether matching requests are allowed or denied.
    pub effect: Effect,
}

/// A complete policy document (one YAML file).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

fn default_version() -> String {
    "1".into()
}

impl Policy {
    pub fn from_yaml(yaml: &str) -> Result<Self> {
        Ok(serde_yaml::from_str(yaml)?)
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let text = std::fs::read_to_string(path)?;
        Self::from_yaml(&text)
    }

    /// Build a permissive policy that allows everything.
    pub fn allow_all() -> Self {
        Self {
            version: "1".into(),
            rules: vec![Rule {
                path_pattern: "**".into(),
                principals: vec!["*".into()],
                permissions: vec![
                    Permission::Read,
                    Permission::Write,
                    Permission::Delete,
                    Permission::List,
                ],
                effect: Effect::Allow,
            }],
        }
    }
}

/// Incoming access request.
pub struct AccessRequest {
    pub path: String,
    pub principal: String,
    pub permission: Permission,
}

/// Result of evaluating a request against policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny(String),
}

impl Decision {
    pub fn is_allow(&self) -> bool {
        matches!(self, Self::Allow)
    }
}

/// Multi-policy engine.  Policies are evaluated in load order.
/// Within each policy, rules are tested in order; the first match decides.
/// Default when nothing matches: **Deny**.
#[derive(Default)]
pub struct PolicyEngine {
    policies: Vec<Policy>,
}

impl PolicyEngine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_policy(&mut self, policy: Policy) {
        self.policies.push(policy);
    }

    pub fn load_yaml_file(&mut self, path: impl AsRef<Path>) -> Result<()> {
        self.policies.push(Policy::load(path)?);
        Ok(())
    }

    /// Evaluate whether `req` is permitted.
    pub fn evaluate(&self, req: &AccessRequest) -> Decision {
        for policy in &self.policies {
            for rule in &policy.rules {
                if !path_matches(&rule.path_pattern, &req.path) {
                    continue;
                }
                if !principal_matches(&rule.principals, &req.principal) {
                    continue;
                }
                if !rule.permissions.contains(&req.permission) {
                    continue;
                }
                return match rule.effect {
                    Effect::Allow => Decision::Allow,
                    Effect::Deny => Decision::Deny(format!(
                        "denied by rule (pattern={} principal={} perm={})",
                        rule.path_pattern, req.principal, req.permission
                    )),
                };
            }
        }
        Decision::Deny(format!(
            "no matching allow rule (path={} principal={} perm={})",
            req.path, req.principal, req.permission
        ))
    }

    /// Enforce lineage constraint on write (T4.7): reject if any source
    /// that feeds the write is inaccessible to the principal.
    pub fn check_lineage_constraint(
        &self,
        sink_path: &str,
        source_paths: &[&str],
        principal: &str,
    ) -> Decision {
        for &src in source_paths {
            let d = self.evaluate(&AccessRequest {
                path: src.to_string(),
                principal: principal.to_string(),
                permission: Permission::Read,
            });
            if let Decision::Deny(reason) = d {
                return Decision::Deny(format!(
                    "lineage constraint: write to {sink_path} blocked — {reason}"
                ));
            }
        }
        self.evaluate(&AccessRequest {
            path: sink_path.to_string(),
            principal: principal.to_string(),
            permission: Permission::Write,
        })
    }
}

// ---------------------------------------------------------------------------
// Pattern matching helpers
// ---------------------------------------------------------------------------

fn path_matches(pattern: &str, path: &str) -> bool {
    if pattern == "**" || pattern == "*" {
        return true;
    }
    let pat: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    let seg: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    match_segs(&pat, &seg)
}

fn match_segs(pat: &[&str], seg: &[&str]) -> bool {
    match pat.first() {
        None => seg.is_empty(),
        Some(&"**") => {
            for i in 0..=seg.len() {
                if match_segs(&pat[1..], &seg[i..]) {
                    return true;
                }
            }
            false
        }
        Some(&p) => {
            if seg.is_empty() {
                return false;
            }
            if glob_seg(p, seg[0]) {
                match_segs(&pat[1..], &seg[1..])
            } else {
                false
            }
        }
    }
}

fn glob_seg(pattern: &str, segment: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if !pattern.contains('*') {
        return pattern == segment;
    }
    // Simple prefix*suffix matching
    let parts: Vec<&str> = pattern.splitn(2, '*').collect();
    segment.starts_with(parts[0]) && segment.ends_with(parts[1])
}

fn principal_matches(principals: &[String], principal: &str) -> bool {
    principals.iter().any(|p| p == "*" || p == principal)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine_with_yaml(yaml: &str) -> PolicyEngine {
        let mut e = PolicyEngine::new();
        e.add_policy(Policy::from_yaml(yaml).unwrap());
        e
    }

    fn req(path: &str, principal: &str, perm: Permission) -> AccessRequest {
        AccessRequest {
            path: path.into(),
            principal: principal.into(),
            permission: perm,
        }
    }

    const YAML: &str = r#"
version: "1"
rules:
  - path_pattern: "/public/**"
    principals: ["*"]
    permissions: [read, list]
    effect: allow
  - path_pattern: "/secret/**"
    principals: ["admin"]
    permissions: [read, write, delete, list]
    effect: allow
  - path_pattern: "/secret/**"
    principals: ["*"]
    permissions: [read, write, delete, list]
    effect: deny
"#;

    #[test]
    fn allow_public_read() {
        let e = engine_with_yaml(YAML);
        assert!(e
            .evaluate(&req("/public/foo.txt", "alice", Permission::Read))
            .is_allow());
    }

    #[test]
    fn deny_public_write() {
        let e = engine_with_yaml(YAML);
        assert!(!e
            .evaluate(&req("/public/foo.txt", "alice", Permission::Write))
            .is_allow());
    }

    #[test]
    fn admin_secret_allow() {
        let e = engine_with_yaml(YAML);
        assert!(e
            .evaluate(&req("/secret/keys.txt", "admin", Permission::Read))
            .is_allow());
    }

    #[test]
    fn non_admin_secret_deny() {
        let e = engine_with_yaml(YAML);
        assert!(!e
            .evaluate(&req("/secret/keys.txt", "alice", Permission::Read))
            .is_allow());
    }

    #[test]
    fn default_deny_unmatched() {
        let e = engine_with_yaml(YAML);
        assert!(!e
            .evaluate(&req("/other/file", "alice", Permission::Read))
            .is_allow());
    }

    #[test]
    fn lineage_constraint_blocked_source() {
        let e = engine_with_yaml(YAML);
        let d = e.check_lineage_constraint("/public/out", &["/secret/input"], "alice");
        assert!(!d.is_allow());
    }

    #[test]
    fn lineage_constraint_ok() {
        let e = engine_with_yaml(YAML);
        let d = e.check_lineage_constraint("/public/out", &["/public/in.txt"], "alice");
        // alice can read /public/in.txt but can't write /public/out → denied at write check
        assert!(!d.is_allow()); // write denied by default
    }
}
