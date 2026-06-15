//! Evidence collection for compliance controls (T7.4).

use serde::{Deserialize, Serialize};

/// Status of a single evidence item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceStatus {
    Collected,
    Stale,
    Missing,
}

/// A piece of evidence tied to a control.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub control_id: String,
    pub kind: EvidenceKind,
    pub description: String,
    pub path: Option<String>,
    pub status: EvidenceStatus,
    /// Unix timestamp (s) when the evidence was last refreshed.
    pub collected_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceKind {
    AuditLog,
    ConfigSnapshot,
    TestResult,
    PolicyDocument,
    ScanReport,
    BackupVerification,
}

impl Evidence {
    pub fn collect(control_id: impl Into<String>, kind: EvidenceKind, description: impl Into<String>) -> Self {
        Self {
            control_id: control_id.into(),
            kind,
            description: description.into(),
            path: None,
            status: EvidenceStatus::Collected,
            collected_at: now_secs(),
        }
    }

    pub fn is_fresh(&self, max_age_secs: u64) -> bool {
        now_secs().saturating_sub(self.collected_at) <= max_age_secs
    }
}

/// Collect automated evidence from the ATLAS system.
///
/// Each record with a file `path` is probed on disk.  The status is
/// `Collected` only when the file actually exists; otherwise `Missing`.
/// Records with no path (manual/test evidence) remain `Collected`.
pub fn collect_automated(store_path: &str) -> Vec<Evidence> {
    let ts = now_secs();
    let candidates: Vec<(_, _, _, Option<String>)> = vec![
        ("CC6.1", EvidenceKind::AuditLog,
         "Capability-token validation log export",
         Some(format!("{store_path}/audit/access.log"))),
        ("CC6.3", EvidenceKind::ConfigSnapshot,
         "Atlas governor policy snapshot",
         Some(format!("{store_path}/config/policy.json"))),
        ("A1.3",  EvidenceKind::BackupVerification,
         "BLAKE3 footer verification result from last snapshot",
         Some(format!("{store_path}/backup/verify.json"))),
        ("C1.1",  EvidenceKind::ConfigSnapshot,
         "Encryption-at-rest configuration",
         Some(format!("{store_path}/config/encryption.json"))),
        ("A.9.2", EvidenceKind::TestResult,
         "SCIM provisioning round-trip test results",
         None),
        ("A.12.3", EvidenceKind::BackupVerification,
         "Monthly full-restore test log",
         Some(format!("{store_path}/backup/restore-test.log"))),
    ];

    candidates.into_iter().map(|(control_id, kind, description, path)| {
        let status = match &path {
            Some(p) => {
                if std::path::Path::new(p).exists() {
                    EvidenceStatus::Collected
                } else {
                    EvidenceStatus::Missing
                }
            }
            None => EvidenceStatus::Collected,
        };
        Evidence {
            control_id: control_id.into(),
            kind,
            description: description.into(),
            path,
            status,
            collected_at: ts,
        }
    }).collect()
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_returns_evidence() {
        let e = Evidence::collect("CC6.1", EvidenceKind::AuditLog, "test");
        assert_eq!(e.status, EvidenceStatus::Collected);
        assert!(e.is_fresh(3600));
    }

    #[test]
    fn automated_collection_covers_key_controls() {
        let evs = collect_automated("/tmp/store");
        let ids: Vec<&str> = evs.iter().map(|e| e.control_id.as_str()).collect();
        assert!(ids.contains(&"CC6.1"));
        assert!(ids.contains(&"A1.3"));
    }

    /// Evidence with a path that does not exist on disk must be Missing,
    /// not Collected.  This prevents fabricating a green compliance report.
    #[test]
    fn missing_file_yields_missing_status() {
        let evs = collect_automated("/nonexistent_atlas_store_path_xyz");
        for ev in &evs {
            if ev.path.is_some() {
                assert_eq!(
                    ev.status,
                    EvidenceStatus::Missing,
                    "control {} has a path that doesn't exist but status is {:?}",
                    ev.control_id,
                    ev.status
                );
            }
        }
    }

    /// A record without a path (manual/test evidence) remains Collected
    /// regardless of file system state.
    #[test]
    fn no_path_evidence_stays_collected() {
        let evs = collect_automated("/nonexistent_path");
        let no_path: Vec<_> = evs.iter().filter(|e| e.path.is_none()).collect();
        assert!(!no_path.is_empty());
        for ev in no_path {
            assert_eq!(ev.status, EvidenceStatus::Collected);
        }
    }
}
