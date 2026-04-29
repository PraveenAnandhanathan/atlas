//! Gap assessment: identify controls missing evidence (T7.4).

use crate::control::{Control, Framework};
use crate::evidence::{Evidence, EvidenceStatus};
use serde::{Deserialize, Serialize};

/// Severity of a compliance gap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum GapSeverity {
    Low,
    Medium,
    High,
    Critical,
}

/// A single gap found during assessment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Gap {
    pub control_id: String,
    pub framework: Framework,
    pub title: String,
    pub severity: GapSeverity,
    pub reason: String,
}

/// Results of a gap assessment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GapAssessment {
    pub gaps: Vec<Gap>,
    pub controls_checked: usize,
    pub controls_covered: usize,
}

impl GapAssessment {
    pub fn coverage_pct(&self) -> f64 {
        if self.controls_checked == 0 { return 0.0; }
        100.0 * self.controls_covered as f64 / self.controls_checked as f64
    }

    pub fn critical_gaps(&self) -> Vec<&Gap> {
        self.gaps.iter().filter(|g| g.severity == GapSeverity::Critical).collect()
    }
}

/// Run a gap assessment against a set of controls and collected evidence.
pub fn assess(controls: &[Control], evidence: &[Evidence]) -> GapAssessment {
    let mut gaps = Vec::new();
    let mut covered = 0usize;

    for ctrl in controls {
        let ev = evidence.iter().find(|e| e.control_id == ctrl.id);
        match ev {
            Some(e) if e.status == EvidenceStatus::Collected && e.is_fresh(86_400 * 90) => {
                covered += 1;
            }
            Some(e) if e.status == EvidenceStatus::Stale => {
                gaps.push(Gap {
                    control_id: ctrl.id.clone(),
                    framework: ctrl.framework,
                    title: ctrl.title.clone(),
                    severity: if ctrl.automated { GapSeverity::Medium } else { GapSeverity::Low },
                    reason: "Evidence is stale (>90 days)".into(),
                });
            }
            _ => {
                let sev = if ctrl.automated { GapSeverity::High } else { GapSeverity::Medium };
                gaps.push(Gap {
                    control_id: ctrl.id.clone(),
                    framework: ctrl.framework,
                    title: ctrl.title.clone(),
                    severity: sev,
                    reason: "No evidence collected".into(),
                });
            }
        }
    }

    GapAssessment { gaps, controls_checked: controls.len(), controls_covered: covered }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::{catalogue, Category};
    use crate::evidence::{Evidence, EvidenceKind, collect_automated};

    #[test]
    fn coverage_increases_with_evidence() {
        let controls = catalogue();
        let no_ev = assess(&controls, &[]);
        let with_ev = assess(&controls, &collect_automated("/tmp"));
        assert!(with_ev.controls_covered >= no_ev.controls_covered);
    }

    #[test]
    fn missing_automated_control_is_high_severity() {
        let ctrl = Control::new("TEST-1", Framework::Soc2, Category::Security, "T", "D", true);
        let result = assess(&[ctrl], &[]);
        assert_eq!(result.gaps[0].severity, GapSeverity::High);
    }

    #[test]
    fn coverage_pct_range() {
        let controls = catalogue();
        let result = assess(&controls, &collect_automated("/tmp"));
        let pct = result.coverage_pct();
        assert!((0.0..=100.0).contains(&pct));
    }
}
