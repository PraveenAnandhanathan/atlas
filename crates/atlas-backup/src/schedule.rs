//! Backup schedule and retention policies (T7.2).

use serde::{Deserialize, Serialize};

/// How often to take a backup.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupSchedule {
    /// Every `interval_secs` seconds.
    Interval { interval_secs: u64 },
    /// Cron-style expression (e.g. `"0 2 * * *"` = 02:00 daily).
    Cron { expr: String },
    /// Manual trigger only.
    Manual,
}

impl BackupSchedule {
    pub fn daily_at_2am() -> Self {
        Self::Cron { expr: "0 2 * * *".into() }
    }

    pub fn hourly() -> Self {
        Self::Interval { interval_secs: 3600 }
    }
}

/// How long to keep old backup bundles.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionPolicy {
    /// Keep all backups newer than this many seconds.
    pub keep_recent_secs: u64,
    /// Keep at most this many full backups.
    pub max_full_backups: usize,
    /// Keep at most this many incremental bundles between two fulls.
    pub max_incremental_per_chain: usize,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            keep_recent_secs: 7 * 24 * 3600, // 7 days
            max_full_backups: 4,
            max_incremental_per_chain: 48,
        }
    }
}

impl RetentionPolicy {
    /// Return indices of manifests to delete given their ages in seconds.
    pub fn manifests_to_prune(&self, ages_secs: &[u64]) -> Vec<usize> {
        ages_secs
            .iter()
            .enumerate()
            .filter(|(_, &age)| age > self.keep_recent_secs)
            .map(|(i, _)| i)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retention_prune_old() {
        let policy = RetentionPolicy::default();
        let ages = vec![0, 3600, 8 * 24 * 3600]; // first two recent, last old
        let to_prune = policy.manifests_to_prune(&ages);
        assert_eq!(to_prune, vec![2]);
    }

    #[test]
    fn schedule_daily() {
        let s = BackupSchedule::daily_at_2am();
        assert!(matches!(s, BackupSchedule::Cron { .. }));
    }
}
