//! ATLAS onboarding, installer wizards, and sample-data seeding (T6.7).
//!
//! Covers the three parts of task T6.7:
//!
//! 1. **Platform installer** ([`installer`]) — generates OS-specific
//!    installer packages (`.msi` on Windows, `.pkg`/`.dmg` on macOS,
//!    `.deb`/`.rpm`/AppImage on Linux) and provides the install/uninstall
//!    logic called by the GUI wizard.
//!
//! 2. **First-mount wizard** ([`wizard`]) — a step-by-step TUI / embedded
//!    GUI flow that walks a new user through creating their first ATLAS
//!    store, choosing Solo vs Team mode, enabling the semantic plane, and
//!    setting an initial policy.
//!
//! 3. **Sample data** ([`sample_data`]) — seeds a fresh store with a
//!    small curated dataset (a miniature model checkpoint, a parquet
//!    table, and a README) so the user has something to browse on first
//!    launch.

pub mod installer;
pub mod sample_data;
pub mod wizard;

pub use installer::{InstallConfig, InstallError, Platform};
pub use sample_data::seed_sample_data;
pub use wizard::{OnboardingState, WizardStep};
