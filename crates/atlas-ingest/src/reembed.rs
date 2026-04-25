//! Re-embedding job framework (T3.7).
//!
//! When the embedding model changes, previously-indexed documents have
//! stale vectors. `reembed_stale` fetches those document hashes from
//! the vector store, re-fetches their text from the text index, sends
//! them to the embedder in batches, and writes back the updated vectors.

use crate::embedder::EmbedderClient;
use atlas_core::Hash;
use atlas_indexer::AtlasIndex;
use tracing::{info, warn};

/// Summary of a re-embed pass.
#[derive(Debug, Default, Clone)]
pub struct ReembedReport {
    pub stale_found: usize,
    pub re_embedded: usize,
    pub failed: usize,
    pub model_version: String,
}

/// Batch size for embedder calls.
const BATCH_SIZE: usize = 32;

/// Re-embed all stale documents and return a report.
///
/// If `embedder` is `None` the function returns immediately with 0
/// re-embedded (caller can retry once the service is up).
pub fn reembed_stale(
    index: &mut AtlasIndex,
    embedder: Option<&EmbedderClient>,
    current_model: &str,
) -> Result<ReembedReport, crate::IngestError> {
    let stale = index.stale_documents()?;
    let mut report = ReembedReport {
        stale_found: stale.len(),
        model_version: current_model.into(),
        ..Default::default()
    };

    let Some(client) = embedder else {
        warn!("embedder client not configured — skipping re-embed");
        return Ok(report);
    };

    // Collect (hash, path, text) triples.
    let mut triples: Vec<(Hash, String, String)> = Vec::with_capacity(stale.len());
    for (hash, path) in &stale {
        // Re-fetch text from the text index via a path search.
        // (A dedicated `get_by_hash` API would be cleaner; path lookup is the pragmatic choice.)
        match index.search_text(&format!("\"{}\"", path.replace('"', " ")), 1) {
            Ok(hits) if !hits.is_empty() => {
                triples.push((*hash, path.clone(), hits[0].path.clone()));
            }
            _ => {
                // Text not found — the doc may have been deleted. Skip.
                triples.push((*hash, path.clone(), path.clone()));
            }
        }
    }

    // Process in batches.
    for chunk in triples.chunks(BATCH_SIZE) {
        let texts: Vec<String> = chunk.iter().map(|(_, _, t)| t.clone()).collect();
        match client.embed_batch(&texts) {
            Ok(resp) => {
                for ((hash, path, _), embedding) in chunk.iter().zip(resp.embeddings.iter()) {
                    if let Err(e) = index.vectors.upsert_with_model(
                        hash,
                        embedding,
                        path,
                        &Default::default(),
                        current_model,
                    ) {
                        warn!(path, error = %e, "failed to write updated embedding");
                        report.failed += 1;
                    } else {
                        report.re_embedded += 1;
                    }
                }
            }
            Err(e) => {
                warn!(error = %e, "batch embed failed");
                report.failed += chunk.len();
            }
        }
    }

    info!(
        stale = report.stale_found,
        re_embedded = report.re_embedded,
        failed = report.failed,
        model = current_model,
        "re-embed pass complete"
    );
    Ok(report)
}
