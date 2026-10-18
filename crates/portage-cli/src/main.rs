//! `portage` — inventory local disks and clouds, plan space-safe moves.
//!
//! PR 1: `--help` / `init`. PR 1.5: logging, metrics, `status`.
//! PR 3: SQLite catalog + `doctor` integrity stub.
//! PR 4: `provider add local`, `index`, `capacity`, doctor overlay audit.

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

    /// Check catalog integrity and cloud-overlay safety (and optionally back
    /// up catalog.sqlite).
    ///
    /// Runs SQLite integrity + foreign-key checks, then fails if OneDrive or
    /// Google Drive for desktop is installed but no overlay roots are
    /// registered, or if a local root is itself an overlay. With --backup
    /// copies a checkpointed catalog next to the live file. Token and journal
    /// audits arrive in later PRs.
    Doctor(cmd::doctor::DoctorArgs),

    /// Manage storage providers (PR 4: local volumes, including USB).
    Provider(cmd::provider::ProviderArgs),

    /// Inventory local providers into the catalog (metadata only).
    ///
    /// Never reads file contents and never opens cloud placeholders; sync
    /// roots inside a root are skipped. An unplugged volume fails closed.
    Index(cmd::index::IndexArgs),

    /// Show free space per local volume (internal and removable separately).
    Capacity(cmd::capacity::CapacityArgs),
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Init(args) => cmd::init::run(&args, cli.verbose),
        Command::Status(args) => cmd::status::run(&args, cli.verbose),
        Command::Doctor(args) => cmd::doctor::run(&args, cli.verbose),
        Command::Provider(args) => cmd::provider::run(&args, cli.verbose),
        Command::Index(args) => cmd::index::run(&args, cli.verbose),
        Command::Capacity(args) => cmd::capacity::run(&args, cli.verbose),
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            std::process::ExitCode::from(2)
        }
    }
}
