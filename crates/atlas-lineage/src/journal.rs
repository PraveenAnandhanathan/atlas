//! JSONL-backed lineage edge journal with BFS graph traversal.

use crate::{LineageEdge, Result, SamplingConfig};
use atlas_core::Hash;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

pub struct LineageJournal {
    path: PathBuf,
    sampling: SamplingConfig,
    counter: u64,
}

impl LineageJournal {
    pub fn open(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref();
        std::fs::create_dir_all(dir)?;
        Ok(Self {
            path: dir.join("lineage.jsonl"),
            sampling: SamplingConfig::default(),
            counter: 0,
        })
    }

    pub fn with_sampling(mut self, config: SamplingConfig) -> Self {
        self.sampling = config;
        self
    }

    /// Append an edge unconditionally.
    pub fn record(&mut self, edge: LineageEdge) -> Result<()> {
        let line = serde_json::to_string(&edge)?;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(f, "{line}")?;
        Ok(())
    }

    /// Append an edge subject to the sampling rate.
    /// Returns `true` if the edge was written, `false` if skipped.
    pub fn record_sampled(&mut self, edge: LineageEdge) -> Result<bool> {
        self.counter = self.counter.wrapping_add(1);
        let threshold = (self.sampling.rate.clamp(0.0, 1.0) * 1000.0).round() as u64;
        if threshold == 0 || (self.counter % 1000) >= threshold {
            return Ok(false);
        }
        self.record(edge)?;
        Ok(true)
    }

    /// Load all edges from the journal.
    pub fn all_edges(&self) -> Result<Vec<LineageEdge>> {
        self.load_all()
    }

    fn load_all(&self) -> Result<Vec<LineageEdge>> {
        if !self.path.exists() {
            return Ok(vec![]);
        }
        let f = std::fs::File::open(&self.path)?;
        Ok(BufReader::new(f)
            .lines()
            .map_while(|l| l.ok())
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str(&l).ok())
            .collect())
    }

    /// Direct producers: edges where `hash` is the sink.
    pub fn parents(&self, hash: &Hash) -> Result<Vec<LineageEdge>> {
        let hex = hash.to_hex();
        Ok(self
            .load_all()?
            .into_iter()
            .filter(|e| e.sink_hash.to_hex() == hex)
            .collect())
    }

    /// Direct consumers: edges where `hash` is the source.
    pub fn children(&self, hash: &Hash) -> Result<Vec<LineageEdge>> {
        let hex = hash.to_hex();
        Ok(self
            .load_all()?
            .into_iter()
            .filter(|e| e.source_hash.to_hex() == hex)
            .collect())
    }

    /// BFS upstream: all ancestor edges within `depth` hops.
    pub fn ancestors(&self, hash: &Hash, depth: usize) -> Result<Vec<LineageEdge>> {
        let all = self.load_all()?;
        let mut by_sink: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, e) in all.iter().enumerate() {
            by_sink.entry(e.sink_hash.to_hex()).or_default().push(i);
        }
        bfs_collect(&all, &by_sink, hash.to_hex(), depth, |e| {
            e.source_hash.to_hex()
        })
    }

    /// BFS downstream: all descendant edges within `depth` hops.
    pub fn descendants(&self, hash: &Hash, depth: usize) -> Result<Vec<LineageEdge>> {
        let all = self.load_all()?;
        let mut by_source: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, e) in all.iter().enumerate() {
            by_source.entry(e.source_hash.to_hex()).or_default().push(i);
        }
        bfs_collect(&all, &by_source, hash.to_hex(), depth, |e| {
            e.sink_hash.to_hex()
        })
    }
}

fn bfs_collect(
    all: &[LineageEdge],
    index: &HashMap<String, Vec<usize>>,
    start: String,
    depth: usize,
    next_key: impl Fn(&LineageEdge) -> String,
) -> Result<Vec<LineageEdge>> {
    let mut result = Vec::new();
    let mut visited: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<(String, usize)> = VecDeque::new();
    queue.push_back((start, 0));
    while let Some((cur, d)) = queue.pop_front() {
        if d >= depth {
            continue;
        }
        if let Some(idxs) = index.get(&cur) {
            for &i in idxs {
                let e = &all[i];
                if visited.insert(e.id.clone()) {
                    queue.push_back((next_key(e), d + 1));
                    result.push(e.clone());
                }
            }
        }
    }
    Ok(result)
}
