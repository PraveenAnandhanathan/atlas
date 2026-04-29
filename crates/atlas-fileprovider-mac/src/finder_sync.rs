//! Finder Sync badge and toolbar integration (T6.4).
//!
//! The `FinderSyncCore` is called from the Swift `FIFinderSync` subclass
//! via the C-FFI bridge.  It decides badge icons and context-menu items
//! for any path the user has enrolled in the watched folder set.

use serde::{Deserialize, Serialize};

/// Badge icon shown on a file or directory in Finder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BadgeKind {
    /// Content matches the committed version — all synced.
    Synced,
    /// An upload or download is in progress.
    Syncing,
    /// A conflict or error requires attention.
    Error,
    /// Modified locally, not yet committed.
    Modified,
    /// New file not yet tracked.
    Untracked,
}

impl BadgeKind {
    /// Name of the badge image asset bundled in the `.appex`.
    pub fn asset_name(&self) -> &'static str {
        match self {
            Self::Synced    => "badge_synced",
            Self::Syncing   => "badge_syncing",
            Self::Error     => "badge_error",
            Self::Modified  => "badge_modified",
            Self::Untracked => "badge_untracked",
        }
    }
}

/// Toolbar / context-menu action offered by Finder Sync.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolbarAction {
    OpenInExplorer,
    ShowLineage,
    CommitNow,
    BranchFromHere,
    ShowPolicy,
}

impl ToolbarAction {
    pub fn title(&self) -> &'static str {
        match self {
            Self::OpenInExplorer => "Open in ATLAS Explorer",
            Self::ShowLineage    => "Show Lineage",
            Self::CommitNow      => "Commit Now",
            Self::BranchFromHere => "Branch from Here…",
            Self::ShowPolicy     => "Show Policy",
        }
    }

    pub fn toolbar_items() -> Vec<Self> {
        vec![Self::OpenInExplorer, Self::CommitNow, Self::ShowLineage]
    }

    pub fn menu_items() -> Vec<Self> {
        vec![
            Self::OpenInExplorer,
            Self::ShowLineage,
            Self::CommitNow,
            Self::BranchFromHere,
            Self::ShowPolicy,
        ]
    }
}

/// Core that the Swift `FIFinderSync` calls into.
pub struct FinderSyncCore {
    /// Root path of the enrolled ATLAS volume on the local filesystem.
    pub volume_root: std::path::PathBuf,
}

impl FinderSyncCore {
    pub fn new(volume_root: impl Into<std::path::PathBuf>) -> Self {
        Self { volume_root: volume_root.into() }
    }

    /// Determine the appropriate badge for `atlas_path`.
    /// In production this queries the sync daemon; stub returns Synced.
    pub fn badge_for(&self, _atlas_path: &str) -> BadgeKind {
        BadgeKind::Synced
    }

    /// Context-menu items for `atlas_path`.
    pub fn menu_items_for(&self, atlas_path: &str) -> Vec<ToolbarAction> {
        if atlas_path.ends_with('/') || atlas_path == "/" {
            ToolbarAction::menu_items()
        } else {
            ToolbarAction::menu_items()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badge_asset_names_nonempty() {
        let badges = [
            BadgeKind::Synced, BadgeKind::Syncing, BadgeKind::Error,
            BadgeKind::Modified, BadgeKind::Untracked,
        ];
        for b in &badges {
            assert!(!b.asset_name().is_empty());
        }
    }

    #[test]
    fn toolbar_titles_nonempty() {
        for item in ToolbarAction::toolbar_items() {
            assert!(!item.title().is_empty());
        }
    }
}
