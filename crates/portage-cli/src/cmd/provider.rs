//! `portage provider add local` (PR 4). Cloud kinds arrive in PRs 6–8.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use portage_catalog::{Catalog, LockMode};
use portage_providers::registry::{add_local, default_local_id};
use portage_providers::{HostOverlays, Provider, Role};

use super::init::checked_data_dir;

/// Arguments for `portage provider`.
#[derive(Args)]
pub struct ProviderArgs {
    #[command(subcommand)]
    pub command: ProviderCommand,
}

/// `portage provider <command>`.
#[derive(Subcommand)]
pub enum ProviderCommand {
    /// Add or update a provider.
    Add(AddArgs),
}

/// `portage provider add <kind>`.
#[derive(Args)]
pub struct AddArgs {
    #[command(subcommand)]
    pub kind: AddKind,
}

/// Provider kinds that can be added.
#[derive(Subcommand)]
pub enum AddKind {
    /// A local volume root (internal disk or removable / USB).
    ///
    /// Refuses a root that is a OneDrive / Google Drive overlay or sits on a
    /// DriveFS / Cloud Filter volume — add that cloud provider instead.
    /// Overlays inside a broader root (e.g. C:\) are skipped, not refused.
    Local(AddLocalArgs),
}

/// Arguments for `portage provider add local`.
#[derive(Args)]
pub struct AddLocalArgs {
    /// Directory to inventory (e.g. D:\ or E:\Media).
    #[arg(long)]
    pub root: PathBuf,

    /// Provider id. Defaults to local-<drive letter> or local-<dir name>.
    #[arg(long)]
    pub id: Option<String>,

    /// How plans may use this volume: shuttle, final, or both.
    /// Defaults to both for removable disks, final otherwise.
    #[arg(long)]
    pub role: Option<Role>,

    /// Data directory that holds `catalog.sqlite`.
    #[arg(long)]
    pub data_dir: Option<PathBuf>,
}

/// Run `portage provider`.
pub fn run(args: &ProviderArgs, verbose: bool) -> Result<()> {
    match &args.command {
        ProviderCommand::Add(AddArgs {
            kind: AddKind::Local(local),
        }) => add_local_cmd(local, verbose),
    }
}

fn add_local_cmd(args: &AddLocalArgs, verbose: bool) -> Result<()> {
    let dir = checked_data_dir(args.data_dir.as_deref())?;
    let _guard = portage_core::obs::init_tracing(&dir, verbose)
        .with_context(|| format!("initializing logging in {}", dir.display()))?;
    let cat = Catalog::open(&dir, LockMode::Exclusive)
        .with_context(|| format!("opening catalog in {}", dir.display()))?;

    let id = args
        .id
        .clone()
        .unwrap_or_else(|| default_local_id(&args.root));
    let host = HostOverlays::detect();
    let p = add_local(&cat, &id, &args.root, args.role, &host)
        .with_context(|| format!("adding local provider {id}"))?;
    let c = p.config();
    println!("provider : {} (local)", p.id());
    println!("root     : {}", c.root.display());
    println!(
        "volume   : {} serial {} ({}{})",
        if c.volume_label.is_empty() {
            "(no label)"
        } else {
            &c.volume_label
        },
        c.volume_serial,
        if c.removable { "removable" } else { "internal" },
        c.bus
            .as_deref()
            .map(|b| format!(", {b}"))
            .unwrap_or_default()
    );
    println!("roles    : {}", c.roles);
    let inner = p.inner_overlays();
    if !inner.is_empty() {
        println!("skipping overlays inside this root (cloud providers own them):");
        for o in inner {
            println!("  {}  [{}]", o.path.display(), o.detector.as_str());
        }
    }
    println!("next: portage index --provider {}", p.id());
    Ok(())
}
