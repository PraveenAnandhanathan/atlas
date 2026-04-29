//! HTTP server core for the admin console (T6.8).
//!
//! Uses a hand-rolled async TCP listener so we don't pull in axum/hyper
//! as workspace dependencies; those are added to this crate's own
//! Cargo.toml when the team is ready to pin a web framework.

use anyhow::Result;
use atlas_fs::Fs;
use atlas_mcp::CapabilityCore;
use serde_json::json;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Shared server state.
pub struct AppState {
    pub fs: Arc<Fs>,
    pub core: Arc<CapabilityCore>,
}

/// Start the HTTP server and block until a shutdown signal.
pub async fn run(bind: SocketAddr, store: PathBuf, _cors_origins: Vec<String>) -> Result<()> {
    let fs = Fs::open(&store)?;
    let core = CapabilityCore::new(fs.clone(), None, None, None, None, None);

    let state = Arc::new(AppState { fs: Arc::new(fs), core: Arc::new(core) });

    let listener = TcpListener::bind(bind).await?;
    tracing::info!(addr = %bind, "atlas-web listening");

    loop {
        let (stream, peer) = listener.accept().await?;
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, peer, state).await {
                tracing::warn!(peer = %peer, err = %e, "connection error");
            }
        });
    }
}

async fn handle_connection(
    mut stream: tokio::net::TcpStream,
    peer: SocketAddr,
    state: Arc<AppState>,
) -> Result<()> {
    let mut buf = vec![0u8; 8192];
    let n = stream.read(&mut buf).await?;
    if n == 0 { return Ok(()); }

    let request = std::str::from_utf8(&buf[..n]).unwrap_or("");
    let (method, path) = parse_request_line(request);
    tracing::debug!(peer = %peer, method, path, "request");

    let (status, body, content_type) = dispatch(method, path, &state).await;
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n{body}",
        len = body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    Ok(())
}

async fn dispatch(method: &str, path: &str, state: &AppState) -> (u16, String, &'static str) {
    match (method, path) {
        ("GET", "/health") => {
            (200, json!({"status": "ok", "version": env!("CARGO_PKG_VERSION")}).to_string(), "application/json")
        }
        ("GET", "/metrics") => {
            (200, crate::metrics::render(&state.fs), "text/plain")
        }
        ("GET", p) if p.starts_with("/api/") => {
            crate::api::handle(p, state).await
        }
        ("GET", "/") | ("GET", "/index.html") => {
            (200, crate::static_files::INDEX_HTML.to_string(), "text/html")
        }
        _ => {
            (404, json!({"error": "not found"}).to_string(), "application/json")
        }
    }
}

fn parse_request_line(raw: &str) -> (&str, &str) {
    let line = raw.lines().next().unwrap_or("");
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("GET");
    let path   = parts.next().unwrap_or("/");
    (method, path)
}
