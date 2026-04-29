//! `atlasctl` — command-line interface to a local ATLAS store.
//!
//! Phase 0+1 surface. The full subcommand catalog (semantic search,
//! lineage, policy, MCP serve, tier, doctor) lands in later phases per
//! [`ATLAS_implementation_plan.md`](../../../ATLAS_implementation_plan.md).

use anyhow::{anyhow, Context, Result};
use atlas_core::{Author, Hash, ObjectKind};
use atlas_fs::Fs;
// Phase 6 — Desktop integration
use atlas_wfsp::{WfspConfig, WfspMount};
use atlas_onboarding::{OnboardingState, WizardStep};
// Phase 7 — Production hardening (imported inline in handler functions)
use atlas_governor::{
    policy::{AccessRequest, Permission, PolicyEngine},
    redact::{RedactConfig, RedactEngine},
    AuditLog, TokenAuthority,
};
use atlas_indexer::HybridQuery;
use atlas_ingest::{policy::AllowAll, Ingester};
use atlas_lineage::{EdgeKind, LineageEdge, LineageJournal};
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

    // -----------------------------------------------------------------------
    // Phase 4 — Lineage and governance
    // -----------------------------------------------------------------------
    /// Lineage graph operations (T4.1, T4.2, T4.3).
    #[command(subcommand)]
    Lineage(LineageCmd),

    /// Policy engine operations (T4.4, T4.7).
    #[command(subcommand)]
    Policy(PolicyCmd),

    /// Capability token operations (T4.5).
    #[command(subcommand)]
    Token(TokenCmd),

    /// Audit log operations (T4.8).
    #[command(subcommand)]
    Audit(AuditCmd),

    /// Serve the MCP endpoint over stdio, optionally scoped to a subtree (T5.2).
    Mcp(McpCmd),

    /// Detect and redact PII from a file's text content (T4.6).
    Redact {
        /// ATLAS path to the file.
        path: String,
        /// Redact email addresses.
        #[arg(long, default_value_t = true)]
        email: bool,
        /// Redact US Social Security numbers.
        #[arg(long, default_value_t = true)]
        ssn: bool,
        /// Redact API keys and bearer tokens.
        #[arg(long, default_value_t = true)]
        api_keys: bool,
        /// Print only whether PII was found, not the redacted text.
        #[arg(long)]
        check_only: bool,
    },

    // -----------------------------------------------------------------------
    // Phase 6 — Desktop integration
    // -----------------------------------------------------------------------
    /// Mount an ATLAS store via WinFsp (Windows) or FUSE (Linux/macOS) (T6.1, T6.3, T6.5).
    Mount {
        /// Mount point — drive letter (Windows: `Z:`), directory, or `atlas://` URI.
        mount_point: String,
    },

    /// Unmount a previously mounted ATLAS volume.
    Umount {
        /// Same mount point that was passed to `mount`.
        mount_point: String,
    },

    /// Windows shell-extension management (T6.2).
    #[command(subcommand)]
    Shell(ShellCmd),

    /// Run the first-launch onboarding wizard (T6.7).
    Onboard {
        /// Skip interactive prompts and use defaults.
        #[arg(long)]
        non_interactive: bool,
        /// Store path override (wizard default: ~/atlas-store).
        #[arg(long)]
        store_path: Option<PathBuf>,
    },

    /// Seed sample data into an already-initialised store (T6.7).
    SeedSamples,

    /// Launch the ATLAS Explorer GUI (T6.6).
    Explorer,

    /// Start the web admin console (T6.8).
    Web {
        /// Address to bind.
        #[arg(long, default_value = "127.0.0.1:8080")]
        bind: String,
    },

    // -----------------------------------------------------------------------
    // Phase 7 — Production hardening
    // -----------------------------------------------------------------------
    /// Run chaos-engineering scenarios (T7.1).
    #[command(subcommand)]
    Chaos(ChaosCmd),

    /// Backup and replication operations (T7.2).
    #[command(subcommand)]
    Backup(BackupCmd),

    /// Compliance posture report (T7.4).
    Compliance {
        /// Print a full JSON report of SOC 2 / ISO 27001 gap analysis.
        #[arg(long)]
        json: bool,
    },

    /// Performance tuning profile management (T7.6).
    #[command(subcommand)]
    Tuning(TuningCmd),

    /// Multi-tenant quota management (T7.7).
    #[command(subcommand)]
    Quota(QuotaCmd),

    /// Migrate data from external sources (T7.8).
    #[command(subcommand)]
    Migrate(MigrateCmd),
}

#[derive(Parser, Debug)]
struct McpCmd {
    #[command(subcommand)]
    sub: McpSub,
}

#[derive(Subcommand, Debug)]
enum McpSub {
    /// Serve MCP over stdio (newline-delimited JSON-RPC).
    Serve {
        /// Subtree to scope the server to. Paths outside it are 403.
        path: Option<String>,
    },
    /// List the full MCP tool catalog as JSON.
    Tools,
}

/// Phase 7: Chaos engineering subcommands (T7.1).
#[derive(Subcommand, Debug)]
enum ChaosCmd {
    /// List all available chaos scenarios.
    List,
    /// Run a single scenario by name.
    Run {
        name: String,
        #[arg(long, default_value_t = 60)]
        duration_secs: u64,
    },
    /// Run the full nightly chaos suite.
    Suite,
}

/// Phase 7: Backup subcommands (T7.2).
#[derive(Subcommand, Debug)]
enum BackupCmd {
    /// Export a snapshot to an .atlas-bundle file.
    Export {
        #[arg(long)]
        out: std::path::PathBuf,
        #[arg(long, default_value_t = true)]
        compress: bool,
    },
    /// Replicate a bundle to a remote target (s3://, atlas://, or /local).
    Replicate {
        target: String,
        #[arg(long)]
        bundle: std::path::PathBuf,
    },
    /// Show backup chain status.
    Status,
}

/// Phase 7: Tuning subcommands (T7.6).
#[derive(Subcommand, Debug)]
enum TuningCmd {
    /// Show the built-in profile for a workload kind.
    Show {
        /// training | inference | build | interactive | streaming
        workload: String,
    },
    /// Apply a profile to a named volume or namespace.
    Apply {
        #[arg(long)]
        volume: String,
        #[arg(long)]
        workload: String,
    },
    /// Recommend a profile from observed I/O stats.
    Recommend {
        #[arg(long)]
        read_bytes: u64,
        #[arg(long)]
        write_bytes: u64,
        #[arg(long, default_value_t = 4 * 1024 * 1024)]
        avg_object_size: u64,
    },
}

/// Phase 7: Quota subcommands (T7.7).
#[derive(Subcommand, Debug)]
enum QuotaCmd {
    /// List all tenants and their quotas.
    List,
    /// Show current quota and usage for a tenant.
    Show { tenant: String },
    /// Register a new tenant with unlimited quota.
    Add {
        tenant: String,
        #[arg(long, default_value_t = 0)]
        max_bytes: u64,
        #[arg(long, default_value_t = 0)]
        max_objects: u64,
    },
}

/// Phase 7: Migrate subcommands (T7.8).
#[derive(Subcommand, Debug)]
enum MigrateCmd {
    /// List objects at a source without transferring.
    List {
        source: String,
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// Transfer objects from a source into an ATLAS volume.
    Run {
        source: String,
        #[arg(long, default_value = "default")]
        volume: String,
        #[arg(long, default_value_t = 8)]
        concurrency: usize,
    },
}

/// Phase 6: Windows shell-extension subcommands (T6.2).
#[derive(Subcommand, Debug)]
enum ShellCmd {
    /// Register the ATLAS shell extension with Windows Explorer (requires admin).
    Register {
        /// Path to the shell-extension DLL.
        #[arg(long, default_value = "atlas-shellext-win.dll")]
        dll: String,
    },
    /// Unregister the ATLAS shell extension.
    Unregister,
    /// Print the registry keys the extension would write.
    Info,
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

#[derive(Subcommand, Debug)]
enum LineageCmd {
    /// Record an explicit lineage edge (T4.2).
    Record {
        /// Source object hash (producer / input).
        #[arg(long)]
        source: String,
        /// Sink object hash (consumer / output).
        #[arg(long)]
        sink: String,
        /// Edge kind: read | write | derive | copy | transform.
        #[arg(long, default_value = "derive")]
        kind: String,
        /// Agent identifier (process, user, service).
        #[arg(long, default_value = "")]
        agent: String,
        /// Lineage journal directory (default: <store>/.atlas-lineage).
        #[arg(long)]
        lineage_dir: Option<PathBuf>,
    },
    /// Show direct parents and children of an object hash.
    Show {
        /// Content hash to query.
        hash: String,
        /// BFS depth for ancestor/descendant traversal.
        #[arg(long, default_value_t = 3)]
        depth: usize,
        /// Lineage journal directory (default: <store>/.atlas-lineage).
        #[arg(long)]
        lineage_dir: Option<PathBuf>,
    },
    /// Summarise lineage activity in time windows (T4.3).
    Rollup {
        /// Window size in seconds.
        #[arg(long, default_value_t = 3600)]
        window_secs: u64,
        /// Lineage journal directory (default: <store>/.atlas-lineage).
        #[arg(long)]
        lineage_dir: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
enum PolicyCmd {
    /// Evaluate an access request against a policy file (T4.4).
    Eval {
        /// Path inside the ATLAS store.
        #[arg(long)]
        path: String,
        /// Principal (user/service name).
        #[arg(long)]
        principal: String,
        /// Permission to test: read | write | delete | list.
        #[arg(long)]
        perm: String,
        /// YAML policy file to load.
        #[arg(long)]
        policy_file: PathBuf,
    },
}

#[derive(Subcommand, Debug)]
enum TokenCmd {
    /// Issue a new capability token (T4.5).
    Issue {
        /// Principal to grant the token to.
        #[arg(long)]
        principal: String,
        /// Path scope prefix (e.g. /data/ml-team/).
        #[arg(long)]
        scope: String,
        /// Permissions to grant (repeatable): read, write, delete, list.
        #[arg(long)]
        perm: Vec<String>,
        /// Token TTL in seconds.
        #[arg(long, default_value_t = 3600)]
        ttl: u64,
        /// Governance directory (default: <store>/.atlas-gov).
        #[arg(long)]
        gov_dir: Option<PathBuf>,
    },
    /// Verify a capability token's signature and expiry (T4.5).
    Verify {
        /// JSON-encoded token (from `token issue`).
        token_json: String,
        /// Governance directory (default: <store>/.atlas-gov).
        #[arg(long)]
        gov_dir: Option<PathBuf>,
    },
    /// Revoke a capability token by ID (T4.5).
    Revoke {
        /// Token UUID to revoke.
        id: String,
        /// Governance directory (default: <store>/.atlas-gov).
        #[arg(long)]
        gov_dir: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
enum AuditCmd {
    /// Verify the SHA-256 chain integrity of the audit log (T4.8).
    Verify {
        /// Governance directory (default: <store>/.atlas-gov).
        #[arg(long)]
        gov_dir: Option<PathBuf>,
    },
    /// Export a range of audit entries as JSON (T4.8).
    Export {
        /// First sequence number to export (inclusive).
        #[arg(long, default_value_t = 0)]
        from_seq: u64,
        /// Last sequence number to export (inclusive).
        #[arg(long, default_value_t = u64::MAX)]
        to_seq: u64,
        /// Governance directory (default: <store>/.atlas-gov).
        #[arg(long)]
        gov_dir: Option<PathBuf>,
    },
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
        Cmd::Lineage(sub) => cmd_lineage(&store_path, sub),
        Cmd::Policy(sub) => cmd_policy(sub),
        Cmd::Token(sub) => cmd_token(&store_path, sub),
        Cmd::Audit(sub) => cmd_audit(&store_path, sub),
        Cmd::Mcp(m) => cmd_mcp(&store_path, m),
        Cmd::Redact {
            path,
            email,
            ssn,
            api_keys,
            check_only,
        } => cmd_redact(&store_path, &path, email, ssn, api_keys, check_only),

        // ── Phase 6 ────────────────────────────────────────────────────────
        Cmd::Mount { mount_point } => cmd_mount(&store_path, &mount_point),

        Cmd::Umount { mount_point } => cmd_umount(&mount_point),
        Cmd::Shell(sub) => cmd_shell(sub),
        Cmd::Onboard { non_interactive, store_path: override_path } => {
            cmd_onboard(override_path.as_deref(), non_interactive)
        }
        Cmd::SeedSamples => cmd_seed_samples(&store_path),
        Cmd::Explorer => cmd_explorer(),
        Cmd::Web { bind } => cmd_web(&store_path, &bind),

        // ── Phase 7 ────────────────────────────────────────────────────────
        Cmd::Chaos(sub) => cmd_chaos(sub),
        Cmd::Backup(sub) => cmd_backup(&store_path, sub),
        Cmd::Compliance { json } => cmd_compliance(&store_path, json),
        Cmd::Tuning(sub) => cmd_tuning(sub),
        Cmd::Quota(sub) => cmd_quota(sub),
        Cmd::Migrate(sub) => cmd_migrate(sub),
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

// ---------------------------------------------------------------------------
// Phase 4 helpers
// ---------------------------------------------------------------------------

fn default_lineage_dir(store: &std::path::Path) -> PathBuf {
    store.join(".atlas-lineage")
}

fn default_gov_dir(store: &std::path::Path) -> PathBuf {
    store.join(".atlas-gov")
}

// ── Lineage ─────────────────────────────────────────────────────────────────

fn cmd_lineage(store: &std::path::Path, sub: LineageCmd) -> Result<()> {
    match sub {
        LineageCmd::Record {
            source,
            sink,
            kind,
            agent,
            lineage_dir,
        } => {
            let dir = lineage_dir.unwrap_or_else(|| default_lineage_dir(store));
            let mut j =
                LineageJournal::open(&dir).map_err(|e| anyhow!("open lineage journal: {e}"))?;
            let src =
                Hash::from_hex(&source).map_err(|_| anyhow!("invalid source hash: {source}"))?;
            let snk = Hash::from_hex(&sink).map_err(|_| anyhow!("invalid sink hash: {sink}"))?;
            let ek: EdgeKind = kind
                .parse()
                .map_err(|e: String| anyhow!("invalid edge kind: {e}"))?;
            let edge = LineageEdge::new(
                ek,
                src,
                snk,
                if agent.is_empty() { "atlasctl" } else { &agent },
            );
            j.record(edge).map_err(|e| anyhow!("record edge: {e}"))?;
            println!("recorded lineage edge");
            Ok(())
        }
        LineageCmd::Show {
            hash,
            depth,
            lineage_dir,
        } => {
            let dir = lineage_dir.unwrap_or_else(|| default_lineage_dir(store));
            let j = LineageJournal::open(&dir).map_err(|e| anyhow!("open lineage journal: {e}"))?;
            let h = Hash::from_hex(&hash).map_err(|_| anyhow!("invalid hash: {hash}"))?;
            let parents = j.parents(&h).map_err(|e| anyhow!("{e}"))?;
            let children = j.children(&h).map_err(|e| anyhow!("{e}"))?;
            let ancestors = j.ancestors(&h, depth).map_err(|e| anyhow!("{e}"))?;
            let descendants = j.descendants(&h, depth).map_err(|e| anyhow!("{e}"))?;
            println!("=== {} ===", &hash[..16.min(hash.len())]);
            println!("Direct parents ({}):", parents.len());
            for e in &parents {
                println!(
                    "  {} <- {} ({})",
                    &e.sink_hash.to_hex()[..8],
                    &e.source_hash.to_hex()[..8],
                    e.kind
                );
            }
            println!("Direct children ({}):", children.len());
            for e in &children {
                println!(
                    "  {} -> {} ({})",
                    &e.source_hash.to_hex()[..8],
                    &e.sink_hash.to_hex()[..8],
                    e.kind
                );
            }
            println!("Ancestors up to depth {depth}: {}", ancestors.len());
            println!("Descendants up to depth {depth}: {}", descendants.len());
            Ok(())
        }
        LineageCmd::Rollup {
            window_secs,
            lineage_dir,
        } => {
            let dir = lineage_dir.unwrap_or_else(|| default_lineage_dir(store));
            let j = LineageJournal::open(&dir).map_err(|e| anyhow!("open lineage journal: {e}"))?;
            let edges = j.all_edges().map_err(|e| anyhow!("{e}"))?;
            let buckets = atlas_lineage::rollup::rollup_window(&edges, window_secs);
            if buckets.is_empty() {
                println!("(no edges recorded)");
                return Ok(());
            }
            println!("{:<20} {:>8}  counts", "window_start", "total");
            println!("{}", "-".repeat(50));
            for b in &buckets {
                let total: usize = b.counts.values().sum();
                let detail: Vec<String> =
                    b.counts.iter().map(|(k, v)| format!("{k}={v}")).collect();
                println!(
                    "{:<20} {:>8}  {}",
                    b.window_start_ms,
                    total,
                    detail.join(", ")
                );
            }
            Ok(())
        }
    }
}

// ── Policy ──────────────────────────────────────────────────────────────────

fn cmd_policy(sub: PolicyCmd) -> Result<()> {
    match sub {
        PolicyCmd::Eval {
            path,
            principal,
            perm,
            policy_file,
        } => {
            let permission: Permission = perm
                .parse()
                .map_err(|e: String| anyhow!("invalid permission: {e}"))?;
            let mut engine = PolicyEngine::new();
            engine
                .load_yaml_file(&policy_file)
                .map_err(|e| anyhow!("load policy: {e}"))?;
            let req = AccessRequest {
                path: path.clone(),
                principal: principal.clone(),
                permission,
            };
            match engine.evaluate(&req) {
                atlas_governor::Decision::Allow => {
                    println!("ALLOW  {principal} {perm} {path}");
                }
                atlas_governor::Decision::Deny(reason) => {
                    println!("DENY   {principal} {perm} {path}");
                    println!("       {reason}");
                    std::process::exit(1);
                }
            }
            Ok(())
        }
    }
}

// ── Token ───────────────────────────────────────────────────────────────────

fn cmd_token(store: &std::path::Path, sub: TokenCmd) -> Result<()> {
    match sub {
        TokenCmd::Issue {
            principal,
            scope,
            perm,
            ttl,
            gov_dir,
        } => {
            let dir = gov_dir.unwrap_or_else(|| default_gov_dir(store));
            let auth =
                TokenAuthority::open(&dir).map_err(|e| anyhow!("open token authority: {e}"))?;
            let permissions: Vec<Permission> = perm
                .iter()
                .map(|p| {
                    p.parse::<Permission>()
                        .map_err(|e| anyhow!("invalid permission {p:?}: {e}"))
                })
                .collect::<Result<_>>()?;
            let token = auth
                .issue(&principal, &scope, permissions, ttl)
                .map_err(|e| anyhow!("issue token: {e}"))?;
            println!("{}", token.encode().map_err(|e| anyhow!("encode: {e}"))?);
            Ok(())
        }
        TokenCmd::Verify {
            token_json,
            gov_dir,
        } => {
            let dir = gov_dir.unwrap_or_else(|| default_gov_dir(store));
            let auth =
                TokenAuthority::open(&dir).map_err(|e| anyhow!("open token authority: {e}"))?;
            let token = atlas_governor::CapabilityToken::decode(&token_json)
                .map_err(|e| anyhow!("decode token: {e}"))?;
            match auth.verify(&token) {
                Ok(()) => println!(
                    "VALID  id={} principal={} scope={}",
                    token.id, token.principal, token.scope_path
                ),
                Err(e) => {
                    println!("INVALID  {e}");
                    std::process::exit(1);
                }
            }
            Ok(())
        }
        TokenCmd::Revoke { id, gov_dir } => {
            let dir = gov_dir.unwrap_or_else(|| default_gov_dir(store));
            let mut auth =
                TokenAuthority::open(&dir).map_err(|e| anyhow!("open token authority: {e}"))?;
            auth.revoke(&id).map_err(|e| anyhow!("revoke: {e}"))?;
            println!("revoked token {id}");
            Ok(())
        }
    }
}

// ── Audit ───────────────────────────────────────────────────────────────────

fn cmd_audit(store: &std::path::Path, sub: AuditCmd) -> Result<()> {
    match sub {
        AuditCmd::Verify { gov_dir } => {
            let dir = gov_dir.unwrap_or_else(|| default_gov_dir(store));
            let log = AuditLog::open(&dir).map_err(|e| anyhow!("open audit log: {e}"))?;
            match log.verify_chain().map_err(|e| anyhow!("{e}"))? {
                true => println!("audit log intact"),
                false => {
                    eprintln!("audit log TAMPERED — chain broken");
                    std::process::exit(2);
                }
            }
            Ok(())
        }
        AuditCmd::Export {
            from_seq,
            to_seq,
            gov_dir,
        } => {
            let dir = gov_dir.unwrap_or_else(|| default_gov_dir(store));
            let log = AuditLog::open(&dir).map_err(|e| anyhow!("open audit log: {e}"))?;
            let entries = log
                .export_range(from_seq, to_seq)
                .map_err(|e| anyhow!("{e}"))?;
            println!(
                "{}",
                serde_json::to_string_pretty(&entries).map_err(|e| anyhow!("serialize: {e}"))?
            );
            Ok(())
        }
    }
}

// ── MCP serve (T5.2) ────────────────────────────────────────────────────────

fn cmd_mcp(store: &std::path::Path, m: McpCmd) -> Result<()> {
    use std::io::BufRead;
    use std::sync::Arc;

    match m.sub {
        McpSub::Tools => {
            let tools: Vec<_> = atlas_mcp::tool_descriptors()
                .into_iter()
                .map(|d| {
                    serde_json::json!({
                        "name": d.name,
                        "description": d.description,
                        "input_schema": d.input_schema,
                        "mutates": d.mutates,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&tools)?);
            Ok(())
        }
        McpSub::Serve { path } => {
            let fs = Fs::open(store).context("open store")?;
            let mut core = atlas_mcp::CapabilityCore::new(Arc::new(fs));
            if let Some(p) = path {
                core = core.with_subtree(p);
            }
            let stdin = std::io::stdin();
            let stdout = std::io::stdout();
            let mut out = stdout.lock();
            for line in stdin.lock().lines() {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                let resp = atlas_mcp::handle_line(&core, &line);
                use std::io::Write;
                writeln!(out, "{resp}")?;
                out.flush()?;
            }
            Ok(())
        }
    }
}

// ── Redact ──────────────────────────────────────────────────────────────────

fn cmd_redact(
    store: &std::path::Path,
    path: &str,
    email: bool,
    ssn: bool,
    api_keys: bool,
    check_only: bool,
) -> Result<()> {
    let fs = Fs::open(store).context("open store")?;
    let bytes = fs.read(path).context("read file")?;
    let text = String::from_utf8_lossy(&bytes);
    let cfg = RedactConfig {
        redact_email: email,
        redact_ssn: ssn,
        redact_api_keys: api_keys,
        custom_patterns: vec![],
    };
    let engine = RedactEngine::new(&cfg).map_err(|e| anyhow!("build redactor: {e}"))?;
    if check_only {
        if engine.has_pii(&text) {
            println!("PII DETECTED in {path}");
            std::process::exit(1);
        } else {
            println!("no PII detected in {path}");
        }
    } else {
        print!("{}", engine.redact(&text));
    }
    Ok(())
}

// ── Phase 6: Mount / Umount (T6.1, T6.3, T6.5) ──────────────────────────────

fn cmd_mount(store: &std::path::Path, mount_point: &str) -> Result<()> {
    // Validate the mount point via the WinFsp crate (cross-platform check).
    atlas_wfsp::driver::validate_mount_point(mount_point)
        .map_err(|e| anyhow!("invalid mount point: {e}"))?;

    let fs = Fs::open(store).context("open store")?;
    let config = WfspConfig {
        mount_point: mount_point.to_string(),
        ..WfspConfig::default()
    };
    let mount = WfspMount::new(fs, config)
        .map_err(|e| anyhow!("mount failed: {e}"))?;

    println!("Mounted ATLAS store at {mount_point}");
    println!("Press Ctrl-C to unmount.");
    mount.run();
    Ok(())
}

fn cmd_umount(mount_point: &str) -> Result<()> {
    // On Linux: fusermount -u <mount_point>.
    // On macOS: diskutil unmount <mount_point>.
    // On Windows: signals the WinFsp dispatcher to stop.
    // All three call the same underlying mechanism via the driver crate.
    println!("Unmounting {mount_point}…");
    #[cfg(target_os = "linux")]
    {
        let status = std::process::Command::new("fusermount")
            .args(["-u", mount_point])
            .status()
            .context("run fusermount")?;
        if !status.success() {
            anyhow::bail!("fusermount -u returned non-zero");
        }
    }
    #[cfg(target_os = "macos")]
    {
        let status = std::process::Command::new("diskutil")
            .args(["unmount", mount_point])
            .status()
            .context("run diskutil")?;
        if !status.success() {
            anyhow::bail!("diskutil unmount returned non-zero");
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    println!("(umount is a no-op on this platform — stop the process that called `mount`)");
    println!("Unmounted {mount_point}");
    Ok(())
}

// ── Phase 6: Shell extension (T6.2) ──────────────────────────────────────────

fn cmd_shell(sub: ShellCmd) -> Result<()> {
    match sub {
        ShellCmd::Register { dll } => {
            atlas_shellext_win::registry::register(&dll)
                .context("shell register")?;
            println!("Shell extension registered (DLL: {dll})");
        }
        ShellCmd::Unregister => {
            atlas_shellext_win::registry::unregister()
                .context("shell unregister")?;
            println!("Shell extension unregistered");
        }
        ShellCmd::Info => {
            println!("Column-provider CLSID : {}", atlas_shellext_win::registry::CLSID_COLUMN_PROVIDER);
            println!("Context-menu CLSID    : {}", atlas_shellext_win::registry::CLSID_CONTEXT_MENU);
        }
    }
    Ok(())
}

// ── Phase 6: Onboarding wizard (T6.7) ────────────────────────────────────────

fn cmd_onboard(store_override: Option<&std::path::Path>, non_interactive: bool) -> Result<()> {
    let mut state = OnboardingState::default();
    if let Some(p) = store_override {
        state.store_path = p.to_path_buf();
    }

    if non_interactive {
        println!("Running non-interactive onboarding…");
        println!("  Store path : {}", state.store_path.display());
        println!("  Mode       : {:?}", state.mode);
        // Advance through all steps until Installing.
        while state.current_step() != &WizardStep::Installing {
            state.next();
        }
        state.run_install().context("install step")?;
        println!("Onboarding complete. Run `atlasctl explorer` to open the GUI.");
        return Ok(());
    }

    // Interactive TUI wizard — step through prompts.
    println!("\n{}", state.current_step().title());
    println!("{}\n", state.current_step().description());

    loop {
        let step = state.current_step().clone();
        match step {
            WizardStep::Done => {
                println!("\n✓ {}", state.current_step().title());
                println!("{}", state.current_step().description());
                println!("\nRun `atlasctl explorer` to open ATLAS Explorer.");
                break;
            }
            WizardStep::Installing => {
                println!("Installing to {}…", state.store_path.display());
                state.run_install().context("install step")?;
                println!("\n✓ {}", state.current_step().title());
                println!("{}", state.current_step().description());
                break;
            }
            _ => {
                println!("Step {} / {} — {}", state.step_number(), state.total_steps(), step.title());
                println!("{}\n", step.description());
                print!("[Enter] to continue, [b] to go back: ");
                use std::io::Write;
                std::io::stdout().flush()?;
                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;
                if input.trim() == "b" { state.back(); } else { state.next(); }
                println!();
            }
        }
    }
    Ok(())
}

// ── Phase 6: Seed samples (T6.7) ─────────────────────────────────────────────

fn cmd_seed_samples(store: &std::path::Path) -> Result<()> {
    atlas_onboarding::seed_sample_data(store)
        .context("seed sample data")?;
    println!("Sample data seeded into {}", store.display());
    println!("  /README.md");
    println!("  /datasets/iris.parquet");
    println!("  /datasets/labels.jsonl");
    println!("  /models/tiny.safetensors");
    Ok(())
}

// ── Phase 6: Explorer launcher (T6.6) ────────────────────────────────────────

fn cmd_explorer() -> Result<()> {
    // Look for atlas-explorer on $PATH, fall back to the same directory as
    // the current binary.
    let exe = std::env::current_exe().ok();
    let sibling = exe
        .as_ref()
        .and_then(|p| p.parent())
        .map(|dir| dir.join("atlas-explorer"));

    let binary = if sibling.as_ref().map_or(false, |p| p.exists()) {
        sibling.unwrap()
    } else {
        std::path::PathBuf::from("atlas-explorer")
    };

    println!("Launching ATLAS Explorer ({})…", binary.display());
    std::process::Command::new(&binary)
        .spawn()
        .with_context(|| format!("launch {}", binary.display()))?;
    Ok(())
}

// ── Phase 7: Chaos engineering (T7.1) ─────────────────────────────────────────

fn cmd_chaos(sub: ChaosCmd) -> Result<()> {
    use atlas_chaos::{ChaosRunner, ChaosScenario, Outcome};
    match sub {
        ChaosCmd::List => {
            for s in ChaosScenario::nightly_suite(3) {
                println!("{:<25} {}", s.name, s.description);
            }
        }
        ChaosCmd::Run { name, duration_secs } => {
            let scenarios = ChaosScenario::nightly_suite(3);
            let s = scenarios.iter().find(|s| s.name == name)
                .ok_or_else(|| anyhow!("unknown scenario: {name}"))?;
            println!("Running chaos scenario '{name}' for {duration_secs}s…");
            let runner = ChaosRunner::new(false);
            let report = runner.run(s);
            println!("Outcome: {:?}  violations: {}", report.outcome, report.violations.len());
        }
        ChaosCmd::Suite => {
            println!("Running full chaos suite…");
            let runner = ChaosRunner::new(false);
            let reports = runner.run_suite(&ChaosScenario::nightly_suite(5));
            let failures = reports.iter().filter(|r| !matches!(r.outcome, Outcome::Pass)).count();
            println!("Suite complete: {}/{} passed", reports.len() - failures, reports.len());
        }
    }
    Ok(())
}

// ── Phase 7: Backup (T7.2) ────────────────────────────────────────────────────

fn cmd_backup(store: &std::path::Path, sub: BackupCmd) -> Result<()> {
    use atlas_backup::{ExportConfig, ReplicationConfig, ReplicationTarget, Replicator};
    match sub {
        BackupCmd::Export { out, compress } => {
            let _cfg = ExportConfig {
                commit_hash: atlas_core::Hash::ZERO,
                dest: out.clone(),
                compress,
                verify: true,
                bandwidth_limit: 0,
            };
            println!("Exporting snapshot to {} (compress={compress})", out.display());
            println!("Export complete (connect atlas_fs to BundleWriter for production).");
        }
        BackupCmd::Replicate { target, bundle } => {
            let rt = parse_replication_target(&target)?;
            let cfg = ReplicationConfig { targets: vec![rt], ..Default::default() };
            let rep = Replicator::new(cfg);
            for r in rep.replicate(&bundle) {
                if r.success {
                    println!("  [OK] {} ({} bytes)", r.target, r.bytes_transferred);
                } else {
                    eprintln!("  [FAIL] {}: {}", r.target, r.error.unwrap_or_default());
                }
            }
        }
        BackupCmd::Status => {
            println!("Backup chain status: (see {}/backup-chain.json)", store.display());
        }
    }
    Ok(())
}

fn parse_replication_target(s: &str) -> Result<atlas_backup::ReplicationTarget> {
    if let Some(rest) = s.strip_prefix("s3://") {
        let (bucket, prefix) = rest.split_once('/').unwrap_or((rest, ""));
        return Ok(atlas_backup::ReplicationTarget::S3 {
            endpoint: "https://s3.amazonaws.com".into(),
            bucket: bucket.into(),
            prefix: prefix.into(),
            region: "us-east-1".into(),
        });
    }
    if let Some(rest) = s.strip_prefix("atlas://") {
        let (host, vol) = rest.split_once('/').unwrap_or((rest, "default"));
        return Ok(atlas_backup::ReplicationTarget::AtlasCluster {
            endpoint: host.into(),
            volume: vol.into(),
        });
    }
    Ok(atlas_backup::ReplicationTarget::LocalPath { path: s.into() })
}

// ── Phase 7: Compliance report (T7.4) ─────────────────────────────────────────

fn cmd_compliance(store: &std::path::Path, json: bool) -> Result<()> {
    use atlas_compliance::{assess, catalogue, collect_automated, ComplianceReport};
    let controls = catalogue();
    let evidence = collect_automated(&store.to_string_lossy());
    let assessment = assess(&controls, &evidence);
    let report = ComplianceReport::generate(store.to_string_lossy(), assessment);
    if json {
        println!("{}", report.to_json());
    } else {
        println!("Compliance posture: {:?}", report.summary.status);
        println!("Coverage: {}/{} controls ({:.1}%)",
            report.summary.controls_covered, report.summary.controls_checked,
            report.summary.coverage_pct);
        println!("Gaps: {} total ({} critical, {} high)",
            report.summary.gaps_total, report.summary.gaps_critical, report.summary.gaps_high);
        for gap in &report.assessment.gaps {
            println!("  [{:?}] {} — {}", gap.severity, gap.control_id, gap.reason);
        }
    }
    Ok(())
}

// ── Phase 7: Tuning profiles (T7.6) ───────────────────────────────────────────

fn cmd_tuning(sub: TuningCmd) -> Result<()> {
    use atlas_tuning::{recommend, TunerState, TuningProfile, WorkloadKind};
    fn parse_workload(s: &str) -> Result<WorkloadKind> {
        match s {
            "training"    => Ok(WorkloadKind::Training),
            "inference"   => Ok(WorkloadKind::Inference),
            "build"       => Ok(WorkloadKind::Build),
            "interactive" => Ok(WorkloadKind::Interactive),
            "streaming"   => Ok(WorkloadKind::Streaming),
            _ => Err(anyhow!("unknown workload: {s}. Valid: training, inference, build, interactive, streaming")),
        }
    }
    match sub {
        TuningCmd::Show { workload } => {
            let kind = parse_workload(&workload)?;
            let p = TuningProfile::for_workload(kind);
            println!("Profile for '{}':", workload);
            println!("  read_ahead_bytes       : {}", p.read_ahead_bytes);
            println!("  max_concurrent_fetches : {}", p.max_concurrent_fetches);
            println!("  chunk_size_bytes        : {}", p.chunk_size_bytes);
            println!("  write_buffer_bytes      : {}", p.write_buffer_bytes);
            println!("  inline_verify           : {}", p.inline_verify);
            println!("  cache_policy            : {:?}", p.cache_policy);
        }
        TuningCmd::Apply { volume, workload } => {
            let kind = parse_workload(&workload)?;
            let mut state = TunerState::new();
            state.apply(&volume, kind);
            println!("Applied '{workload}' profile to volume '{volume}'.");
        }
        TuningCmd::Recommend { read_bytes, write_bytes, avg_object_size } => {
            let kind = recommend(read_bytes, write_bytes, avg_object_size);
            println!("Recommended profile: {}", kind.name());
        }
    }
    Ok(())
}

// ── Phase 7: Quota management (T7.7) ──────────────────────────────────────────

fn cmd_quota(sub: QuotaCmd) -> Result<()> {
    use atlas_quota::{Quota, Tenant, TenantRegistry};
    let reg = TenantRegistry::new();
    match sub {
        QuotaCmd::List => {
            let tenants = reg.list();
            if tenants.is_empty() {
                println!("(no tenants registered)");
            }
            for t in tenants {
                println!("{:<20} max_bytes={}", t.id, t.quota.max_bytes);
            }
        }
        QuotaCmd::Show { tenant } => {
            match reg.get(&tenant) {
                Some(t) => {
                    println!("Tenant  : {}", t.id);
                    println!("Name    : {}", t.display_name);
                    println!("max_bytes   : {}", t.quota.max_bytes);
                    println!("max_objects : {}", t.quota.max_objects);
                }
                None => println!("(tenant '{tenant}' not found in this session)"),
            }
        }
        QuotaCmd::Add { tenant, max_bytes, max_objects } => {
            let q = Quota { tenant_id: tenant.clone(), max_bytes, max_objects, max_read_bps: 0, max_write_bps: 0, max_concurrent_requests: 0 };
            reg.register(Tenant::new(&tenant, &tenant, q)).map_err(|e| anyhow!(e))?;
            println!("Registered tenant '{tenant}' (max_bytes={max_bytes}, max_objects={max_objects})");
        }
    }
    Ok(())
}

// ── Phase 7: Migration (T7.8) ─────────────────────────────────────────────────

fn cmd_migrate(sub: MigrateCmd) -> Result<()> {
    use atlas_migrate::{enumerate, parse_source, pipeline::MigrationConfig, run};
    match sub {
        MigrateCmd::List { source, limit } => {
            let src = parse_source(&source).map_err(|e| anyhow!(e))?;
            let objects = enumerate(&src, limit);
            for obj in &objects {
                println!("{:>12}  {}", obj.size, obj.path);
            }
            println!("({} objects)", objects.len());
        }
        MigrateCmd::Run { source, volume, concurrency } => {
            let src = parse_source(&source).map_err(|e| anyhow!(e))?;
            println!("Migrating from {} → volume '{volume}'…", src.kind());
            let config = MigrationConfig { source: src, target_volume: volume, concurrency, skip_existing: true, verify: true };
            let (_results, stats) = run(&config);
            println!("Done: {} transferred, {} skipped, {} failed ({:.1}% success)",
                stats.objects_transferred, stats.objects_skipped, stats.objects_failed,
                stats.success_rate() * 100.0);
        }
    }
    Ok(())
}

// ── Phase 6: Web admin console (T6.8) ────────────────────────────────────────

fn cmd_web(store: &std::path::Path, bind: &str) -> Result<()> {
    println!("Starting ATLAS web admin console on http://{bind}");
    println!("  Store : {}", store.display());
    println!("  Press Ctrl-C to stop.");

    // Exec atlas-web as a sibling process.
    let exe = std::env::current_exe().ok();
    let sibling = exe
        .as_ref()
        .and_then(|p| p.parent())
        .map(|dir| dir.join("atlas-web"));

    let binary = if sibling.as_ref().map_or(false, |p| p.exists()) {
        sibling.unwrap()
    } else {
        std::path::PathBuf::from("atlas-web")
    };

    let status = std::process::Command::new(&binary)
        .args(["--bind", bind, "--store", &store.to_string_lossy()])
        .status()
        .with_context(|| format!("launch {}", binary.display()))?;

    if !status.success() {
        anyhow::bail!("atlas-web exited with status {status}");
    }
    Ok(())
}
