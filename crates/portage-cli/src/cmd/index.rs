//! `portage index` — metadata walk of local providers (PR 4). No file body is
//! read; placeholders are recorded, never opened. Hashing is PR 5.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::Args;
use portage_catalog::{Catalog, LockMode};
use portage_engine::index::index_local;
use portage_providers::registry::{load_local, load_local_all};
use portage_providers::{HostOverlays, Provider};

use super::init::default_data_dir;

/// Arguments for `portage index`.
#[derive(Args)]
pub struct IndexArgs {
    /// Only index this provider (default: every local provider).
    #[arg(long)]
    pub provider: Option<String>,

    /// Data directory that holds `catalog.sqlite`.
    #[arg(long)]
    pub data_dir: Option<PathBuf>,
}

/// Run `portage index`.
pub fn run(args: &IndexArgs, verbose: bool) -> Result<()> {
    let dir = match &args.data_dir {
        Some(p) => p.clone(),
        None => default_data_dir()?,
    };
    let _guard = portage_core::obs::init_tracing(&dir, verbose)
        .with_context(|| format!("initializing logging in {}", dir.display()))?;
    let cat = Catalog::open(&dir, LockMode::Exclusive)
        .with_context(|| format!("opening catalog in {}", dir.display()))?;

    let host = HostOverlays::detect();
    let providers = match &args.provider {
        Some(id) => vec![load_local(&cat, id, &host)?],
        None => load_local_all(&cat, &host)?,
    };
    if providers.is_empty() {
        bail!("no local providers; run `portage provider add local --root <dir>` first");
    }

    let mut failed = 0usize;
    for mut p in providers {
        match index_local(&cat, &mut p) {
            Ok(r) => {
                if let Some(root) = &r.moved_to {
                    println!("{}: volume moved, now at {}", p.id(), root.display());
                }
                println!(
                    "{}: {} files, {} dirs, {} placeholders (not opened), {} overlays skipped, {} links skipped, {} unreadable  [scan {}]",
                    p.id(),
                    r.files,
                    r.dirs,
                    r.placeholders,
                    r.skipped_overlays,
                    r.skipped_links,
                    r.unreadable,
                    r.scan_id
                );
            }
            Err(err) => {
                failed += 1;
                eprintln!("{}: {err}", p.id());
            }
        }
    }
    if failed > 0 {
        bail!("{failed} provider(s) failed to index");
    }
    Ok(())
}
