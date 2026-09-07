//! `portage` — inventory local disks and clouds, plan space-safe moves.
//!
//! PR 1: `--help` / `init`. PR 1.5: logging, metrics, `status`.
//! PR 3: SQLite catalog + `doctor` integrity stub.

mod cmd;

use clap::{Parser, Subcommand};

/// Portage inventories files across local volumes (including USB) and clouds,
/// then moves them under a plan you confirm. Nothing mutates without a typed
/// plan id. No telemetry.
#[derive(Parser)]
#[command(name = "portage", version, about, long_about = None)]
struct Cli {
    /// Verbose stderr logging (debug level). RUST_LOG overrides.
    #[arg(long, short = 'v', global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create the Portage data directory, config, and lock file.
    ///
    /// Measures free space on the system drive. If it is below 8 GiB, prints
    /// a recommendation for the largest volume instead — it never relocates
    /// silently. Pass --data-dir to choose the location explicitly.
    Init(cmd::init::InitArgs),

    /// Show run metrics and data-dir health. --format=prom emits Prometheus
    /// text for a local textfile collector (no listener, no push).
    Status(cmd::status::StatusArgs),

    /// Check catalog integrity (and optionally back up catalog.sqlite).
    ///
    /// Overlay, token, and journal audits arrive in later PRs. This stub
    /// opens the catalog, runs SQLite integrity + foreign-key checks, and
    /// with --backup copies a checkpointed catalog next to the live file.
    Doctor(cmd::doctor::DoctorArgs),
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Init(args) => cmd::init::run(&args, cli.verbose),
        Command::Status(args) => cmd::status::run(&args, cli.verbose),
        Command::Doctor(args) => cmd::doctor::run(&args, cli.verbose),
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            std::process::ExitCode::from(2)
        }
    }
}
