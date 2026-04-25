//! `atlasctl` — command-line interface to a local ATLAS store.
//!
//! Phase 0+1 surface. The full subcommand catalog (semantic search,
//! lineage, policy, MCP serve, tier, doctor) lands in later phases per
//! [`ATLAS_implementation_plan.md`](../../../ATLAS_implementation_plan.md).

use anyhow::{anyhow, Context, Result};
use atlas_core::{Author, Hash, ObjectKind};
use atlas_fs::Fs;
use atlas_indexer::HybridQuery;
use atlas_ingest::{policy::AllowAll, Ingester};
use atlas_version::{Change, Version};
use clap::{Parser, Subcommand};
use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;

fn parse_kv(s: &str) -> std::result::Result<(String, String), String> {
    s.split_once('=')
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .ok_or_else(|| format!("expected key=value, got {s:?}"))
}

#[derive(Parser, Debug)]
#[command(name = "atlasctl", version, about = "ATLAS filesystem CLI", long_about = None)]
struct Cli {
    /// Path to the ATLAS store on disk. Defaults to $ATLAS_STORE or ./.atlas-store.
    #[arg(long, global = true, env = "ATLAS_STORE")]
    store: Option<PathBuf>,

    /// Increase log verbosity.
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Initialize a new ATLAS store at the configured path.
    Init,

    /// Show metadata for a path.
    Stat { path: String },

    /// List the contents of a directory.
    Ls {
        #[arg(default_value = "/")]
        path: String,
    },

    /// Read a file's contents to stdout.
    Cat { path: String },

    /// Write stdin (or a host file) to a path inside the store.
    Put {
        /// ATLAS path (must start with `/`).
        path: String,
        /// Source host file. If omitted, reads stdin.
        #[arg(long)]
        from: Option<PathBuf>,
    },

    /// Copy a host file into the store.
    Cp {
        host_path: PathBuf,
        atlas_path: String,
    },

    /// Move/rename within the store.
    Mv { from: String, to: String },

    /// Remove a file or empty directory.
    Rm {
        path: String,
        /// Allow recursive removal of non-empty directories.
        #[arg(long)]
        recursive: bool,
    },

    /// Create an empty directory.
    Mkdir { path: String },

    /// Branch operations.
    #[command(subcommand)]
    Branch(BranchCmd),

    /// Commit current working root.
    Commit {
        #[arg(short, long)]
        message: String,
        /// Author name. Falls back to $ATLAS_AUTHOR_NAME, then "anonymous".
        #[arg(long, env = "ATLAS_AUTHOR_NAME")]
        author_name: Option<String>,
        #[arg(long, env = "ATLAS_AUTHOR_EMAIL")]
        author_email: Option<String>,
    },

    /// Switch HEAD to a branch or detached commit.
    Checkout {
        /// Branch name or commit hash hex.
        target: String,
    },

    /// Show commit log walking back from HEAD.
    Log {
        #[arg(short, long, default_value_t = 20)]
        limit: usize,
    },

    /// Diff two commits (or HEAD~1 vs HEAD if both omitted).
    Diff {
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
    },

    /// Verify integrity of every chunk in the store.
    Verify,

    /// Ingest files into the semantic index.
    Ingest {
        /// ATLAS directory to ingest recursively (default: /).
        #[arg(default_value = "/")]
        path: String,
        /// Index directory (default: <store>/.atlas-index).
        #[arg(long)]
        index_dir: Option<PathBuf>,
        /// URL of the embedder service (optional).
        #[arg(long, env = "ATLAS_EMBEDDER_URL")]
        embedder_url: Option<String>,
    },

    /// Search the semantic index.
    Find {
        /// Keyword query (Tantivy syntax). May be combined with --near.
        #[arg(long, short)]
        query: Option<String>,
        /// Path to a file whose embedding is used as the vector query.
        #[arg(long)]
        near: Option<String>,
        /// Xattr filter key=value (repeatable).
        #[arg(long, value_parser = parse_kv)]
        filter: Vec<(String, String)>,
        /// Maximum results to return.
        #[arg(long, short, default_value_t = 10)]
        limit: usize,
        /// Weight given to the vector score (0 = text only, 1 = vector only).
        #[arg(long, default_value_t = 0.5)]
        vector_weight: f32,
        /// Index directory (default: <store>/.atlas-index).
        #[arg(long)]
        index_dir: Option<PathBuf>,
        /// URL of the embedder service (for --near queries).
        #[arg(long, env = "ATLAS_EMBEDDER_URL")]
        embedder_url: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
enum BranchCmd {
    /// Create a new branch at the current HEAD commit.
    Create { name: String },
    /// List all branches.
    List,
    /// Delete a branch (cannot delete the branch HEAD points at).
    Delete { name: String },
}

fn default_store() -> PathBuf {
    PathBuf::from("./.atlas-store")
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    let store_path = cli.store.unwrap_or_else(default_store);
    match cli.cmd {
        Cmd::Init => cmd_init(&store_path),
        Cmd::Stat { path } => cmd_stat(&store_path, &path),
        Cmd::Ls { path } => cmd_ls(&store_path, &path),
        Cmd::Cat { path } => cmd_cat(&store_path, &path),
        Cmd::Put { path, from } => cmd_put(&store_path, &path, from.as_deref()),
        Cmd::Cp {
            host_path,
            atlas_path,
        } => cmd_cp(&store_path, &host_path, &atlas_path),
        Cmd::Mv { from, to } => cmd_mv(&store_path, &from, &to),
        Cmd::Rm { path, recursive } => cmd_rm(&store_path, &path, recursive),
        Cmd::Mkdir { path } => cmd_mkdir(&store_path, &path),
        Cmd::Branch(b) => cmd_branch(&store_path, b),
        Cmd::Commit {
            message,
            author_name,
            author_email,
        } => cmd_commit(&store_path, &message, author_name, author_email),
        Cmd::Checkout { target } => cmd_checkout(&store_path, &target),
        Cmd::Log { limit } => cmd_log(&store_path, limit),
        Cmd::Diff { from, to } => cmd_diff(&store_path, from.as_deref(), to.as_deref()),
        Cmd::Verify => cmd_verify(&store_path),
        Cmd::Ingest {
            path,
            index_dir,
            embedder_url,
        } => cmd_ingest(&store_path, &path, index_dir, embedder_url.as_deref()),
        Cmd::Find {
            query,
            near,
            filter,
            limit,
            vector_weight,
            index_dir,
            embedder_url,
        } => cmd_find(
            &store_path,
            query.as_deref(),
            near.as_deref(),
            filter,
            limit,
            vector_weight,
            index_dir,
            embedder_url.as_deref(),
        ),
    }
}

fn init_tracing(verbose: u8) {
    let level = match verbose {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(level)),
        )
        .try_init();
}

fn cmd_init(store: &std::path::Path) -> Result<()> {
    Fs::init(store).context("init store")?;
    println!("initialized ATLAS store at {}", store.display());
    Ok(())
}

fn cmd_stat(store: &std::path::Path, path: &str) -> Result<()> {
    let fs = Fs::open(store)?;
    let e = fs.stat(path)?;
    println!("path:    {}", e.path);
    println!("kind:    {}", e.kind);
    println!("hash:    {}", e.hash);
    println!("size:    {}", e.size);
    Ok(())
}

fn cmd_ls(store: &std::path::Path, path: &str) -> Result<()> {
    let fs = Fs::open(store)?;
    for e in fs.list(path)? {
        let mark = match e.kind {
            ObjectKind::Dir => "/",
            ObjectKind::Symlink => "@",
            _ => "",
        };
        let name = e
            .path
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("/");
        println!("{:>10}  {}  {}{}", e.size, e.hash.short(), name, mark);
    }
    Ok(())
}

fn cmd_cat(store: &std::path::Path, path: &str) -> Result<()> {
    let fs = Fs::open(store)?;
    let bytes = fs.read(path)?;
    use std::io::Write;
    std::io::stdout().write_all(&bytes)?;
    Ok(())
}

fn cmd_put(store: &std::path::Path, path: &str, from: Option<&std::path::Path>) -> Result<()> {
    let fs = Fs::open(store)?;
    let bytes = match from {
        Some(p) => std::fs::read(p)?,
        None => {
            let mut buf = Vec::new();
            std::io::stdin().read_to_end(&mut buf)?;
            buf
        }
    };
    let e = fs.write(path, &bytes)?;
    println!("{} {} {} bytes", e.hash.short(), e.path, e.size);
    Ok(())
}

fn cmd_cp(store: &std::path::Path, host: &std::path::Path, atlas: &str) -> Result<()> {
    let fs = Fs::open(store)?;
    let bytes = std::fs::read(host)?;
    let e = fs.write(atlas, &bytes)?;
    println!(
        "{} -> {} ({} bytes, {})",
        host.display(),
        e.path,
        e.size,
        e.hash.short()
    );
    Ok(())
}

fn cmd_mv(store: &std::path::Path, from: &str, to: &str) -> Result<()> {
    let fs = Fs::open(store)?;
    fs.rename(from, to)?;
    Ok(())
}

fn cmd_rm(store: &std::path::Path, path: &str, recursive: bool) -> Result<()> {
    let fs = Fs::open(store)?;
    if recursive {
        rm_recursive(&fs, path)?;
    } else {
        fs.delete(path)?;
    }
    Ok(())
}

fn rm_recursive(fs: &Fs, path: &str) -> Result<()> {
    let entry = fs.stat(path)?;
    if entry.kind == ObjectKind::Dir {
        // Snapshot then remove children — listing is invalidated as we go.
        let kids: Vec<_> = fs.list(path)?.into_iter().map(|e| e.path).collect();
        for k in kids {
            rm_recursive(fs, &k)?;
        }
    }
    fs.delete(path)?;
    Ok(())
}

fn cmd_mkdir(store: &std::path::Path, path: &str) -> Result<()> {
    Fs::open(store)?.mkdir(path)?;
    Ok(())
}

fn cmd_branch(store: &std::path::Path, b: BranchCmd) -> Result<()> {
    let fs = Fs::open(store)?;
    let v = Version::new(&fs);
    match b {
        BranchCmd::Create { name } => {
            let br = v.branch_create(&name, None)?;
            println!("created branch '{}' at {}", br.name, br.head.short());
        }
        BranchCmd::List => {
            let head = v.head().ok();
            let head_branch = match head {
                Some(atlas_object::HeadState::Branch(n)) => Some(n),
                _ => None,
            };
            for b in v.branch_list()? {
                let marker = if Some(&b.name) == head_branch.as_ref() {
                    "* "
                } else {
                    "  "
                };
                println!("{marker}{} {}", b.name, b.head.short());
            }
        }
        BranchCmd::Delete { name } => {
            v.branch_delete(&name)?;
            println!("deleted branch '{name}'");
        }
    }
    Ok(())
}

fn cmd_commit(
    store: &std::path::Path,
    message: &str,
    name: Option<String>,
    email: Option<String>,
) -> Result<()> {
    let fs = Fs::open(store)?;
    let v = Version::new(&fs);
    let author = Author::new(
        name.unwrap_or_else(|| "anonymous".into()),
        email.unwrap_or_else(|| "anon@atlas".into()),
    );
    let h = v.commit(author, message)?;
    println!("commit {}", h);
    Ok(())
}

fn cmd_checkout(store: &std::path::Path, target: &str) -> Result<()> {
    let fs = Fs::open(store)?;
    let v = Version::new(&fs);
    // Try as a branch name first; fall back to commit hash.
    if v.branch_list()?.iter().any(|b| b.name == target) {
        v.checkout_branch(target)?;
        println!("HEAD -> {}", target);
    } else {
        let h =
            Hash::from_hex(target).map_err(|_| anyhow!("not a branch or commit hash: {target}"))?;
        v.checkout_commit(h)?;
        println!("HEAD detached at {}", h.short());
    }
    Ok(())
}

fn cmd_log(store: &std::path::Path, limit: usize) -> Result<()> {
    let fs = Fs::open(store)?;
    let v = Version::new(&fs);
    for c in v.log(None, limit)? {
        println!("commit {}", c.hash);
        println!("Author: {} <{}>", c.author.name, c.author.email);
        println!("Date:   {}", c.timestamp);
        println!();
        for line in c.message.lines() {
            println!("    {line}");
        }
        println!();
    }
    Ok(())
}

fn cmd_diff(store: &std::path::Path, from: Option<&str>, to: Option<&str>) -> Result<()> {
    let fs = Fs::open(store)?;
    let v = Version::new(&fs);
    let to_hash = match to {
        Some(s) => resolve_commitish(&fs, s)?,
        None => v.head_commit()?,
    };
    let from_hash = match from {
        Some(s) => resolve_commitish(&fs, s)?,
        None => {
            // HEAD~1 — first parent of HEAD.
            let h = v.head_commit()?;
            let c = fs
                .meta()
                .get_commit(&h)?
                .ok_or_else(|| anyhow!("HEAD commit missing"))?;
            *c.parents.first().unwrap_or(&h)
        }
    };
    for ch in v.diff_commits(from_hash, to_hash)? {
        match ch {
            Change::Added { path, .. } => println!("A {path}"),
            Change::Removed { path, .. } => println!("D {path}"),
            Change::Modified { path, .. } => println!("M {path}"),
        }
    }
    Ok(())
}

/// Resolve a string to a commit hash. Accepts either a 64-hex commit hash
/// or a branch name.
fn resolve_commitish(fs: &Fs, s: &str) -> Result<Hash> {
    if let Ok(h) = Hash::from_hex(s) {
        return Ok(h);
    }
    if let Some(b) = fs.meta().get_branch(s)? {
        return Ok(b.head);
    }
    Err(anyhow!("not a commit hash or branch: {s}"))
}

fn cmd_verify(store: &std::path::Path) -> Result<()> {
    let fs = Fs::open(store)?;
    use atlas_chunk::ChunkStore;
    let chunks = atlas_chunk::LocalChunkStore::open(store.join("chunks"))?;
    let hashes = chunks.iter_hashes()?;
    let n = hashes.len();
    let mut failures = 0usize;
    for h in hashes {
        if let Err(e) = chunks.verify(&h) {
            failures += 1;
            eprintln!("FAIL {h}: {e}");
        }
    }
    println!("verified {n} chunks, {failures} failure(s)");
    if failures > 0 {
        std::process::exit(2);
    }
    let _ = fs;
    Ok(())
}

fn default_index_dir(store: &std::path::Path) -> PathBuf {
    store.join(".atlas-index")
}

fn cmd_ingest(
    store: &std::path::Path,
    path: &str,
    index_dir: Option<PathBuf>,
    embedder_url: Option<&str>,
) -> Result<()> {
    let fs = Fs::open(store).context("open store")?;
    let idx = index_dir.unwrap_or_else(|| default_index_dir(store));
    let mut ingester =
        Ingester::open(&idx, embedder_url).map_err(|e| anyhow!("open index: {e}"))?;
    let count = ingester
        .ingest_tree(&fs, path, &AllowAll)
        .map_err(|e| anyhow!("ingest: {e}"))?;
    println!("indexed {count} file(s) under {path}");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn cmd_find(
    store: &std::path::Path,
    query: Option<&str>,
    near: Option<&str>,
    filter: Vec<(String, String)>,
    limit: usize,
    vector_weight: f32,
    index_dir: Option<PathBuf>,
    embedder_url: Option<&str>,
) -> Result<()> {
    let idx = index_dir.unwrap_or_else(|| default_index_dir(store));
    let ingester = Ingester::open(&idx, embedder_url).map_err(|e| anyhow!("open index: {e}"))?;

    // Build vector query by embedding the `--near` file if given.
    let embedding: Option<Vec<f32>> = match near {
        Some(atlas_path) => {
            let fs = Fs::open(store).context("open store")?;
            let bytes = fs.read(atlas_path).context("read near file")?;
            let text = atlas_ingest::formats::extract_text(atlas_path, &bytes);
            match &ingester.embedder {
                Some(client) => match client.embed(&text) {
                    Ok(r) => Some(r.embedding),
                    Err(e) => {
                        eprintln!("warn: embedder unavailable for --near: {e}");
                        None
                    }
                },
                None => {
                    eprintln!("warn: --near requires --embedder-url");
                    None
                }
            }
        }
        None => None,
    };

    let xattr_filters: HashMap<String, String> = filter.into_iter().collect();
    let q = HybridQuery {
        text: query.map(|s| s.to_string()),
        embedding,
        xattr_filters,
        limit,
        vector_weight,
    };

    let results = ingester.search(&q).map_err(|e| anyhow!("search: {e}"))?;
    if results.is_empty() {
        println!("(no results)");
        return Ok(());
    }
    println!("{:<8} {:<10} path", "score", "hash");
    println!("{}", "-".repeat(60));
    for r in results {
        println!("{:<8.4} {:<10} {}", r.score, r.file_hash.short(), r.path);
    }
    Ok(())
}
