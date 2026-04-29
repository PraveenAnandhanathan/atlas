//! ATLAS SOC 2 / ISO 27001 control readiness framework (T7.4).
//!
//! - [`control`]: control catalogue (SOC 2 trust-service criteria, ISO 27001 Annex A).
//! - [`evidence`]: automated evidence collection.
//! - [`gap`]: gap assessment — identifies controls without fresh evidence.
//! - [`report`]: renders the compliance posture as a JSON report.

pub mod control;
pub mod evidence;
pub mod gap;
pub mod report;

pub use control::{catalogue, Control, Framework, Category};
pub use evidence::{collect_automated, Evidence, EvidenceKind, EvidenceStatus};
pub use gap::{assess, Gap, GapAssessment, GapSeverity};
pub use report::{ComplianceReport, ComplianceStatus, ReportSummary};
