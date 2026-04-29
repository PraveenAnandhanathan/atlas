//! SOC 2 / ISO 27001 control registry (T7.4).

use serde::{Deserialize, Serialize};

/// Compliance framework.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Framework {
    Soc2,
    Iso27001,
    Gdpr,
    Hipaa,
}

impl Framework {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Soc2    => "SOC 2",
            Self::Iso27001 => "ISO 27001",
            Self::Gdpr    => "GDPR",
            Self::Hipaa   => "HIPAA",
        }
    }
}

/// Trust-service category (SOC 2) or control domain (ISO 27001).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Category {
    Security,
    Availability,
    ProcessingIntegrity,
    Confidentiality,
    Privacy,
    AccessControl,
    Cryptography,
    IncidentManagement,
    BackupAndRecovery,
    ChangeManagement,
}

/// A single compliance control.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Control {
    pub id: String,
    pub framework: Framework,
    pub category: Category,
    pub title: String,
    pub description: String,
    pub automated: bool,
}

impl Control {
    pub fn new(id: impl Into<String>, framework: Framework, category: Category, title: impl Into<String>, desc: impl Into<String>, automated: bool) -> Self {
        Self { id: id.into(), framework, category, title: title.into(), description: desc.into(), automated }
    }
}

/// Build the full ATLAS control catalogue.
pub fn catalogue() -> Vec<Control> {
    vec![
        // SOC 2 — Security
        Control::new("CC6.1", Framework::Soc2, Category::Security,
            "Logical Access Controls",
            "Restrict logical access to system components based on least privilege.", true),
        Control::new("CC6.2", Framework::Soc2, Category::Security,
            "User Authentication",
            "Require multi-factor authentication for privileged operations.", true),
        Control::new("CC6.3", Framework::Soc2, Category::Security,
            "Authorisation Enforcement",
            "Enforce capability-token authorisation on every data-plane request.", true),
        Control::new("CC7.1", Framework::Soc2, Category::Security,
            "Vulnerability Detection",
            "Detect vulnerabilities through automated scanning and CVE alerting.", false),
        Control::new("CC7.2", Framework::Soc2, Category::IncidentManagement,
            "Incident Response",
            "Maintain an incident-response plan and conduct annual tabletop exercises.", false),
        // SOC 2 — Availability
        Control::new("A1.1", Framework::Soc2, Category::Availability,
            "RPO / RTO Commitments",
            "Maintain RPO ≤ 1 h and RTO ≤ 4 h for Tier-1 volumes.", true),
        Control::new("A1.2", Framework::Soc2, Category::Availability,
            "Redundant Infrastructure",
            "Deploy storage nodes across at least three availability zones.", true),
        Control::new("A1.3", Framework::Soc2, Category::BackupAndRecovery,
            "Backup Integrity",
            "Verify BLAKE3 footer of every snapshot bundle before offsite transfer.", true),
        // SOC 2 — Confidentiality
        Control::new("C1.1", Framework::Soc2, Category::Confidentiality,
            "Data Encryption at Rest",
            "Encrypt all object chunks with AES-256-GCM.", true),
        Control::new("C1.2", Framework::Soc2, Category::Cryptography,
            "Data Encryption in Transit",
            "All inter-node traffic uses mutually-authenticated TLS 1.3.", true),
        // SOC 2 — Privacy
        Control::new("P1.1", Framework::Soc2, Category::Privacy,
            "Data Classification",
            "Tag objects with sensitivity labels and enforce data-handling policies.", true),
        // ISO 27001
        Control::new("A.9.1", Framework::Iso27001, Category::AccessControl,
            "Access Control Policy",
            "Establish, document, and review an access-control policy.", false),
        Control::new("A.9.2", Framework::Iso27001, Category::AccessControl,
            "User Access Management",
            "Provision and de-provision accounts via SCIM within one business day.", true),
        Control::new("A.12.1", Framework::Iso27001, Category::ChangeManagement,
            "Change Management",
            "Review and test all infrastructure changes in staging before production.", false),
        Control::new("A.12.3", Framework::Iso27001, Category::BackupAndRecovery,
            "Information Backup",
            "Perform daily incremental backups with monthly full-restore tests.", true),
        Control::new("A.16.1", Framework::Iso27001, Category::IncidentManagement,
            "Incident Management",
            "Classify, escalate, and post-mortem all Sev-1 incidents within 5 days.", false),
        Control::new("A.18.1", Framework::Iso27001, Category::Privacy,
            "Regulatory Compliance",
            "Map all personal data flows; maintain records of processing activities.", false),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_not_empty() {
        assert!(!catalogue().is_empty());
    }

    #[test]
    fn automated_controls_exist() {
        assert!(catalogue().iter().any(|c| c.automated));
    }

    #[test]
    fn both_frameworks_covered() {
        let cat = catalogue();
        assert!(cat.iter().any(|c| c.framework == Framework::Soc2));
        assert!(cat.iter().any(|c| c.framework == Framework::Iso27001));
    }
}
