//! `portage doctor` — catalog integrity stub (PR 3). Overlay / token
//! checks arrive with later PRs.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::Args;
use portage_catalog::lock::LockMode;
use portage_catalog::Catalog;

use super::init::default_data_dir;

/// Arguments for `portage doctor`.
#[derive(Args)]
pub struct DoctorArgs {
    /// Data directory that holds `catalog.sqlite` and `portage.lock`.
    #[arg(long)]
    pub data_dir: Option<PathBuf>,

    /// Copy catalog.sqlite (after a WAL checkpoint) next to it.
    #[arg(long)]
    pub backup: bool,
}

/// Run `portage doctor`.
pub fn run(args: &DoctorArgs, verbose: bool) -> Result<()> {
    let dir = match &args.data_dir {
        Some(p) => p.clone(),
        None => default_data_dir()?,
    };
    let _guard = portage_core::obs::init_tracing(&dir, verbose)
        .with_context(|| format!("initializing logging in {}", dir.display()))?;

    // Exclusive so a first `doctor` can create + migrate the catalog.
    let mode = if args.backup || !dir.join("catalog.sqlite").exists() {
        LockMode::Exclusive
    } else {
        LockMode::Shared
    };
    let cat = Catalog::open(&dir, mode)
        .with_context(|| format!("opening catalog in {}", dir.display()))?;
    let report = cat.integrity().context("integrity_check")?;
    let counts = cat.counts().context("counts")?;
    let version = cat.schema_version().context("schema version")?;

    println!("catalog  : {}", cat.paths().catalog().display());
    println!("schema   : {version}");
    println!("files    : {}", counts.files);
    println!("blobs    : {}", counts.blobs);
    println!("replicas : {}", counts.replicas);
    println!(
        "integrity: {}",
        if report.is_ok() { "ok" } else { "FAILED" }
    );

    if args.backup {
        let dest = cat.backup_path_today().context("backup path")?;
        cat.checkpoint_and_backup(&dest)
            .with_context(|| format!("backing up to {}", dest.display()))?;
        println!("backup   : {}", dest.display());
    }

    if !report.is_ok() {
        bail!(
            "catalog integrity failed: {} (fk violations: {})",
            report.integrity,
            report.foreign_key_violations
        );
    }
    println!(
        "catalog ok  schema={version}  files={}  blobs={}  replicas={}",
        counts.files, counts.blobs, counts.replicas
    );
    println!("doctor   : ok");
    Ok(())
}
