//! `atlas-backup` CLI (T7.2).

use anyhow::Result;
use atlas_backup::{BackupChain, BundleWriter, ExportConfig, ReplicationConfig, ReplicationTarget, Replicator};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "atlas-backup", version, about = "ATLAS backup and replication tool")]
struct Args {
    #[arg(long, env = "ATLAS_STORE")]
    store: PathBuf,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Export a snapshot to an .atlas-bundle file.
    Export {
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        commit: Option<String>,
        #[arg(long, default_value_t = true)]
        compress: bool,
    },
    /// Replicate the latest bundle to a remote target.
    Replicate {
        /// s3://bucket/prefix  or  atlas://host/volume  or  /local/path
        target: String,
        #[arg(long)]
        bundle: PathBuf,
    },
    /// Show the incremental backup chain in the store.
    Status,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let args = Args::parse();
    match args.cmd {
        Cmd::Export { out, commit: _, compress } => {
            let cfg = ExportConfig {
                commit_hash: atlas_core::Hash::ZERO,
                dest: out.clone(),
                compress,
                verify: true,
                bandwidth_limit: 0,
            };
            tracing::info!(dest = %out.display(), compress, "opening store for export");
            let fs = atlas_fs::Fs::open(&args.store)?;
            let file = std::fs::File::create(&out)
                .map_err(|e| anyhow::anyhow!("create {}: {e}", out.display()))?;
            let mut writer = BundleWriter::new(std::io::BufWriter::new(file), cfg)?;
            for hash_res in fs.chunks().iter_hashes() {
                let hash = hash_res.map_err(|e| anyhow::anyhow!("chunk iter: {e}"))?;
                let data = fs.chunks().get(&hash)
                    .map_err(|e| anyhow::anyhow!("chunk get {}: {e}", hash.to_hex()))?;
                writer.write_chunk(&hash, &data)?;
            }
            let stats = writer.finish()?;
            println!(
                "Exported {} chunk(s) ({} bytes raw, {:.2}x compression) to {}",
                stats.chunks_written,
                stats.bytes_written,
                stats.compression_ratio(),
                out.display()
            );
        }
        Cmd::Replicate { target, bundle } => {
            let rt = parse_target(&target)?;
            let cfg = ReplicationConfig { targets: vec![rt], ..Default::default() };
            let rep = Replicator::new(cfg);
            let results = rep.replicate(&bundle);
            for r in results {
                if r.success {
                    println!("  [OK] {} ({} bytes)", r.target, r.bytes_transferred);
                } else {
                    eprintln!("  [FAIL] {}: {}", r.target, r.error.unwrap_or_default());
                }
            }
        }
        Cmd::Status => {
            let chain_path = args.store.join("backup-chain.json");
            if !chain_path.exists() {
                println!("No backup chain found at {}", chain_path.display());
                println!("Run `atlas-backup export --out <file>` to create the first backup.");
            } else {
                let raw = std::fs::read_to_string(&chain_path)
                    .map_err(|e| anyhow::anyhow!("read {}: {e}", chain_path.display()))?;
                let chain: BackupChain = serde_json::from_str(&raw)
                    .map_err(|e| anyhow::anyhow!("parse backup chain: {e}"))?;
                println!(
                    "Backup chain: {} backup(s), {} full, {} bytes total",
                    chain.manifests.len(),
                    chain.full_count(),
                    chain.total_bytes()
                );
                for m in &chain.manifests {
                    let short_commit = hex::encode(&m.commit_hash.as_bytes()[..4]);
                    let kind = if m.is_full() { "full" } else { "incr" };
                    println!(
                        "  [{}] id={} commit={} chunks={} bytes={}",
                        kind, m.id, short_commit, m.chunk_count, m.byte_count
                    );
                }
            }
        }
    }
    Ok(())
}

fn parse_target(s: &str) -> Result<ReplicationTarget> {
    if s.starts_with("s3://") {
        let rest = &s[5..];
        let (bucket, prefix) = rest.split_once('/').unwrap_or((rest, ""));
        Ok(ReplicationTarget::S3 {
            endpoint: "https://s3.amazonaws.com".into(),
            bucket: bucket.into(),
            prefix: prefix.into(),
            region: "us-east-1".into(),
        })
    } else if s.starts_with("atlas://") {
        let rest = &s[8..];
        let (host, vol) = rest.split_once('/').unwrap_or((rest, "default"));
        Ok(ReplicationTarget::AtlasCluster {
            endpoint: host.into(),
            volume: vol.into(),
        })
    } else {
        Ok(ReplicationTarget::LocalPath { path: s.into() })
    }
}
