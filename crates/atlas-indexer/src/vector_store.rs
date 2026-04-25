//! Flat (exhaustive cosine) vector store.
//!
//! Embeddings are stored as length-prefixed f32 arrays in a simple
//! line-delimited JSON file. Exhaustive O(n) search is fine up to
//! ~100k documents; DiskANN/HNSW upgrade is deferred to Phase 4.

use crate::{Result, SearchResult};
use atlas_core::Hash;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

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
}

impl VectorStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        std::fs::create_dir_all(&path)?;
        let file_path = path.join("vectors.jsonl");
        let entries = if file_path.exists() {
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
        Ok(Self { path, entries })
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
        self.flush()
    }

    pub fn delete(&mut self, hash: &Hash) -> Result<()> {
        let hex = hash.to_hex();
        self.entries.retain(|e| e.hash_hex != hex);
        self.flush()
    }

    /// Cosine similarity nearest-neighbour search (brute-force).
    pub fn search(&self, query: &[f32], limit: usize) -> Result<Vec<SearchResult>> {
        if query.is_empty() || self.entries.is_empty() {
            return Ok(vec![]);
        }
        let q_norm = l2_norm(query);
        if q_norm < f32::EPSILON {
            return Ok(vec![]);
        }

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
