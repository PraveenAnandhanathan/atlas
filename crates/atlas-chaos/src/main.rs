//! `atlas-chaos` CLI — run fault-injection scenarios (T7.1).

use anyhow::Result;
use atlas_chaos::{ChaosRunner, ChaosScenario};
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "atlas-chaos", version, about = "ATLAS fault-injection harness")]
struct Args {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// List all built-in scenarios.
    List,
    /// Run one named scenario (dry-run by default).
    Run {
        /// Scenario name (from `list`).
        name: String,
        /// Actually apply faults to a real cluster (off by default).
        #[arg(long)]
        live: bool,
        /// Number of cluster nodes for rolling-restart scenario.
        #[arg(long, default_value_t = 3)]
        nodes: usize,
    },
    /// Run the full nightly suite (dry-run).
    Suite {
        #[arg(long, default_value_t = 3)]
        nodes: usize,
        /// Emit JSON report to stdout.
        #[arg(long)]
        json: bool,
    },
}

fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let args = Args::parse();
    match args.cmd {
        Cmd::List => {
            println!("Built-in chaos scenarios:");
            for s in ChaosScenario::nightly_suite(3) {
                println!("  {:30}  {}", s.name, s.description);
            }
        }
        Cmd::Run { name, live, nodes } => {
            let suite = ChaosScenario::nightly_suite(nodes);
            let scenario = suite.iter().find(|s| s.name == name)
                .ok_or_else(|| anyhow::anyhow!("unknown scenario: {name}"))?;
            let runner = ChaosRunner::new(!live);
            let report = runner.run(scenario);
            println!("{}", report.summary());
            if !report.passed() { std::process::exit(1); }
        }
        Cmd::Suite { nodes, json } => {
            let suite = ChaosScenario::nightly_suite(nodes);
            let runner = ChaosRunner::new(true);
            let reports = runner.run_suite(&suite);
            if json {
                println!("{}", serde_json::to_string_pretty(&reports)?);
            } else {
                for r in &reports { println!("{}", r.summary()); }
            }
            let failures = reports.iter().filter(|r| !r.passed()).count();
            if failures > 0 {
                eprintln!("{failures} scenario(s) failed");
                std::process::exit(1);
            }
        }
    }
    Ok(())
}
