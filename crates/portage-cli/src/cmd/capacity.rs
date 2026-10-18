//! `portage capacity` — free space per local volume, internal and removable
//! listed separately (PR 4). Each measurement is stored as a snapshot.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Args;
use portage_catalog::{Catalog, LockMode};
use portage_core::ByteSize;
use portage_engine::index::record_capacity;
use portage_providers::registry::load_local_all;
use portage_providers::{HostOverlays, LocalProvider, Provider};

use super::init::default_data_dir;

/// Arguments for `portage capacity`.
#[derive(Args)]
pub struct CapacityArgs {
    /// Data directory that holds `catalog.sqlite`.
    #[arg(long)]
    pub data_dir: Option<PathBuf>,
}

/// Run `portage capacity`.
pub fn run(args: &CapacityArgs, verbose: bool) -> Result<()> {
    let dir = match &args.data_dir {
        Some(p) => p.clone(),
        None => default_data_dir()?,
    };
    let _guard = portage_core::obs::init_tracing(&dir, verbose)
        .with_context(|| format!("initializing logging in {}", dir.display()))?;
    let cat = Catalog::open(&dir, LockMode::Exclusive)
        .with_context(|| format!("opening catalog in {}", dir.display()))?;

    let providers = load_local_all(&cat, &HostOverlays::detect())?;
    if providers.is_empty() {
        println!("no local providers; run `portage provider add local --root <dir>`");
        return Ok(());
    }
    let (removable, internal): (Vec<_>, Vec<_>) =
        providers.iter().partition(|p| p.config().removable);
    for (title, group) in [("internal", internal), ("removable", removable)] {
        if group.is_empty() {
            continue;
        }
        println!("{title}:");
        for p in group {
            print_one(&cat, p);
        }
    }
    Ok(())
}

fn print_one(cat: &Catalog, p: &LocalProvider) {
    let c = p.config();
    match record_capacity(cat, p) {
        Ok(cap) => println!(
            "  {:<16} {:<12} free {:>12}  total {:>12}  roles {}  {}",
            p.id().as_str(),
            c.volume_serial,
            ByteSize::new(cap.free_bytes).to_string(),
            cap.total_bytes
                .map(|t| ByteSize::new(t).to_string())
                .unwrap_or_else(|| "?".into()),
            c.roles,
            c.root.display()
        ),
        // Offline is reported, never guessed: no snapshot is written.
        Err(err) => println!(
            "  {:<16} {:<12} OFFLINE  {err}",
            p.id().as_str(),
            c.volume_serial
        ),
    }
}
