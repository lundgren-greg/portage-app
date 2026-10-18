//! `portage status` — data-dir health and metrics.
//!
//! PR 1.5 scope: measure free space on the data-dir volume, refresh the
//! Prometheus snapshot at `%data_dir%/metrics/portage.prom`, and print
//! either a human table or Prometheus text (`--format=prom`). Journal and
//! plan status arrive with PR 11.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Args, ValueEnum};
use portage_core::obs::{self, MetricsRegistry};
use portage_core::ByteSize;

use super::init;

/// Output format for `portage status`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum Format {
    /// Human-readable table.
    Text,
    /// Prometheus text exposition format.
    Prom,
}

/// Arguments for `portage status`.
#[derive(Args)]
pub struct StatusArgs {
    /// Data directory (defaults to the platform data dir created by init).
    #[arg(long)]
    pub data_dir: Option<PathBuf>,

    /// Output format.
    #[arg(long, value_enum, default_value_t = Format::Text)]
    pub format: Format,
}

/// Run `portage status`.
pub fn run(args: &StatusArgs, verbose: bool) -> Result<()> {
    let data_dir = init::checked_data_dir(args.data_dir.as_deref())?;
    if !data_dir.join("portage.lock").exists() {
        bail!(
            "{} is not a Portage data dir (no portage.lock); run: portage init",
            data_dir.display()
        );
    }

    let _guard = obs::init_tracing(&data_dir, verbose)
        .with_context(|| format!("initializing logging in {}", data_dir.display()))?;

    let free = init::volume_free(&data_dir)?;
    let metrics = MetricsRegistry::new();
    metrics.set_gauge("space.free", free.bytes() as f64);
    metrics.inc("status.runs", 1);
    tracing::info!(free = free.bytes(), data_dir = %data_dir.display(), "status");

    // Refresh the local scrape target regardless of the output format.
    metrics
        .write_prom(&data_dir)
        .with_context(|| format!("writing metrics under {}", data_dir.display()))?;

    match args.format {
        Format::Prom => print!("{}", metrics.to_prometheus()),
        Format::Text => {
            println!("data dir : {}", data_dir.display());
            println!("free     : {}", ByteSize::new(free.bytes()));
            println!("lock     : {}", data_dir.join("portage.lock").display());
            println!(
                "metrics  : {}",
                data_dir.join("metrics").join("portage.prom").display()
            );
            println!("journal  : none (executor arrives in PR 11)");
        }
    }
    Ok(())
}
