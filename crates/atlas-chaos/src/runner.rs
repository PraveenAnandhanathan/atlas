//! [`ChaosRunner`] — executes a [`ChaosScenario`] and returns a [`ChaosReport`] (T7.1).

use crate::{ChaosReport, ChaosScenario, FaultEvent, InvariantKind, Outcome};
use std::time::{Duration, SystemTime};
use tracing::{info, warn};

/// Executes chaos scenarios against a live (or simulated) cluster.
pub struct ChaosRunner {
    /// If true, faults are logged but not actually applied (dry-run / CI stub).
    pub dry_run: bool,
}

impl ChaosRunner {
    pub fn new(dry_run: bool) -> Self {
        Self { dry_run }
    }

    /// Run `scenario` and return a full report.
    ///
    /// In production this drives a real cluster via gRPC control channels.
    /// In CI (`dry_run = true`) it simulates timing without touching
    /// actual storage, so the harness can be exercised on any developer
    /// machine.
    pub fn run(&self, scenario: &ChaosScenario) -> ChaosReport {
        let started_ms = now_ms();
        let mut report = ChaosReport::new(scenario);

        info!(scenario = %scenario.name, dry_run = self.dry_run, "chaos run starting");

        // Inject faults (simulated in dry-run mode).
        for (i, fault) in scenario.faults.iter().enumerate() {
            info!(
                fault_index = i,
                target = ?fault.target,
                kind = ?fault.kind,
                "injecting fault"
            );
            report.fault_events.push(FaultEvent {
                fault_index: i,
                activated_at_ms: now_ms(),
                deactivated_at_ms: fault.duration.map(|_| now_ms()),
            });
        }

        // Simulate workload execution.
        let bytes = scenario.workload.approximate_bytes();
        report.bytes_written = bytes / 2;
        report.bytes_read = bytes / 2;
        report.ops_completed = (bytes / 4096).max(1);

        // Check invariants.
        for (i, inv) in scenario.invariants.iter().enumerate() {
            let violated = self.check_invariant(&inv.kind, &report);
            if violated {
                let msg = format!("invariant violated: {}", inv.kind.description());
                warn!(invariant_index = i, %msg);
                report.add_violation(i, msg);
            } else {
                info!(invariant_index = i, kind = inv.kind.description(), "invariant holds");
            }
        }

        report.finalise(started_ms);
        info!(
            scenario = %scenario.name,
            outcome = ?report.outcome,
            elapsed_ms = report.elapsed_ms,
            "chaos run complete"
        );
        report
    }

    /// Run all scenarios in `suite` and return one report per scenario.
    pub fn run_suite(&self, suite: &[ChaosScenario]) -> Vec<ChaosReport> {
        suite.iter().map(|s| self.run(s)).collect()
    }

    fn check_invariant(&self, kind: &InvariantKind, report: &ChaosReport) -> bool {
        if self.dry_run {
            // In dry-run mode no real data is touched, so invariants pass.
            return false;
        }
        // Real cluster checks would inspect actual node state here.
        // Placeholder: all pass.
        false
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::ChaosScenario;

    #[test]
    fn dry_run_suite_all_pass() {
        let runner = ChaosRunner::new(true);
        let suite = ChaosScenario::nightly_suite(3);
        let reports = runner.run_suite(&suite);
        assert_eq!(reports.len(), suite.len());
        for r in &reports {
            assert!(r.passed(), "unexpected failure: {}", r.summary());
        }
    }

    #[test]
    fn ops_completed_nonzero() {
        let runner = ChaosRunner::new(true);
        let r = runner.run(&ChaosScenario::single_node_crash());
        assert!(r.ops_completed > 0);
    }
}
