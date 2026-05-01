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
    ///
    /// Reads `{volume_root}/.atlas/sync-state.json`, which the sync daemon
    /// writes atomically as a `HashMap<String, BadgeKind>` keyed by
    /// volume-relative path.  Falls back to `Synced` if the file is absent
    /// or the path is not listed (meaning no pending changes are known).
    pub fn badge_for(&self, atlas_path: &str) -> BadgeKind {
        self.load_sync_state()
            .and_then(|map| map.get(atlas_path).cloned())
            .unwrap_or(BadgeKind::Synced)
    }

    fn load_sync_state(&self) -> Option<std::collections::HashMap<String, BadgeKind>> {
        let state_file = self.volume_root.join(".atlas").join("sync-state.json");
        let raw = std::fs::read(&state_file).ok()?;
        serde_json::from_slice(&raw).ok()
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
    use tempfile::TempDir;

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

    #[test]
    fn badge_for_defaults_to_synced_when_no_state_file() {
        let dir = TempDir::new().unwrap();
        let core = FinderSyncCore::new(dir.path());
        assert_eq!(core.badge_for("/some/file.txt"), BadgeKind::Synced);
    }

    #[test]
    fn badge_for_reads_state_file() {
        let dir = TempDir::new().unwrap();
        let atlas_dir = dir.path().join(".atlas");
        std::fs::create_dir_all(&atlas_dir).unwrap();
        let state: std::collections::HashMap<&str, BadgeKind> = [
            ("/model.safetensors", BadgeKind::Modified),
            ("/data/iris.parquet", BadgeKind::Syncing),
        ]
        .into_iter()
        .collect();
        std::fs::write(atlas_dir.join("sync-state.json"), serde_json::to_vec(&state).unwrap()).unwrap();

        let core = FinderSyncCore::new(dir.path());
        assert_eq!(core.badge_for("/model.safetensors"), BadgeKind::Modified);
        assert_eq!(core.badge_for("/data/iris.parquet"), BadgeKind::Syncing);
        assert_eq!(core.badge_for("/untracked.txt"), BadgeKind::Synced);
    }
}
