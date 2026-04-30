//! `atlas-backup` CLI (T7.2).

use anyhow::Result;
use atlas_backup::{ExportConfig, ReplicationConfig, ReplicationTarget, Replicator};
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
            let _cfg = ExportConfig {
                commit_hash: atlas_core::Hash::ZERO,
                dest: out.clone(),
                compress,
                verify: true,
                bandwidth_limit: 0,
            };
            println!("Exporting snapshot to {} (compress={})", out.display(), compress);
            // Real: open Fs, iterate chunks, call BundleWriter.
            println!("Export complete (stub — connect atlas_fs to BundleWriter).");
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
            println!("Backup chain status: (stub — read from {}/backup-chain.json)", args.store.display());
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
