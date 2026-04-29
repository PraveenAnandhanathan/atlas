//! Compliance report generation (T7.4).

use crate::gap::GapAssessment;
use serde::{Deserialize, Serialize};

/// Full compliance posture report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceReport {
    pub generated_at: u64,
    pub store_path: String,
    pub assessment: GapAssessment,
    pub summary: ReportSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportSummary {
    pub controls_checked: usize,
    pub controls_covered: usize,
    pub gaps_total: usize,
    pub gaps_critical: usize,
    pub gaps_high: usize,
    pub coverage_pct: f64,
    pub status: ComplianceStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComplianceStatus {
    /// No critical or high gaps.
    Compliant,
    /// Has high gaps but no critical.
    NeedsAttention,
    /// Has critical gaps.
    NonCompliant,
}

impl ComplianceReport {
    pub fn generate(store_path: impl Into<String>, assessment: GapAssessment) -> Self {
        use crate::gap::GapSeverity;
        let gaps_critical = assessment.gaps.iter().filter(|g| g.severity == GapSeverity::Critical).count();
        let gaps_high = assessment.gaps.iter().filter(|g| g.severity == GapSeverity::High).count();
        let status = if gaps_critical > 0 {
            ComplianceStatus::NonCompliant
        } else if gaps_high > 0 {
            ComplianceStatus::NeedsAttention
        } else {
            ComplianceStatus::Compliant
        };
        let summary = ReportSummary {
            controls_checked: assessment.controls_checked,
            controls_covered: assessment.controls_covered,
            gaps_total: assessment.gaps.len(),
            gaps_critical,
            gaps_high,
            coverage_pct: assessment.coverage_pct(),
            status,
        };
        Self {
            generated_at: now_secs(),
            store_path: store_path.into(),
            assessment,
            summary,
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
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
    use crate::control::catalogue;
    use crate::evidence::collect_automated;
    use crate::gap::assess;

    #[test]
    fn report_serialises_to_json() {
        let controls = catalogue();
        let evidence = collect_automated("/tmp");
        let assessment = assess(&controls, &evidence);
        let report = ComplianceReport::generate("/tmp", assessment);
        let json = report.to_json();
        assert!(json.contains("coverage_pct"));
    }

    #[test]
    fn full_evidence_yields_needs_attention_or_compliant() {
        let controls = catalogue();
        let evidence = collect_automated("/tmp");
        let assessment = assess(&controls, &evidence);
        let report = ComplianceReport::generate("/tmp", assessment);
        assert_ne!(report.summary.status, ComplianceStatus::NonCompliant);
    }
}
