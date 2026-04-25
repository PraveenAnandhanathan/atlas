//! Time-window rollup of lineage edges (T4.3).

use crate::LineageEdge;
use std::collections::{HashMap, HashSet};

/// Aggregate statistics for one fixed-width time window.
#[derive(Debug, Clone)]
pub struct RollupBucket {
    /// Window start, milliseconds since Unix epoch.
    pub window_start_ms: u64,
    /// Window end (exclusive), milliseconds since Unix epoch.
    pub window_end_ms: u64,
    /// Edge count by EdgeKind display string.
    pub counts: HashMap<String, usize>,
    pub unique_sources: usize,
    pub unique_sinks: usize,
}

/// Partition `edges` into fixed-width windows of `window_secs` seconds.
/// Empty windows are omitted.
pub fn rollup_window(edges: &[LineageEdge], window_secs: u64) -> Vec<RollupBucket> {
    if edges.is_empty() || window_secs == 0 {
        return vec![];
    }
    let window_ms = window_secs * 1000;
    let min_ts = edges.iter().map(|e| e.ts).min().unwrap_or(0);
    let max_ts = edges.iter().map(|e| e.ts).max().unwrap_or(0);

    let mut buckets = Vec::new();
    let mut t = (min_ts / window_ms) * window_ms;
    while t <= max_ts {
        let start = t;
        let end = t + window_ms;
        let window_edges: Vec<&LineageEdge> = edges
            .iter()
            .filter(|e| e.ts >= start && e.ts < end)
            .collect();
        if !window_edges.is_empty() {
            let mut counts: HashMap<String, usize> = HashMap::new();
            let mut sources: HashSet<String> = HashSet::new();
            let mut sinks: HashSet<String> = HashSet::new();
            for e in &window_edges {
                *counts.entry(e.kind.to_string()).or_default() += 1;
                sources.insert(e.source_hash.to_hex());
                sinks.insert(e.sink_hash.to_hex());
            }
            buckets.push(RollupBucket {
                window_start_ms: start,
                window_end_ms: end,
                counts,
                unique_sources: sources.len(),
                unique_sinks: sinks.len(),
            });
        }
        t += window_ms;
    }
    buckets
}
