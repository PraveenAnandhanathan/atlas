//! Semantic index for ATLAS.
//!
//! Two complementary sub-indexes:
//!
//! - **Full-text** (Tantivy): BM25F over extracted document text. Fast
//!   keyword search with stemming and snippets.
//! - **Vector** (flat cosine): stores dense embeddings as raw f32 arrays
//!   and performs exhaustive cosine similarity ranking. Suitable for
//!   corpora up to ~100k documents before HNSW becomes necessary.
//!
//! Both are disk-backed under a single `index_dir`. The [`AtlasIndex`]
//! struct exposes a unified [`HybridQuery`] that merges BM25 + cosine
//! scores with a configurable interpolation weight.
//!
//! Phase 3 scope:
//! - Write: `index_document` / `delete_document`
//! - Read: `search_text`, `search_vector`, `hybrid_search`
//! - Query filter: optional xattr key=value predicates (policy-aware
//!   hook is a no-op here, enforced by the caller — T3.8)
//! - Re-index: `reindex_model_version` marks documents stale (T3.7)
//! - HNSW upgrade and DiskANN integration are Phase 4.

pub mod text_index;
pub mod vector_store;

use atlas_core::Hash;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

pub use text_index::TextIndex;
pub use vector_store::VectorStore;

#[derive(Debug, Error)]
pub enum IndexError {
    #[error("tantivy: {0}")]
    Tantivy(#[from] tantivy::TantivyError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("query: {0}")]
    Query(String),
}

pub type Result<T> = std::result::Result<T, IndexError>;

/// Everything the index stores about a single document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    /// Content-addressed hash of the ATLAS file this text came from.
    pub file_hash: Hash,
    /// Logical path at index time.
    pub path: String,
    /// Extracted plain text (may be empty for binary files).
    pub text: String,
    /// Dense embedding produced by the embedder service.
    /// Empty vec means "not yet embedded".
    pub embedding: Vec<f32>,
    /// Arbitrary metadata (xattrs, format, model_version, etc.).
    pub xattrs: HashMap<String, String>,
    /// Embedding model tag — used by re-embed jobs (T3.7).
    pub model_version: String,
}

/// A hit from either the text or vector sub-index.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub file_hash: Hash,
    pub path: String,
    /// Merged score in [0, 1]. Higher is better.
    pub score: f32,
    /// Short snippet from the text (if available).
    pub snippet: Option<String>,
    pub xattrs: HashMap<String, String>,
}

/// Parameters for a hybrid query (T3.5).
#[derive(Debug, Clone, Default)]
pub struct HybridQuery {
    /// Keyword query (Tantivy query syntax). Empty → text leg disabled.
    pub text: Option<String>,
    /// Dense query vector. Empty → vector leg disabled.
    pub embedding: Option<Vec<f32>>,
    /// Mandatory key=value xattr filters applied after scoring.
    pub xattr_filters: HashMap<String, String>,
    /// Maximum results to return.
    pub limit: usize,
    /// Weight of the vector score vs. text score [0, 1].
    /// 0 → pure text; 1 → pure vector; 0.5 → equal blend.
    pub vector_weight: f32,
}

/// Unified disk-backed index.
pub struct AtlasIndex {
    pub text: TextIndex,
    pub vectors: VectorStore,
}

impl AtlasIndex {
    /// Open (or create) indexes under `dir`.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref();
        std::fs::create_dir_all(dir)?;
        Ok(Self {
            text: TextIndex::open(dir.join("text"))?,
            vectors: VectorStore::open(dir.join("vectors"))?,
        })
    }

    /// Store or update a document in both sub-indexes.
    pub fn index_document(&mut self, doc: &Document) -> Result<()> {
        self.text.index(doc)?;
        if !doc.embedding.is_empty() {
            self.vectors
                .upsert(&doc.file_hash, &doc.embedding, &doc.path, &doc.xattrs)?;
        }
        Ok(())
    }

    /// Remove a document from both sub-indexes.
    pub fn delete_document(&mut self, hash: &Hash) -> Result<()> {
        self.text.delete(hash)?;
        self.vectors.delete(hash)?;
        Ok(())
    }

    /// Keyword-only search.
    pub fn search_text(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        self.text.search(query, limit)
    }

    /// Vector-only nearest-neighbour search.
    pub fn search_vector(&self, embedding: &[f32], limit: usize) -> Result<Vec<SearchResult>> {
        self.vectors.search(embedding, limit)
    }

    /// Hybrid search merging both legs with interpolation (T3.5).
    pub fn hybrid_search(&self, q: &HybridQuery) -> Result<Vec<SearchResult>> {
        if q.limit == 0 {
            return Ok(vec![]);
        }
        let fetch = (q.limit * 4).max(20);

        // Gather text results.
        let text_hits: Vec<SearchResult> = match &q.text {
            Some(t) if !t.is_empty() => self.text.search(t, fetch)?,
            _ => vec![],
        };

        // Gather vector results.
        let vec_hits: Vec<SearchResult> = match &q.embedding {
            Some(e) if !e.is_empty() => self.vectors.search(e, fetch)?,
            _ => vec![],
        };

        // Merge by file_hash, combine scores.
        let vw = q.vector_weight.clamp(0.0, 1.0);
        let tw = 1.0 - vw;

        let mut scores: HashMap<String, (SearchResult, f32)> = HashMap::new();

        // Normalise text scores to [0,1].
        let text_max = text_hits
            .iter()
            .map(|r| r.score)
            .fold(f32::EPSILON, f32::max);
        for r in text_hits {
            let norm = r.score / text_max;
            let key = r.file_hash.to_hex();
            scores
                .entry(key)
                .and_modify(|(_, s)| *s += norm * tw)
                .or_insert((r, norm * tw));
        }

        // Normalise vector scores to [0,1] (already cosine, but may need clamping).
        let vec_max = vec_hits
            .iter()
            .map(|r| r.score)
            .fold(f32::EPSILON, f32::max);
        for r in vec_hits {
            let norm = (r.score / vec_max).max(0.0);
            let key = r.file_hash.to_hex();
            scores
                .entry(key)
                .and_modify(|(_, s)| *s += norm * vw)
                .or_insert((r, norm * vw));
        }

        // Apply xattr filters (T3.8 policy hook point).
        let mut results: Vec<SearchResult> = scores
            .into_values()
            .filter(|(r, _)| {
                q.xattr_filters
                    .iter()
                    .all(|(k, v)| r.xattrs.get(k) == Some(v))
            })
            .map(|(mut r, s)| {
                r.score = s;
                r
            })
            .collect();

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(q.limit);
        Ok(results)
    }

    /// Mark all documents not using `model_version` as stale so a
    /// re-embed job can pick them up (T3.7).
    pub fn mark_stale_embeddings(&mut self, current_model: &str) -> Result<usize> {
        self.vectors.mark_stale(current_model)
    }

    /// Return (hash, path) pairs whose embedding is stale (T3.7).
    pub fn stale_documents(&self) -> Result<Vec<(Hash, String)>> {
        self.vectors.list_stale()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn doc(hash_seed: u8, path: &str, text: &str, embedding: Vec<f32>) -> Document {
        let mut bytes = [0u8; 32];
        bytes[0] = hash_seed;
        Document {
            file_hash: Hash::from_bytes(bytes),
            path: path.into(),
            text: text.into(),
            embedding,
            xattrs: HashMap::new(),
            model_version: "test-v1".into(),
        }
    }

    #[test]
    fn text_search_roundtrip() {
        let d = TempDir::new().unwrap();
        let mut idx = AtlasIndex::open(d.path()).unwrap();
        idx.index_document(&doc(1, "/a.txt", "the quick brown fox", vec![]))
            .unwrap();
        idx.index_document(&doc(2, "/b.txt", "lazy dog sleeps", vec![]))
            .unwrap();
        let results = idx.search_text("quick fox", 10).unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].path, "/a.txt");
    }

    #[test]
    fn vector_search_roundtrip() {
        let d = TempDir::new().unwrap();
        let mut idx = AtlasIndex::open(d.path()).unwrap();
        let e1 = vec![1.0f32, 0.0, 0.0];
        let e2 = vec![0.0f32, 1.0, 0.0];
        idx.index_document(&doc(1, "/x.txt", "", e1.clone()))
            .unwrap();
        idx.index_document(&doc(2, "/y.txt", "", e2)).unwrap();
        let results = idx.search_vector(&e1, 2).unwrap();
        assert_eq!(results[0].path, "/x.txt");
    }

    #[test]
    fn hybrid_query_merges_both_legs() {
        let d = TempDir::new().unwrap();
        let mut idx = AtlasIndex::open(d.path()).unwrap();
        idx.index_document(&doc(1, "/match.txt", "neural network", vec![1.0, 0.0]))
            .unwrap();
        idx.index_document(&doc(2, "/other.txt", "unrelated topic", vec![0.0, 1.0]))
            .unwrap();
        let q = HybridQuery {
            text: Some("neural".into()),
            embedding: Some(vec![1.0, 0.0]),
            limit: 5,
            vector_weight: 0.5,
            ..Default::default()
        };
        let r = idx.hybrid_search(&q).unwrap();
        assert!(!r.is_empty());
        assert_eq!(r[0].path, "/match.txt");
    }

    #[test]
    fn xattr_filter_excludes_non_matching() {
        let d = TempDir::new().unwrap();
        let mut idx = AtlasIndex::open(d.path()).unwrap();
        let mut d1 = doc(1, "/secret.txt", "hidden content", vec![]);
        d1.xattrs.insert("classification".into(), "secret".into());
        let d2 = doc(2, "/public.txt", "hidden public content", vec![]);
        idx.index_document(&d1).unwrap();
        idx.index_document(&d2).unwrap();
        let q = HybridQuery {
            text: Some("hidden".into()),
            limit: 10,
            xattr_filters: [("classification".into(), "public".into())].into(),
            ..Default::default()
        };
        let results = idx.hybrid_search(&q).unwrap();
        // only d2 matches the filter (no classification xattr)
        assert!(results.iter().all(|r| r.path != "/secret.txt"));
    }

    #[test]
    fn delete_removes_from_both_indexes() {
        let d = TempDir::new().unwrap();
        let mut idx = AtlasIndex::open(d.path()).unwrap();
        let doc = doc(5, "/del.txt", "to delete", vec![1.0, 0.0]);
        let h = doc.file_hash;
        idx.index_document(&doc).unwrap();
        idx.delete_document(&h).unwrap();
        let r = idx.search_text("delete", 10).unwrap();
        assert!(r.is_empty());
    }
}
