use atlas_storage::{serve, ServerConfig};
use clap::Parser;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "atlas-storage", version, about = "ATLAS storage server")]
struct Args {
    #[arg(long, default_value = "0.0.0.0:7373", env = "ATLAS_STORAGE_BIND")]
    bind: String,
    #[arg(long, env = "ATLAS_STORAGE_CHUNKS")]
    chunks_dir: PathBuf,
    #[arg(long, env = "ATLAS_STORAGE_META")]
    meta_dir: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let args = Args::parse();
    serve(ServerConfig {
        bind: args.bind,
        chunks_dir: args.chunks_dir,
        meta_dir: args.meta_dir,
    })
    .await
}
