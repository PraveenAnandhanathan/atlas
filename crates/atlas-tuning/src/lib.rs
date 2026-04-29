//! ATLAS per-workload performance tuning profiles (T7.6).
//!
//! - [`profile`]: built-in profiles for Training, Inference, Build, Interactive, Streaming.
//! - [`tuner`]: runtime state that maps volume/namespace names to active profiles.

pub mod profile;
pub mod tuner;

pub use profile::{CachePolicy, TuningProfile, WorkloadKind};
pub use tuner::{recommend, TunerState};
