//! Short-lived session tokens issued after successful OIDC/SAML login (T7.3).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A session issued to a successfully authenticated user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthSession {
    /// Opaque session token (256-bit random hex in production).
    pub token: String,
    /// The ATLAS principal name derived from the IdP identity.
    pub principal: String,
    /// Groups the principal belongs to.
    pub groups: Vec<String>,
    /// Unix timestamp (ms) when the session expires.
    pub expires_at_ms: u64,
    /// Authentication method used to create this session.
    pub auth_method: AuthMethod,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AuthMethod {
    Oidc,
    Saml,
    ApiKey,
}

impl AuthSession {
    pub fn new(token: impl Into<String>, principal: impl Into<String>, groups: Vec<String>, ttl_ms: u64, method: AuthMethod) -> Self {
        Self {
            token: token.into(),
            principal: principal.into(),
            groups,
            expires_at_ms: now_ms() + ttl_ms,
            auth_method: method,
        }
    }

    pub fn is_expired(&self) -> bool {
        now_ms() >= self.expires_at_ms
    }
}

/// Thread-safe in-memory session store.
#[derive(Default, Clone)]
pub struct SessionStore {
    sessions: Arc<Mutex<HashMap<String, AuthSession>>>,
}

impl SessionStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, session: AuthSession) -> Result<(), String> {
        let mut store = self.sessions.lock().map_err(|e| e.to_string())?;
        store.insert(session.token.clone(), session);
        Ok(())
    }

    pub fn get(&self, token: &str) -> Option<AuthSession> {
        let store = self.sessions.lock().ok()?;
        let s = store.get(token)?;
        if s.is_expired() { None } else { Some(s.clone()) }
    }

    pub fn revoke(&self, token: &str) -> bool {
        self.sessions.lock().ok().map(|mut s| s.remove(token).is_some()).unwrap_or(false)
    }

    /// Remove all expired sessions.
    pub fn purge_expired(&self) -> usize {
        let mut store = match self.sessions.lock() {
            Ok(s) => s,
            Err(_) => return 0,
        };
        let before = store.len();
        store.retain(|_, s| !s.is_expired());
        before - store.len()
    }

    pub fn active_count(&self) -> usize {
        self.sessions.lock().map(|s| s.values().filter(|sess| !sess.is_expired()).count()).unwrap_or(0)
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(token: &str, ttl_ms: u64) -> AuthSession {
        AuthSession::new(token, "alice", vec!["admins".into()], ttl_ms, AuthMethod::Oidc)
    }

    #[test]
    fn insert_and_get() {
        let store = SessionStore::new();
        store.insert(session("tok1", 60_000)).unwrap();
        let s = store.get("tok1").unwrap();
        assert_eq!(s.principal, "alice");
        assert!(!s.is_expired());
    }

    #[test]
    fn expired_session_not_returned() {
        let store = SessionStore::new();
        store.insert(session("tok2", 0)).unwrap();
        assert!(store.get("tok2").is_none());
    }

    #[test]
    fn revoke_removes_session() {
        let store = SessionStore::new();
        store.insert(session("tok3", 60_000)).unwrap();
        assert!(store.revoke("tok3"));
        assert!(store.get("tok3").is_none());
    }

    #[test]
    fn purge_expired_removes_stale() {
        let store = SessionStore::new();
        store.insert(session("live", 60_000)).unwrap();
        store.insert(session("dead", 0)).unwrap();
        let removed = store.purge_expired();
        assert_eq!(removed, 1);
        assert_eq!(store.active_count(), 1);
    }
}
