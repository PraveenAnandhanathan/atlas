//! Ingest pipeline (T3.3).
//!
//! `Ingester` is the single entry point: give it a file path and it
//! 1. reads the bytes from the ATLAS store,
//! 2. detects the format and extracts plain text (T3.4),
//! 3. calls the embedder service to get a dense embedding (T3.1),
//! 4. writes a `Document` into `AtlasIndex` (T3.2).
//!
//! The `EmbedderClient` is a blocking HTTP client pointing at the
//! Python embedder service (`services/embedder/`). If the service is
//! unavailable the pipeline falls back to storing text-only (no vector).
//!
//! Re-embedding (T3.7): `Ingester::reembed_stale` fetches stale docs
//! from the vector store and sends them through the embedder with a new
//! model version tag.
//!
//! Policy filtering (T3.8): callers pass an optional `PolicyFilter`
//! closure; if it returns `false` for a document the ingest is skipped.

pub mod embedder;
pub mod formats;
pub mod policy;
pub mod reembed;

use atlas_core::{Hash, ObjectKind};
use atlas_fs::Fs;
use atlas_indexer::{AtlasIndex, Document, HybridQuery, IndexError, SearchResult};
use embedder::EmbedderClient;
use formats::extract_text;
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;
use tracing::{debug, info, warn};

#[derive(Debug, Error)]
pub enum IngestError {
    #[error("index: {0}")]
    Index(#[from] IndexError),
    #[error("atlas: {0}")]
    Atlas(#[from] atlas_core::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("embedder: {0}")]
    Embedder(String),
}

pub type Result<T> = std::result::Result<T, IngestError>;

/// Drives the full ingest pipeline for one ATLAS store.
pub struct Ingester {
    pub index: AtlasIndex,
    pub embedder: Option<EmbedderClient>,
}

impl Ingester {
    /// Open an ingester backed by `index_dir`.
    /// `embedder_url` is optional; pass `None` for text-only indexing.
    pub fn open(index_dir: impl AsRef<Path>, embedder_url: Option<&str>) -> Result<Self> {
        Ok(Self {
            index: AtlasIndex::open(index_dir)?,
            embedder: embedder_url.map(EmbedderClient::new),
        })
    }

    /// Ingest a single file from the ATLAS store. Idempotent.
    ///
    /// `path` is the logical ATLAS path (e.g. `/docs/notes.txt`).
    /// `xattrs` are additional metadata to store alongside (e.g. xattrs
    /// read from the ATLAS file entry).
    pub fn ingest_file(
        &mut self,
        fs: &Fs,
        path: &str,
        xattrs: HashMap<String, String>,
        policy: &dyn policy::PolicyFilter,
    ) -> Result<Hash> {
        let entry = fs.stat(path)?;
        if entry.kind != ObjectKind::File {
            return Err(IngestError::Embedder(format!("{path} is not a file")));
        }
        let file_hash = entry.hash;
        let bytes = fs.read(path)?;

        let text = extract_text(path, &bytes);
        debug!(path, chars = text.len(), "extracted text");

        // Build preliminary doc to check policy.
        let doc_preview = Document {
            file_hash,
            path: path.into(),
            text: text.clone(),
            embedding: vec![],
            xattrs: xattrs.clone(),
            model_version: String::new(),
        };
        if !policy.allow(&doc_preview) {
            info!(path, "policy blocked ingest");
            return Ok(file_hash);
        }

        // Request embedding.
        let (embedding, model_version) = match &self.embedder {
            Some(client) => match client.embed(&text) {
                Ok(resp) => (resp.embedding, resp.model_version),
                Err(e) => {
                    warn!(path, error = %e, "embedder unavailable, storing text-only");
                    (vec![], String::new())
                }
            },
            None => (vec![], String::new()),
        };

        let doc = Document {
            file_hash,
            path: path.into(),
            text,
            embedding,
            xattrs,
            model_version,
        };
        self.index.index_document(&doc)?;
        info!(path, hash = %file_hash.short(), "ingested");
        Ok(file_hash)
    }

    /// Walk an ATLAS directory recursively and ingest every file.
    pub fn ingest_tree(
        &mut self,
        fs: &Fs,
        dir: &str,
        policy: &dyn policy::PolicyFilter,
    ) -> Result<usize> {
        let mut count = 0;
        self.walk_dir(fs, dir, policy, &mut count)?;
        Ok(count)
    }

    fn walk_dir(
        &mut self,
        fs: &Fs,
        dir: &str,
        policy: &dyn policy::PolicyFilter,
        count: &mut usize,
    ) -> Result<()> {
        let entries = fs.list(dir)?;
        for e in entries {
            match e.kind {
                ObjectKind::Dir => self.walk_dir(fs, &e.path, policy, count)?,
                ObjectKind::File => {
                    self.ingest_file(fs, &e.path, HashMap::new(), policy)?;
                    *count += 1;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Semantic search delegating to the underlying index.
    pub fn search(&self, q: &HybridQuery) -> Result<Vec<SearchResult>> {
        Ok(self.index.hybrid_search(q)?)
    }

    /// Re-embed stale documents (those indexed with an old model) and
    /// update the vector store with `current_model` (T3.7).
    pub fn reembed_stale(&mut self, current_model: &str) -> Result<reembed::ReembedReport> {
        reembed::reembed_stale(&mut self.index, self.embedder.as_ref(), current_model)
    }
}
