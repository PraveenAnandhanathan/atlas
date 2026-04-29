//! ATLAS fault-injection framework (T7.1).
//!
//! Provides a composable, reproducible chaos harness used by nightly CI
//! runs and on-demand game days.  Every fault type is a value; runners
//! pick a scenario, inject faults, drive workload, and assert invariants.
//!
//! # Design
//!
//! ```text
//! ChaosScenario
//!   └── Vec<Fault>          (what to inject)
//!   └── Workload            (what to run while faults are active)
//!   └── Vec<Invariant>      (what must stay true throughout)
//!
//! ChaosRunner::run(&scenario) → ChaosReport
//! ```
//!
//! Faults are injected via thin shim traits so the real storage /
//! metadata / network implementations can be swapped for instrumented
//! test doubles without modifying production code.

pub mod fault;
pub mod invariant;
pub mod report;
pub mod runner;
pub mod scenario;
pub mod workload;

pub use fault::{Fault, FaultKind, FaultTarget};
pub use invariant::{Invariant, InvariantKind};
pub use report::{ChaosReport, FaultEvent, InvariantViolation, Outcome};
pub use runner::ChaosRunner;
pub use scenario::ChaosScenario;
pub use workload::{Workload, WorkloadKind};
