//! Ed25519 capability token issuance, verification, and revocation (T4.5).
//!
//! Tokens are JSON objects signed over a canonical body that omits the
//! `signature` field.  The signing key is stored as a hex-encoded seed
//! in `<gov_dir>/signing.key`; revoked IDs are persisted to
//! `<gov_dir>/revoked.json`.

use crate::{policy::Permission, GovernorError, Result};
use ed25519_dalek::{Signer, SigningKey, Verifier};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// A signed capability granting specific permissions on a path scope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityToken {
    pub id: String,
    pub principal: String,
    /// All paths that start with this prefix are covered.
    pub scope_path: String,
    pub permissions: Vec<Permission>,
    /// Unix seconds.
    pub issued_at: u64,
    /// Unix seconds.
    pub expires_at: u64,
    /// Hex-encoded Ed25519 signature over the canonical token body.
    pub signature: String,
}

impl CapabilityToken {
    /// Deterministic JSON of the unsigned body (fields sorted alphabetically,
    /// `signature` excluded).
    fn body_bytes(&self) -> Vec<u8> {
        #[derive(Serialize)]
        struct Body<'a> {
            expires_at: u64,
            id: &'a str,
            issued_at: u64,
            permissions: &'a Vec<Permission>,
            principal: &'a str,
            scope_path: &'a str,
        }
        serde_json::to_vec(&Body {
            expires_at: self.expires_at,
            id: &self.id,
            issued_at: self.issued_at,
            permissions: &self.permissions,
            principal: &self.principal,
            scope_path: &self.scope_path,
        })
        .unwrap_or_default()
    }

    pub fn is_expired(&self) -> bool {
        now_secs() > self.expires_at
    }

    /// Returns true if this token covers `path` with `permission`.
    pub fn covers(&self, path: &str, permission: &Permission) -> bool {
        path.starts_with(&self.scope_path) && self.permissions.contains(permission)
    }

    pub fn encode(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }

    pub fn decode(s: &str) -> Result<Self> {
        Ok(serde_json::from_str(s)?)
    }
}

/// Manages the signing keypair and revocation list for one governance domain.
pub struct TokenAuthority {
    key: SigningKey,
    revoked: HashSet<String>,
    revoked_path: PathBuf,
}

impl TokenAuthority {
    /// Generate a fresh keypair under `dir` (overwrites existing).
    pub fn generate(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref();
        std::fs::create_dir_all(dir)?;
        let mut seed = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut seed);
        let key = SigningKey::from_bytes(&seed);
        std::fs::write(dir.join("signing.key"), hex::encode(key.to_bytes()))?;
        let rev_path = dir.join("revoked.json");
        std::fs::write(&rev_path, "[]")?;
        Ok(Self {
            key,
            revoked: HashSet::new(),
            revoked_path: rev_path,
        })
    }

    /// Load an existing keypair from `dir`.
    pub fn load(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref();
        let hex_key = std::fs::read_to_string(dir.join("signing.key"))?;
        let bytes = hex::decode(hex_key.trim()).map_err(|e| GovernorError::Sign(e.to_string()))?;
        let arr: [u8; 32] = bytes
            .try_into()
            .map_err(|_| GovernorError::Sign("invalid key length".into()))?;
        let key = SigningKey::from_bytes(&arr);
        let rev_path = dir.join("revoked.json");
        let revoked_json = std::fs::read_to_string(&rev_path).unwrap_or_else(|_| "[]".into());
        let revoked: HashSet<String> = serde_json::from_str(&revoked_json).unwrap_or_default();
        Ok(Self {
            key,
            revoked,
            revoked_path: rev_path,
        })
    }

    /// Load if exists, generate if not.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref();
        if dir.join("signing.key").exists() {
            Self::load(dir)
        } else {
            Self::generate(dir)
        }
    }

    /// Issue a signed capability token.
    pub fn issue(
        &self,
        principal: &str,
        scope_path: &str,
        permissions: Vec<Permission>,
        ttl_secs: u64,
    ) -> Result<CapabilityToken> {
        let now = now_secs();
        let mut token = CapabilityToken {
            id: Uuid::new_v4().to_string(),
            principal: principal.to_string(),
            scope_path: scope_path.to_string(),
            permissions,
            issued_at: now,
            expires_at: now + ttl_secs,
            signature: String::new(),
        };
        let sig = self.key.sign(&token.body_bytes());
        token.signature = hex::encode(sig.to_bytes());
        Ok(token)
    }

    /// Verify signature, expiry, and revocation status.
    pub fn verify(&self, token: &CapabilityToken) -> Result<()> {
        if self.revoked.contains(&token.id) {
            return Err(GovernorError::Token(format!(
                "token {} is revoked",
                token.id
            )));
        }
        if token.is_expired() {
            return Err(GovernorError::Token(format!(
                "token {} is expired",
                token.id
            )));
        }
        let sig_bytes =
            hex::decode(&token.signature).map_err(|e| GovernorError::Token(e.to_string()))?;
        let sig_arr: [u8; 64] = sig_bytes
            .try_into()
            .map_err(|_| GovernorError::Token("invalid signature length".into()))?;
        let sig = ed25519_dalek::Signature::from_bytes(&sig_arr);
        self.key
            .verifying_key()
            .verify(&token.body_bytes(), &sig)
            .map_err(|e| GovernorError::Token(e.to_string()))
    }

    /// Revoke a token by ID (persisted immediately).
    pub fn revoke(&mut self, id: &str) -> Result<()> {
        self.revoked.insert(id.to_string());
        let json = serde_json::to_string(&self.revoked)?;
        std::fs::write(&self.revoked_path, json)?;
        Ok(())
    }

    pub fn is_revoked(&self, id: &str) -> bool {
        self.revoked.contains(id)
    }

    pub fn public_key_hex(&self) -> String {
        hex::encode(self.key.verifying_key().to_bytes())
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn issue_and_verify() {
        let dir = tempdir().unwrap();
        let auth = TokenAuthority::generate(dir.path()).unwrap();
        let token = auth
            .issue("alice", "/data/", vec![Permission::Read], 3600)
            .unwrap();
        assert!(auth.verify(&token).is_ok());
        assert!(token.covers("/data/foo.txt", &Permission::Read));
        assert!(!token.covers("/other/", &Permission::Read));
    }

    #[test]
    fn revoke_invalidates() {
        let dir = tempdir().unwrap();
        let mut auth = TokenAuthority::generate(dir.path()).unwrap();
        let token = auth
            .issue("bob", "/", vec![Permission::Read], 3600)
            .unwrap();
        auth.revoke(&token.id).unwrap();
        assert!(auth.verify(&token).is_err());
    }

    #[test]
    fn expired_token_rejected() {
        let dir = tempdir().unwrap();
        let auth = TokenAuthority::generate(dir.path()).unwrap();
        let mut token = auth
            .issue("carol", "/", vec![Permission::Write], 1)
            .unwrap();
        // Manually expire it
        token.expires_at = 0;
        assert!(auth.verify(&token).is_err());
    }

    #[test]
    fn tampered_signature_rejected() {
        let dir = tempdir().unwrap();
        let auth = TokenAuthority::generate(dir.path()).unwrap();
        let mut token = auth
            .issue("dave", "/", vec![Permission::Read], 3600)
            .unwrap();
        token.principal = "evil".into();
        assert!(auth.verify(&token).is_err());
    }

    #[test]
    fn roundtrip_encode_decode() {
        let dir = tempdir().unwrap();
        let auth = TokenAuthority::generate(dir.path()).unwrap();
        let token = auth
            .issue("eve", "/docs/", vec![Permission::List], 60)
            .unwrap();
        let json = token.encode().unwrap();
        let decoded = CapabilityToken::decode(&json).unwrap();
        assert!(auth.verify(&decoded).is_ok());
    }
}
