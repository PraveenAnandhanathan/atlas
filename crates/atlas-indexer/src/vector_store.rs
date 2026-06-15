//! Vector store with two-layer architecture:
//!
//! 1. **Flat persistence layer** — embeddings are stored as line-delimited JSON
//!    (`vectors.jsonl`).  Exhaustive O(n) search is the fallback.
//! 2. **HNSW in-memory index** — built from the flat store at open time and
//!    updated on every `upsert`.  Used for ANN search when the corpus has at
//!    least 100 nodes (threshold where HNSW pays off).
//!
//! The search path is:
//! - `n < 100`  → brute-force cosine similarity (exact)
//! - `n >= 100` → HNSW k-NN followed by exact re-ranking of the candidates

use crate::hnsw::HnswIndex;
use crate::{Result, SearchResult};
use atlas_core::Hash;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use tracing::{error, warn};

/// Threshold: switch from brute-force to HNSW once the store is this large.
const HNSW_THRESHOLD: usize = 100;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VectorEntry {
    hash_hex: String,
    path: String,
    embedding: Vec<f32>,
    xattrs: HashMap<String, String>,
    model_version: String,
    stale: bool,
}

pub struct VectorStore {
    path: PathBuf,
    entries: Vec<VectorEntry>,
    /// HNSW index: node `i` corresponds to `entries[i]`.
    /// Rebuilt whenever `entries` is reloaded or mutated.
    hnsw: HnswIndex,
}

impl VectorStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        std::fs::create_dir_all(&path)?;
        let file_path = path.join("vectors.jsonl");
        let entries: Vec<VectorEntry> = if file_path.exists() {
            let f = std::fs::File::open(&file_path)?;
            BufReader::new(f)
                .lines()
                .map_while(|l| l.ok())
                .filter(|l| !l.trim().is_empty())
                .filter_map(|l| serde_json::from_str(&l).ok())
                .collect()
        } else {
            Vec::new()
        };

        let hnsw = Self::build_hnsw(&entries);
        Ok(Self {
            path,
            entries,
            hnsw,
        })
    }

    /// Build an HNSW index from scratch over the given entry list.
    fn build_hnsw(entries: &[VectorEntry]) -> HnswIndex {
        let mut idx = HnswIndex::new();
        for e in entries {
            if !e.embedding.is_empty() {
                idx.insert(&e.embedding);
            }
        }
        idx
    }

    fn file_path(&self) -> PathBuf {
        self.path.join("vectors.jsonl")
    }

    fn flush(&self) -> Result<()> {
        let f = std::fs::File::create(self.file_path())?;
        let mut w = BufWriter::new(f);
        for e in &self.entries {
            serde_json::to_writer(&mut w, e)?;
            w.write_all(b"\n")?;
        }
        w.flush()?;
        Ok(())
    }

    /// Rebuild the HNSW index in place (called after any structural mutation).
    fn rebuild_hnsw(&mut self) {
        self.hnsw = Self::build_hnsw(&self.entries);
    }

    pub fn upsert(
        &mut self,
        hash: &Hash,
        embedding: &[f32],
        path: &str,
        xattrs: &HashMap<String, String>,
    ) -> Result<()> {
        self.delete(hash)?;
        self.entries.push(VectorEntry {
            hash_hex: hash.to_hex(),
            path: path.into(),
            embedding: embedding.to_vec(),
            xattrs: xattrs.clone(),
            model_version: String::new(),
            stale: false,
        });
        self.rebuild_hnsw();
        self.flush()
    }

    pub fn upsert_with_model(
        &mut self,
        hash: &Hash,
        embedding: &[f32],
        path: &str,
        xattrs: &HashMap<String, String>,
        model_version: &str,
    ) -> Result<()> {
        self.delete(hash)?;
        self.entries.push(VectorEntry {
            hash_hex: hash.to_hex(),
            path: path.into(),
            embedding: embedding.to_vec(),
            xattrs: xattrs.clone(),
            model_version: model_version.into(),
            stale: false,
        });
        self.rebuild_hnsw();
        self.flush()
    }

    pub fn delete(&mut self, hash: &Hash) -> Result<()> {
        let hex = hash.to_hex();
        let before = self.entries.len();
        self.entries.retain(|e| e.hash_hex != hex);
        let removed = before - self.entries.len();
        if removed > 0 {
            // A deletion invalidates node-id ↔ entry-index correspondence,
            // so rebuild the entire HNSW.
            self.rebuild_hnsw();
        }
        self.flush()
    }

    /// Cosine similarity nearest-neighbour search.
    ///
    /// For corpora with fewer than `HNSW_THRESHOLD` entries, brute-force O(n)
    /// is used.  Above that threshold the HNSW index is queried and the
    /// resulting candidates are re-ranked exactly.
    pub fn search(&self, query: &[f32], limit: usize) -> Result<Vec<SearchResult>> {
        let n = self.entries.len();
        if n > 250_000 {
            error!(
                documents = n,
                "vector store exceeds 250 000 documents; search latency may be unacceptable"
            );
        } else if n > 50_000 && self.hnsw.len() < HNSW_THRESHOLD {
            warn!(
                documents = n,
                "vector store has more than 50 000 documents without HNSW; \
                 brute-force search latency may be unacceptable"
            );
        }
        if query.is_empty() || self.entries.is_empty() {
            return Ok(vec![]);
        }
        let q_norm = l2_norm(query);
        if q_norm < f32::EPSILON {
            return Ok(vec![]);
        }

        if self.hnsw.len() >= HNSW_THRESHOLD {
            self.search_via_hnsw(query, q_norm, limit)
        } else {
            self.search_brute_force(query, q_norm, limit)
        }
    }

    /// HNSW-accelerated search with exact re-ranking of candidates.
    fn search_via_hnsw(
        &self,
        query: &[f32],
        q_norm: f32,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        // Over-fetch candidates so re-ranking has enough material.
        let fetch = (limit * 4).max(crate::hnsw::EF_SEARCH);
        let candidates = self.hnsw.knn_search(query, fetch);

        // Re-rank candidates exactly (the HNSW approximate score may differ
        // slightly from the precise cosine due to normalisation).
        let mut scored: Vec<(f32, usize)> = candidates
            .into_iter()
            .filter_map(|(_, node_id)| {
                self.entries.get(node_id).and_then(|e| {
                    if e.embedding.len() == query.len() {
                        let score = cosine_sim(query, &e.embedding, q_norm);
                        Some((score, node_id))
                    } else {
                        None
                    }
                })
            })
            .collect();

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);

        Ok(scored
            .into_iter()
            .map(|(score, id)| {
                let e = &self.entries[id];
                SearchResult {
                    file_hash: Hash::from_hex(&e.hash_hex).unwrap_or(Hash::ZERO),
                    path: e.path.clone(),
                    score,
                    snippet: None,
                    xattrs: e.xattrs.clone(),
                }
            })
            .collect())
    }

    /// Brute-force O(n) cosine similarity search (used for small corpora).
    fn search_brute_force(
        &self,
        query: &[f32],
        q_norm: f32,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        let mut scored: Vec<(f32, &VectorEntry)> = self
            .entries
            .iter()
            .filter(|e| e.embedding.len() == query.len())
            .map(|e| {
                let score = cosine_sim(query, &e.embedding, q_norm);
                (score, e)
            })
            .collect();

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);

        Ok(scored
            .into_iter()
            .map(|(score, e)| SearchResult {
                file_hash: Hash::from_hex(&e.hash_hex).unwrap_or(Hash::ZERO),
                path: e.path.clone(),
                score,
                snippet: None,
                xattrs: e.xattrs.clone(),
            })
            .collect())
    }

    /// Mark entries whose model_version differs from `current` as stale (T3.7).
    pub fn mark_stale(&mut self, current_model: &str) -> Result<usize> {
        let mut count = 0;
        for e in &mut self.entries {
            if e.model_version != current_model && !e.stale {
                e.stale = true;
                count += 1;
            }
        }
        self.flush()?;
        Ok(count)
    }

    /// Look up a stored embedding by logical path.
    /// Returns the embedding of the first non-empty match, or `None`.
    pub fn get_embedding_by_path(&self, path: &str) -> Option<Vec<f32>> {
        self.entries
            .iter()
            .find(|e| e.path == path && !e.embedding.is_empty())
            .map(|e| e.embedding.clone())
    }

    /// Return (hash, path) for stale entries (T3.7).
    pub fn list_stale(&self) -> Result<Vec<(Hash, String)>> {
        Ok(self
            .entries
            .iter()
            .filter(|e| e.stale)
            .filter_map(|e| {
                Hash::from_hex(&e.hash_hex)
                    .ok()
                    .map(|h| (h, e.path.clone()))
            })
            .collect())
    }
}

fn l2_norm(v: &[f32]) -> f32 {
    v.iter().map(|x| x * x).sum::<f32>().sqrt()
}

fn cosine_sim(a: &[f32], b: &[f32], a_norm: f32) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let b_norm = l2_norm(b);
    if b_norm < f32::EPSILON {
        return 0.0;
    }
    (dot / (a_norm * b_norm)).clamp(-1.0, 1.0)
}
