//! First-mount onboarding wizard (T6.7).
//!
//! The wizard is driven as a state machine: the GUI or TUI calls
//! `next()` / `back()` and renders whatever `current_step()` returns.

use crate::installer::{InstallConfig, Platform};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Each step shown to the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WizardStep {
    /// Welcome screen with high-level explanation.
    Welcome,
    /// Choose Solo (single-node) vs Team (cluster) mode.
    ChooseMode,
    /// Pick the store location and data directory.
    ChooseLocation,
    /// Enable / configure the semantic plane (embedder, index).
    SemanticPlane,
    /// Review policy defaults — read-only for guests, read-write for owner.
    PolicyDefaults,
    /// Run installation and seed sample data.
    Installing,
    /// All done — show the "Open ATLAS Explorer" button.
    Done,
}

impl WizardStep {
    pub fn title(&self) -> &'static str {
        match self {
            Self::Welcome       => "Welcome to ATLAS",
            Self::ChooseMode    => "Deployment Mode",
            Self::ChooseLocation => "Store Location",
            Self::SemanticPlane => "Semantic Search",
            Self::PolicyDefaults => "Default Policy",
            Self::Installing    => "Installing…",
            Self::Done          => "You're all set!",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Self::Welcome =>
                "ATLAS is a content-addressed filesystem with Git-like versioning, \
                 semantic search, lineage tracking, and a policy engine.",
            Self::ChooseMode =>
                "Solo mode is a single-node store — great for personal use. \
                 Team mode spreads data across 3+ nodes for collaborative workloads.",
            Self::ChooseLocation =>
                "Choose where to store ATLAS data. SSD storage is recommended \
                 for metadata; large spinning disks work fine for bulk chunks.",
            Self::SemanticPlane =>
                "Enable semantic search to query files by meaning. \
                 Requires GPU or CPU embedder (downloaded separately).",
            Self::PolicyDefaults =>
                "Set who can read and write your volume by default. \
                 You can refine policies at any subtree level later.",
            Self::Installing =>
                "Initialising the store, writing config, and seeding sample data…",
            Self::Done =>
                "Your ATLAS volume is ready. Open ATLAS Explorer to browse, \
                 search, and manage your data.",
        }
    }

    fn all() -> &'static [WizardStep] {
        &[
            Self::Welcome,
            Self::ChooseMode,
            Self::ChooseLocation,
            Self::SemanticPlane,
            Self::PolicyDefaults,
            Self::Installing,
            Self::Done,
        ]
    }

    fn index(&self) -> usize {
        Self::all().iter().position(|s| s == self).unwrap_or(0)
    }
}

/// Deployment mode chosen by the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeployMode {
    Solo,
    Team,
}

/// Accumulated wizard state — mutated as the user progresses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnboardingState {
    pub step: WizardStep,
    pub mode: DeployMode,
    pub store_path: PathBuf,
    pub semantic_enabled: bool,
    pub shell_integration: bool,
    pub install_gui: bool,
    pub error: Option<String>,
}

impl Default for OnboardingState {
    fn default() -> Self {
        let default_store = dirs_or_home().join("atlas-store");
        Self {
            step: WizardStep::Welcome,
            mode: DeployMode::Solo,
            store_path: default_store,
            semantic_enabled: false,
            shell_integration: true,
            install_gui: true,
            error: None,
        }
    }
}

impl OnboardingState {
    pub fn current_step(&self) -> &WizardStep { &self.step }
    pub fn step_number(&self) -> usize { self.step.index() + 1 }
    pub fn total_steps(&self) -> usize { WizardStep::all().len() }

    /// Advance to the next step.
    pub fn next(&mut self) {
        let idx = self.step.index();
        if idx + 1 < WizardStep::all().len() {
            self.step = WizardStep::all()[idx + 1].clone();
        }
    }

    /// Go back one step.
    pub fn back(&mut self) {
        let idx = self.step.index();
        if idx > 0 {
            self.step = WizardStep::all()[idx - 1].clone();
        }
    }

    /// Execute the install step — call from the GUI on the `Installing` screen.
    pub fn run_install(&mut self) -> anyhow::Result<()> {
        let config = InstallConfig {
            platform: Platform::current(),
            prefix: self.store_path.join("bin"),
            data_dir: self.store_path.clone(),
            auto_mount: true,
            shell_integration: self.shell_integration,
            install_gui: self.install_gui,
        };
        crate::installer::install(&config)?;
        crate::sample_data::seed_sample_data(&self.store_path)?;
        self.next();
        Ok(())
    }
}

fn dirs_or_home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wizard_step_sequence() {
        let mut state = OnboardingState::default();
        assert_eq!(state.step, WizardStep::Welcome);
        state.next();
        assert_eq!(state.step, WizardStep::ChooseMode);
        state.back();
        assert_eq!(state.step, WizardStep::Welcome);
    }

    #[test]
    fn step_titles_nonempty() {
        for step in WizardStep::all() {
            assert!(!step.title().is_empty());
        }
    }

    #[test]
    fn step_numbering() {
        let state = OnboardingState::default();
        assert_eq!(state.step_number(), 1);
        assert_eq!(state.total_steps(), 7);
    }

    #[test]
    fn back_at_start_is_noop() {
        let mut state = OnboardingState::default();
        state.back();
        assert_eq!(state.step, WizardStep::Welcome);
    }
}
