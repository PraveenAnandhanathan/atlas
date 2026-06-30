//! Sled-backed durable session store (P8 enterprise design).
//!
//! Sessions survive process restarts.  The store serialises each
//! [`AuthSession`] as JSON under its token key.  Expired sessions are
//! removed lazily on `get()` and eagerly in `purge_expired()`.
//!
//! # Usage
//!
//! ```ignore
//! use atlas_auth::session_sled::SledSessionStore;
//! let store = SledSessionStore::open("/var/atlas/sessions")?;
//! store.insert(session)?;
//! let s = store.get("tok");
//! ```

use crate::session::AuthSession;
use std::path::Path;
use tracing;

/// Durable session store backed by an embedded sled database.
///
/// This is the recommended backend for production deployments.  Use
/// [`crate::session::SessionStore`] (in-memory) only for tests and
/// single-process dev servers.
pub struct SledSessionStore {
    db: sled::Db,
}

impl SledSessionStore {
    /// Open (or create) a sled session database at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, sled::Error> {
        let db = sled::open(path)?;
        Ok(Self { db })
    }

    /// Persist a session.  Overwrites any existing entry with the same token.
    pub fn insert(&self, session: &AuthSession) -> Result<(), String> {
        let bytes = serde_json::to_vec(session).map_err(|e| e.to_string())?;
        self.db
            .insert(session.token.as_bytes(), bytes)
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Return a live (non-expired) session, or `None` if missing or expired.
    /// Expired entries are removed from the database as a side effect.
    pub fn get(&self, token: &str) -> Option<AuthSession> {
        let bytes = self.db.get(token.as_bytes()).ok()??;
        let session: AuthSession = serde_json::from_slice(&bytes).ok()?;
        if session.is_expired() {
            if let Err(e) = self.db.remove(token.as_bytes()) {
                tracing::warn!(error = %e, "session_sled: failed to remove expired session");
            }
            return None;
        }
        Some(session)
    }

    /// Remove a session.  Returns `true` if the token existed.
    pub fn revoke(&self, token: &str) -> bool {
        self.db
            .remove(token.as_bytes())
            .map(|v| v.is_some())
            .unwrap_or(false)
    }

    /// Remove all expired sessions from the database.  Returns the count removed.
    pub fn purge_expired(&self) -> usize {
        let mut removed = 0usize;
        for item in self.db.iter().values().filter_map(|r| r.ok()) {
            if let Ok(s) = serde_json::from_slice::<AuthSession>(&item) {
                if s.is_expired() {
                    let _ = self.db.remove(s.token.as_bytes());
                    removed += 1;
                }
            }
        }
        removed
    }

    /// Number of active (non-expired) sessions in the store.
    pub fn active_count(&self) -> usize {
        self.db
            .iter()
            .values()
            .filter_map(|r| r.ok())
            .filter_map(|b| serde_json::from_slice::<AuthSession>(&b).ok())
            .filter(|s| !s.is_expired())
            .count()
    }

    /// Flush all pending writes to disk (useful before shutdown).
    pub fn flush(&self) -> Result<(), sled::Error> {
        self.db.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{AuthMethod, AuthSession};

    fn session(token: &str, ttl_ms: u64) -> AuthSession {
        AuthSession::new(token, "alice", vec![], ttl_ms, AuthMethod::Oidc)
    }

    #[test]
    fn insert_get_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = SledSessionStore::open(dir.path()).unwrap();
        store.insert(&session("tok1", 60_000)).unwrap();
        assert_eq!(store.get("tok1").unwrap().principal, "alice");
    }

    #[test]
    fn expired_session_not_returned() {
        let dir = tempfile::tempdir().unwrap();
        let store = SledSessionStore::open(dir.path()).unwrap();
        store.insert(&session("tok2", 0)).unwrap();
        assert!(store.get("tok2").is_none());
    }

    #[test]
    fn revoke_removes_session() {
        let dir = tempfile::tempdir().unwrap();
        let store = SledSessionStore::open(dir.path()).unwrap();
        store.insert(&session("tok3", 60_000)).unwrap();
        assert!(store.revoke("tok3"));
        assert!(store.get("tok3").is_none());
    }

    #[test]
    fn sessions_survive_restart() {
        let dir = tempfile::tempdir().unwrap();
        {
            let store = SledSessionStore::open(dir.path()).unwrap();
            store.insert(&session("persistent", 3_600_000)).unwrap();
            store.flush().unwrap();
        }
        // Re-open from disk.
        let store2 = SledSessionStore::open(dir.path()).unwrap();
        assert!(
            store2.get("persistent").is_some(),
            "session must survive store restart"
        );
    }

    #[test]
    fn purge_expired_cleans_stale() {
        let dir = tempfile::tempdir().unwrap();
        let store = SledSessionStore::open(dir.path()).unwrap();
        store.insert(&session("live", 3_600_000)).unwrap();
        store.insert(&session("dead", 0)).unwrap();
        let removed = store.purge_expired();
        assert_eq!(removed, 1);
        assert_eq!(store.active_count(), 1);
    }
}
