//! SHA-256 chained tamper-evident audit log (T4.8).
//!
//! Every entry records the SHA-256 hash of the previous entry, forming a
//! chain.  Any post-hoc modification to any entry breaks all subsequent
//! hashes, which `verify_chain` detects.

use crate::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

/// One immutable audit record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    /// Monotonically increasing (per log).
    pub seq: u64,
    /// Milliseconds since Unix epoch.
    pub ts: u64,
    /// Category string, e.g. `"policy.eval"`, `"token.issue"`, `"file.read"`.
    pub event_type: String,
    /// Object being acted on.
    pub subject: String,
    /// Principal or service that triggered the event.
    pub actor: String,
    /// Structured detail fields.
    pub detail: HashMap<String, String>,
    /// SHA-256 hex of this entry's canonical form (excludes this field).
    pub hash: String,
    /// `hash` of the previous entry, or `"genesis"` for the first entry.
    pub prev_hash: String,
}

pub struct AuditLog {
    path: PathBuf,
    seq: u64,
    prev_hash: String,
}

impl AuditLog {
    pub fn open(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref();
        std::fs::create_dir_all(dir)?;
        let path = dir.join("audit.jsonl");
        let (seq, prev_hash) = if path.exists() {
            let f = std::fs::File::open(&path)?;
            let last = BufReader::new(f)
                .lines()
                .map_while(|l| l.ok())
                .filter(|l| !l.trim().is_empty())
                .filter_map(|l| serde_json::from_str::<AuditEntry>(&l).ok())
                .last();
            match last {
                Some(e) => (e.seq + 1, e.hash),
                None => (0, "genesis".into()),
            }
        } else {
            (0, "genesis".into())
        };
        Ok(Self {
            path,
            seq,
            prev_hash,
        })
    }

    /// Append one event to the log.
    pub fn append(
        &mut self,
        event_type: &str,
        subject: &str,
        actor: &str,
        detail: HashMap<String, String>,
    ) -> Result<AuditEntry> {
        let mut entry = AuditEntry {
            seq: self.seq,
            ts: now_millis(),
            event_type: event_type.to_string(),
            subject: subject.to_string(),
            actor: actor.to_string(),
            detail,
            hash: String::new(),
            prev_hash: self.prev_hash.clone(),
        };
        entry.hash = compute_hash(&entry);
        let line = serde_json::to_string(&entry)?;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(f, "{line}")?;
        self.prev_hash = entry.hash.clone();
        self.seq += 1;
        Ok(entry)
    }

    /// Replay the entire log and verify every link in the chain.
    /// Returns `Ok(true)` if intact, `Ok(false)` if tampered.
    pub fn verify_chain(&self) -> Result<bool> {
        if !self.path.exists() {
            return Ok(true);
        }
        let f = std::fs::File::open(&self.path)?;
        let entries: Vec<AuditEntry> = BufReader::new(f)
            .lines()
            .map_while(|l| l.ok())
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str(&l).ok())
            .collect();

        let mut prev_hash = "genesis".to_string();
        for entry in &entries {
            if entry.prev_hash != prev_hash {
                return Ok(false);
            }
            if entry.hash != compute_hash(entry) {
                return Ok(false);
            }
            prev_hash = entry.hash.clone();
        }
        Ok(true)
    }

    /// Export entries with seq in [from_seq, to_seq] (inclusive).
    pub fn export_range(&self, from_seq: u64, to_seq: u64) -> Result<Vec<AuditEntry>> {
        if !self.path.exists() {
            return Ok(vec![]);
        }
        let f = std::fs::File::open(&self.path)?;
        Ok(BufReader::new(f)
            .lines()
            .map_while(|l| l.ok())
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str::<AuditEntry>(&l).ok())
            .filter(|e| e.seq >= from_seq && e.seq <= to_seq)
            .collect())
    }
}

fn compute_hash(entry: &AuditEntry) -> String {
    // Canonical form: alphabetically-sorted fields, `hash` excluded.
    #[derive(Serialize)]
    struct Canonical<'a> {
        actor: &'a str,
        detail: &'a HashMap<String, String>,
        event_type: &'a str,
        prev_hash: &'a str,
        seq: u64,
        subject: &'a str,
        ts: u64,
    }
    let bytes = serde_json::to_vec(&Canonical {
        actor: &entry.actor,
        detail: &entry.detail,
        event_type: &entry.event_type,
        prev_hash: &entry.prev_hash,
        seq: entry.seq,
        subject: &entry.subject,
        ts: entry.ts,
    })
    .unwrap_or_default();
    let mut h = Sha256::new();
    h.update(&bytes);
    hex::encode(h.finalize())
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn chain_verifies_after_append() {
        let dir = tempdir().unwrap();
        let mut log = AuditLog::open(dir.path()).unwrap();
        for i in 0..5 {
            log.append(
                "test.event",
                &format!("/path/{i}"),
                "tester",
                HashMap::new(),
            )
            .unwrap();
        }
        assert!(log.verify_chain().unwrap());
    }

    #[test]
    fn export_range() {
        let dir = tempdir().unwrap();
        let mut log = AuditLog::open(dir.path()).unwrap();
        for i in 0..5 {
            log.append("ev", &format!("/{i}"), "actor", HashMap::new())
                .unwrap();
        }
        let slice = log.export_range(1, 3).unwrap();
        assert_eq!(slice.len(), 3);
        assert_eq!(slice[0].seq, 1);
    }

    #[test]
    fn tampered_entry_detected() {
        let dir = tempdir().unwrap();
        let mut log = AuditLog::open(dir.path()).unwrap();
        log.append("ev", "/a", "actor", HashMap::new()).unwrap();
        log.append("ev", "/b", "actor", HashMap::new()).unwrap();

        // Read the raw JSONL, corrupt the first entry's subject, write back.
        let content = std::fs::read_to_string(log.path.clone()).unwrap();
        let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
        let mut entry: AuditEntry = serde_json::from_str(&lines[0]).unwrap();
        entry.subject = "/tampered".into();
        lines[0] = serde_json::to_string(&entry).unwrap();
        std::fs::write(&log.path, lines.join("\n") + "\n").unwrap();

        assert!(!log.verify_chain().unwrap());
    }
}
