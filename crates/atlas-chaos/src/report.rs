//! Chaos run report types (T7.1).

use crate::ChaosScenario;
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime};

/// A fault activation/deactivation event recorded during a run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FaultEvent {
    pub fault_index: usize,
    pub activated_at_ms: u64,
    pub deactivated_at_ms: Option<u64>,
}

/// A recorded invariant violation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvariantViolation {
    pub invariant_index: usize,
    pub description: String,
    pub detected_at_ms: u64,
}

/// Overall outcome of a chaos run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Pass,
    Fail,
    Timeout,
    Error(String),
}

/// Full report produced by [`super::ChaosRunner`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChaosReport {
    pub scenario_name: String,
    pub outcome: Outcome,
    pub started_at_ms: u64,
    pub elapsed_ms: u64,
    pub fault_events: Vec<FaultEvent>,
    pub violations: Vec<InvariantViolation>,
    pub ops_completed: u64,
    pub bytes_written: u64,
    pub bytes_read: u64,
    pub errors_observed: u64,
}

impl ChaosReport {
    pub fn new(scenario: &ChaosScenario) -> Self {
        Self {
            scenario_name: scenario.name.clone(),
            outcome: Outcome::Pass,
            started_at_ms: now_ms(),
            elapsed_ms: 0,
            fault_events: Vec::new(),
            violations: Vec::new(),
            ops_completed: 0,
            bytes_written: 0,
            bytes_read: 0,
            errors_observed: 0,
        }
    }

    pub fn add_violation(&mut self, idx: usize, description: impl Into<String>) {
        self.violations.push(InvariantViolation {
            invariant_index: idx,
            description: description.into(),
            detected_at_ms: now_ms(),
        });
        self.outcome = Outcome::Fail;
    }

    pub fn finalise(&mut self, started_ms: u64) {
        self.elapsed_ms = now_ms().saturating_sub(started_ms);
        if self.outcome == Outcome::Pass && self.violations.is_empty() {
            self.outcome = Outcome::Pass;
        }
    }

    pub fn passed(&self) -> bool {
        self.outcome == Outcome::Pass
    }

    pub fn summary(&self) -> String {
        format!(
            "[{}] {} — {} violation(s), {} ops, {:.1} MB written, {:.1} MB read, {} errors, {}ms",
            if self.passed() { "PASS" } else { "FAIL" },
            self.scenario_name,
            self.violations.len(),
            self.ops_completed,
            self.bytes_written as f64 / (1024.0 * 1024.0),
            self.bytes_read as f64 / (1024.0 * 1024.0),
            self.errors_observed,
            self.elapsed_ms,
        )
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
    fn new_report_is_pass() {
        let s = ChaosScenario::single_node_crash();
        let r = ChaosReport::new(&s);
        assert_eq!(r.outcome, Outcome::Pass);
        assert!(r.violations.is_empty());
    }

    #[test]
    fn add_violation_flips_to_fail() {
        let s = ChaosScenario::single_node_crash();
        let mut r = ChaosReport::new(&s);
        r.add_violation(0, "data mismatch");
        assert_eq!(r.outcome, Outcome::Fail);
    }

    #[test]
    fn summary_contains_scenario_name() {
        let s = ChaosScenario::single_node_crash();
        let r = ChaosReport::new(&s);
        assert!(r.summary().contains("single_node_crash"));
    }
}
