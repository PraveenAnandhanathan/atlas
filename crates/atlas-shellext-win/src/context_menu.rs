//! ATLAS Explorer context-menu handler (T6.2).
//!
//! Implements the `IContextMenu` COM interface surface.  The pure-Rust
//! layer here decides which menu items to show and what they do; the
//! COM registration and `QueryContextMenu` / `InvokeCommand` dispatch
//! lives in the thin C++ shim compiled only on Windows.

use serde::{Deserialize, Serialize};

/// Action that can be triggered from the context menu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContextAction {
    /// Open the file/directory in the ATLAS Explorer GUI.
    OpenInExplorer,
    /// Copy the BLAKE3 hash of the selected file to the clipboard.
    CopyHash,
    /// Open the lineage graph viewer for the selected path.
    ShowLineage,
    /// Commit the working root snapshot immediately.
    CommitNow,
    /// Create a new branch from the current commit of this path.
    BranchFromHere,
    /// Reveal the policy attached to this path.
    ShowPolicy,
}

impl ContextAction {
    pub fn label(&self) -> &'static str {
        match self {
            Self::OpenInExplorer => "Open in ATLAS Explorer",
            Self::CopyHash       => "Copy ATLAS hash",
            Self::ShowLineage    => "Show lineage",
            Self::CommitNow      => "Commit now",
            Self::BranchFromHere => "Branch from here…",
            Self::ShowPolicy     => "Show policy",
        }
    }

    /// Verb string used by `IContextMenu::GetCommandString`.
    pub fn verb(&self) -> &'static str {
        match self {
            Self::OpenInExplorer => "atlas.open",
            Self::CopyHash       => "atlas.copy-hash",
            Self::ShowLineage    => "atlas.lineage",
            Self::CommitNow      => "atlas.commit",
            Self::BranchFromHere => "atlas.branch",
            Self::ShowPolicy     => "atlas.policy",
        }
    }

    /// Items shown for a regular file.
    pub fn for_file() -> Vec<Self> {
        vec![
            Self::OpenInExplorer,
            Self::CopyHash,
            Self::ShowLineage,
            Self::CommitNow,
            Self::BranchFromHere,
            Self::ShowPolicy,
        ]
    }

    /// Items shown for a directory.
    pub fn for_directory() -> Vec<Self> {
        vec![
            Self::OpenInExplorer,
            Self::ShowLineage,
            Self::CommitNow,
            Self::BranchFromHere,
            Self::ShowPolicy,
        ]
    }
}

/// Handler that resolves actions to shell commands / IPC calls.
pub struct ContextMenuHandler {
    /// Path to the `atlasctl` binary (resolved at registration time).
    pub atlasctl_path: std::path::PathBuf,
}

impl ContextMenuHandler {
    pub fn new(atlasctl_path: impl Into<std::path::PathBuf>) -> Self {
        Self { atlasctl_path: atlasctl_path.into() }
    }

    /// Build the command-line for `action` on `atlas_path`.
    /// The shell extension invokes this via `ShellExecuteEx` or IPC.
    pub fn command_for(&self, action: &ContextAction, atlas_path: &str) -> Vec<String> {
        let ctl = self.atlasctl_path.to_string_lossy().to_string();
        match action {
            ContextAction::OpenInExplorer => {
                vec!["atlas-explorer".into(), "--path".into(), atlas_path.to_string()]
            }
            ContextAction::CopyHash => {
                vec![ctl, "stat".into(), atlas_path.to_string(), "--format".into(), "hash".into()]
            }
            ContextAction::ShowLineage => {
                vec![ctl, "lineage".into(), "show".into(), atlas_path.to_string()]
            }
            ContextAction::CommitNow => {
                vec![ctl, "commit".into(), "--message".into(), "manual commit from Explorer".into()]
            }
            ContextAction::BranchFromHere => {
                vec![ctl, "branch".into(), "create".into(), "--from".into(), "HEAD".into()]
            }
            ContextAction::ShowPolicy => {
                vec![ctl, "policy".into(), "show".into(), atlas_path.to_string()]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbs_are_unique() {
        let actions = ContextAction::for_file();
        let verbs: Vec<_> = actions.iter().map(|a| a.verb()).collect();
        let unique: std::collections::HashSet<_> = verbs.iter().copied().collect();
        assert_eq!(verbs.len(), unique.len());
    }

    #[test]
    fn labels_are_nonempty() {
        for action in ContextAction::for_file() {
            assert!(!action.label().is_empty(), "{:?} has empty label", action);
        }
    }

    #[test]
    fn command_for_copy_hash_contains_path() {
        let handler = ContextMenuHandler::new("/usr/bin/atlasctl");
        let cmd = handler.command_for(&ContextAction::CopyHash, "/data/model.safetensors");
        assert!(cmd.iter().any(|a| a.contains("model.safetensors")));
    }
}
