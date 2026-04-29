//! ATLAS web admin console (T6.8).
//!
//! A lightweight HTTP server that serves:
//! - `GET  /`          → admin dashboard HTML (single-page app).
//! - `GET  /api/*`     → proxied to the REST capability core.
//! - `GET  /metrics`   → Prometheus-style text metrics.
//! - `GET  /health`    → `{"status":"ok"}`.
//!
//! The static SPA lives in `services/web/static/`.  In production it is
//! embedded in the binary with `include_str!` / `include_bytes!`.
//!
//! Usage:
//!   atlas-web --bind 127.0.0.1:8080 --store /var/lib/atlas

mod api;
mod metrics;
mod server;
mod static_files;

use anyhow::Result;
use clap::Parser;
use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "atlas-web", version, about = "ATLAS web admin console")]
struct Args {
    /// Socket address to bind (host:port).
    #[arg(long, default_value = "127.0.0.1:8080")]
    bind: SocketAddr,

    /// Path to the ATLAS store.
    #[arg(long, env = "ATLAS_STORE")]
    store: PathBuf,

    /// Allowed CORS origin (repeat for multiple).
    #[arg(long)]
    cors_origin: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    tracing::info!(bind = %args.bind, store = %args.store.display(), "atlas-web starting");
    server::run(args.bind, args.store, args.cors_origin).await
}
