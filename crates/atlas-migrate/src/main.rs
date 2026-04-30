//! `atlas-migrate` CLI (T7.8).

use anyhow::{Context, Result};
use atlas_fs::Fs;
use atlas_migrate::{parse_source, pipeline::MigrationConfig, run};
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "atlas-migrate", version, about = "ATLAS data migration tool")]
struct Args {
    /// Path to the target ATLAS store (opened or initialised if absent).
    #[arg(long, default_value = ".atlas")]
    store: std::path::PathBuf,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Migrate objects from a source into an ATLAS volume.
    Run {
        /// Source URI: s3://bucket/prefix, gcs://bucket/prefix,
        /// /local/path, or git-lfs://repo-url#ref
        source: String,
        /// Target ATLAS volume name.
        #[arg(long, default_value = "default")]
        volume: String,
        /// Number of parallel transfer workers.
        #[arg(long, default_value_t = 8)]
        concurrency: usize,
        /// Skip objects already present in ATLAS.
        #[arg(long, default_value_t = true)]
        skip_existing: bool,
        /// Verify BLAKE3 hash after each transfer.
        #[arg(long, default_value_t = true)]
        verify: bool,
    },
    /// List objects available at a source without transferring.
    List {
        source: String,
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
}

fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let args = Args::parse();
    match args.cmd {
        Cmd::Run { source, volume, concurrency, skip_existing, verify } => {
            let src = parse_source(&source).map_err(|e| anyhow::anyhow!(e))?;
            println!("Migrating from {} ({}) → volume '{volume}'", source, src.kind());
            let fs = open_or_init(&args.store)?;
            let config = MigrationConfig { source: src, target_volume: volume, concurrency, skip_existing, verify };
            let (_results, stats) = run(&config, &fs);
            println!("Done: {} transferred, {} skipped, {} failed ({:.1}% success)",
                stats.objects_transferred, stats.objects_skipped, stats.objects_failed,
                stats.success_rate() * 100.0);
        }
        Cmd::List { source, limit } => {
            let src = parse_source(&source).map_err(|e| anyhow::anyhow!(e))?;
            let objects = atlas_migrate::enumerate(&src, limit);
            for obj in &objects {
                println!("{:>12}  {}", obj.size, obj.path);
            }
            println!("({} objects)", objects.len());
        }
    }
    Ok(())
}

fn open_or_init(store: &std::path::Path) -> Result<Fs> {
    if store.join("config.bin").exists() {
        Fs::open(store).map_err(|e| anyhow::anyhow!("open store: {e}"))
    } else {
        Fs::init(store).map_err(|e| anyhow::anyhow!("init store: {e}"))
    }
}
