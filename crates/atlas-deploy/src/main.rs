//! Generates deployment scaffolding (systemd units + shell helpers) for
//! a single-node ATLAS storage server or a 3-node CRAQ chain. The
//! command writes plain text files to a chosen output directory; it
//! never touches running infrastructure.
//!
//! Usage:
//!   atlas-deploy single  --out ./deploy --bind 0.0.0.0:7373 --data /var/lib/atlas
//!   atlas-deploy cluster --out ./deploy --nodes node1,node2,node3 --data /var/lib/atlas

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Parser)]
#[command(
    name = "atlas-deploy",
    version,
    about = "Generate ATLAS deployment files"
)]
struct Args {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Single-node deployment: one storage server.
    Single {
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value = "0.0.0.0:7373")]
        bind: String,
        #[arg(long, default_value = "/var/lib/atlas")]
        data: PathBuf,
    },
    /// 3-node CRAQ chain.
    Cluster {
        #[arg(long)]
        out: PathBuf,
        /// Comma-separated host names in head→tail order.
        #[arg(long)]
        nodes: String,
        #[arg(long, default_value = "/var/lib/atlas")]
        data: PathBuf,
        #[arg(long, default_value = "7373")]
        port: u16,
    },
}

fn main() -> Result<()> {
    match Args::parse().cmd {
        Cmd::Single { out, bind, data } => emit_single(&out, &bind, &data),
        Cmd::Cluster {
            out,
            nodes,
            data,
            port,
        } => {
            let nodes: Vec<&str> = nodes.split(',').map(|s| s.trim()).collect();
            anyhow::ensure!(
                nodes.len() == 3,
                "cluster expects exactly 3 nodes, got {}",
                nodes.len()
            );
            emit_cluster(&out, &nodes, &data, port)
        }
    }
}

fn emit_single(out: &Path, bind: &str, data: &Path) -> Result<()> {
    fs::create_dir_all(out).with_context(|| format!("create {}", out.display()))?;
    let chunks = data.join("chunks");
    let meta = data.join("meta");

    let unit = format!(
        "[Unit]\n\
         Description=ATLAS storage server\n\
         After=network.target\n\
         \n\
         [Service]\n\
         ExecStartPre=/bin/mkdir -p {chunks} {meta}\n\
         ExecStart=/usr/local/bin/atlas-storage \\\n  \
             --bind {bind} \\\n  \
             --chunks-dir {chunks} \\\n  \
             --meta-dir {meta}\n\
         Restart=on-failure\n\
         RestartSec=2s\n\
         LimitNOFILE=1048576\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        chunks = chunks.display(),
        meta = meta.display(),
        bind = bind,
    );
    fs::write(out.join("atlas-storage.service"), unit)?;

    let install = format!(
        "#!/usr/bin/env bash\nset -euo pipefail\n\
         install -m 0755 atlas-storage /usr/local/bin/atlas-storage\n\
         install -m 0644 atlas-storage.service /etc/systemd/system/atlas-storage.service\n\
         mkdir -p {chunks} {meta}\n\
         systemctl daemon-reload\n\
         systemctl enable --now atlas-storage\n",
        chunks = chunks.display(),
        meta = meta.display(),
    );
    fs::write(out.join("install.sh"), install)?;

    println!(
        "wrote single-node unit + install script to {}",
        out.display()
    );
    Ok(())
}

fn emit_cluster(out: &Path, nodes: &[&str], data: &Path, port: u16) -> Result<()> {
    fs::create_dir_all(out)?;
    for (i, host) in nodes.iter().enumerate() {
        let role = match i {
            0 => "head",
            1 => "middle",
            _ => "tail",
        };
        let unit = format!(
            "[Unit]\n\
             Description=ATLAS storage ({role}) for {host}\n\
             After=network.target\n\
             \n\
             [Service]\n\
             Environment=ATLAS_ROLE={role}\n\
             ExecStartPre=/bin/mkdir -p {chunks} {meta}\n\
             ExecStart=/usr/local/bin/atlas-storage \\\n  \
                 --bind 0.0.0.0:{port} \\\n  \
                 --chunks-dir {chunks} \\\n  \
                 --meta-dir {meta}\n\
             Restart=on-failure\n\
             RestartSec=2s\n\
             LimitNOFILE=1048576\n\
             \n\
             [Install]\n\
             WantedBy=multi-user.target\n",
            role = role,
            host = host,
            port = port,
            chunks = data.join("chunks").display(),
            meta = data.join("meta").display(),
        );
        fs::write(out.join(format!("atlas-{host}.service")), unit)?;
    }

    let chain_csv: Vec<String> = nodes.iter().map(|n| format!("{n}:{port}")).collect();
    let chain_env = format!("ATLAS_CHAIN={}\n", chain_csv.join(","));
    fs::write(out.join("chain.env"), chain_env)?;

    let install = format!(
        "#!/usr/bin/env bash\n\
         set -euo pipefail\n\
         # Push the unit and binary to each host with scp/ssh; this is a\n\
         # template — adapt to your fleet automation.\n\
         NODES=({})\n\
         for n in \"${{NODES[@]}}\"; do\n  \
             scp atlas-storage \"$n\":/tmp/atlas-storage\n  \
             scp \"atlas-$n.service\" \"$n\":/tmp/\n  \
             ssh \"$n\" 'sudo install -m 0755 /tmp/atlas-storage /usr/local/bin/atlas-storage \\\n    \
                 && sudo install -m 0644 /tmp/atlas-'\"$n\"'.service /etc/systemd/system/atlas-storage.service \\\n    \
                 && sudo systemctl daemon-reload \\\n    \
                 && sudo systemctl enable --now atlas-storage'\n\
         done\n",
        nodes.join(" ")
    );
    fs::write(out.join("install.sh"), install)?;

    println!(
        "wrote 3-node cluster scaffolding ({} → {} → {}) to {}",
        nodes[0],
        nodes[1],
        nodes[2],
        out.display()
    );
    Ok(())
}
