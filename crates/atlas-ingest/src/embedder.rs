//! HTTP client for the Python embedder service (T3.1).
//!
//! The embedder service exposes:
//!   POST /embed         { "text": "..." }   → { "embedding": [f32…], "model_version": "…" }
//!   POST /embed_batch   { "texts": ["…"…] } → { "embeddings": [[f32…]…], "model_version": "…" }
//!   GET  /health                            → { "status": "ok" }
//!   GET  /models                            → { "current": "…", "available": ["…"] }

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct EmbedderClient {
    base_url: String,
    client: reqwest::blocking::Client,
}

#[derive(Debug, Serialize)]
struct EmbedRequest<'a> {
    text: &'a str,
}

#[derive(Debug, Serialize)]
struct BatchRequest<'a> {
    texts: &'a [String],
}

#[derive(Debug, Deserialize)]
pub struct EmbedResponse {
    pub embedding: Vec<f32>,
    pub model_version: String,
}

#[derive(Debug, Deserialize)]
pub struct BatchResponse {
    pub embeddings: Vec<Vec<f32>>,
    pub model_version: String,
}

#[derive(Debug, Deserialize)]
pub struct HealthResponse {
    pub status: String,
}

#[derive(Debug, Deserialize)]
pub struct ModelsResponse {
    pub current: String,
    pub available: Vec<String>,
}

impl EmbedderClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client: reqwest::blocking::Client::new(),
        }
    }

    pub fn embed(&self, text: &str) -> Result<EmbedResponse, String> {
        let url = format!("{}/embed", self.base_url);
        let resp = self
            .client
            .post(&url)
            .json(&EmbedRequest { text })
            .send()
            .map_err(|e| format!("http: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("embedder returned {}", resp.status()));
        }
        resp.json::<EmbedResponse>()
            .map_err(|e| format!("json: {e}"))
    }

    pub fn embed_batch(&self, texts: &[String]) -> Result<BatchResponse, String> {
        let url = format!("{}/embed_batch", self.base_url);
        let resp = self
            .client
            .post(&url)
            .json(&BatchRequest { texts })
            .send()
            .map_err(|e| format!("http: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("embedder returned {}", resp.status()));
        }
        resp.json::<BatchResponse>()
            .map_err(|e| format!("json: {e}"))
    }

    pub fn health(&self) -> Result<bool, String> {
        let url = format!("{}/health", self.base_url);
        let resp = self
            .client
            .get(&url)
            .send()
            .map_err(|e| format!("http: {e}"))?;
        Ok(resp.status().is_success())
    }

    pub fn models(&self) -> Result<ModelsResponse, String> {
        let url = format!("{}/models", self.base_url);
        let resp = self
            .client
            .get(&url)
            .send()
            .map_err(|e| format!("http: {e}"))?;
        resp.json::<ModelsResponse>()
            .map_err(|e| format!("json: {e}"))
    }
}
