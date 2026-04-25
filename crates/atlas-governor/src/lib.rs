//! ATLAS governance plane (T4.4 – T4.9).
//!
//! Components:
//! - `policy`  — YAML-driven rule engine evaluated at open / write time.
//! - `token`   — Ed25519-signed capability tokens (issue / verify / revoke).
//! - `redact`  — Read-time PII redaction (emails, SSNs, API keys, custom).
//! - `audit`   — SHA-256 chained tamper-evident audit log.
//! - `sign`    — Low-level Ed25519 signing utilities for commits and policy.

pub mod audit;
pub mod policy;
pub mod redact;
pub mod sign;
pub mod token;

pub use audit::AuditLog;
pub use policy::{AccessRequest, Decision, Effect, Permission, Policy, PolicyEngine, Rule};
pub use redact::{RedactConfig, RedactEngine};
pub use sign::{sign_bytes, verify_bytes, SignedEnvelope};
pub use token::{CapabilityToken, TokenAuthority};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum GovernorError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("yaml: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("signing: {0}")]
    Sign(String),
    #[error("policy: {0}")]
    Policy(String),
    #[error("token: {0}")]
    Token(String),
    #[error("audit: {0}")]
    Audit(String),
    #[error("regex: {0}")]
    Regex(#[from] regex::Error),
}

pub type Result<T> = std::result::Result<T, GovernorError>;
