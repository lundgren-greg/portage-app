//! `portage doctor` — catalog integrity (PR 3) and the K6 overlay audit
//! (PR 4). Token and journal checks arrive with later PRs.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::Args;
use portage_catalog::lock::LockMode;
use portage_catalog::{Catalog, Hydration, OverlayRootRow};
use portage_providers::local::check_root;
use portage_providers::local::pathcmp::{path_str, HOST_CASE_INSENSITIVE};
use portage_providers::local::sync_roots::{Detector, InstalledSyncClients, OverlayRoot};
use portage_providers::registry::load_local_all;
use portage_providers::{HostOverlays, LocalConfig, Provider};

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

/// K6 audit, pure over its inputs.
///
/// Fails if a sync client is installed but no overlay root was registered,
/// or if any configured local root is itself (inside) an overlay.
pub fn overlay_problems(
    installed: InstalledSyncClients,
    registered: &[OverlayRootRow],
    detected: &[OverlayRoot],
    locals: &[(String, LocalConfig)],
    case_insensitive: bool,
) -> Vec<String> {
    let mut problems = Vec::new();
    if installed.any() && registered.is_empty() {
        let which = match (installed.onedrive, installed.drivefs) {
            (true, true) => "OneDrive and Google Drive for desktop are",
            (true, false) => "OneDrive is",
            _ => "Google Drive for desktop is",
        };
        problems.push(format!(
            "{which} installed but no overlay_roots are registered; run \
             `portage provider add local --root <dir>` or `portage index` so sync roots are excluded"
        ));
    }
    let mut overlays: Vec<OverlayRoot> = detected.to_vec();
    overlays.extend(registered.iter().map(|r| OverlayRoot {
        path: PathBuf::from(&r.path),
        provider_id: r.provider_id.clone(),
        detector: Detector::CloudFilterVolume,
    }));
    for (id, cfg) in locals {
        if let Err(err) = check_root(
            &path_str(&cfg.root),
            &overlays,
            None,
            Hydration::LocalFull,
            case_insensitive,
        ) {
            problems.push(format!("local provider {id}: {err}"));
        }
    }
    problems
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

    let host = HostOverlays::detect();
    let registered = cat.overlay_roots().context("overlay_roots")?;
    let locals: Vec<(String, LocalConfig)> = load_local_all(&cat, &host)?
        .into_iter()
        .map(|p| (p.id().to_string(), p.config().clone()))
        .collect();
    println!("overlays : {} registered", registered.len());
    let problems = overlay_problems(
        host.installed,
        &registered,
        &host.roots,
        &locals,
        HOST_CASE_INSENSITIVE,
    );
    if !problems.is_empty() {
        for p in &problems {
            eprintln!("doctor: {p}");
        }
        bail!("overlay check failed ({} problem(s))", problems.len());
    }
    println!("doctor   : ok");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use portage_providers::Role;

    fn row(path: &str) -> OverlayRootRow {
        OverlayRootRow {
            path: path.into(),
            provider_id: "overlay:onedrive".into(),
            detector: "onedrive_userfolder".into(),
        }
    }

    fn local(root: &str) -> (String, LocalConfig) {
        (
            "local-x".into(),
            LocalConfig {
                root: PathBuf::from(root),
                roles: Role::Final,
                volume_serial: "1".into(),
                volume_label: String::new(),
                volume_mount: PathBuf::from(r"C:\"),
                removable: false,
                bus: None,
                drive_type: "fixed".into(),
            },
        )
    }

    const BOTH: InstalledSyncClients = InstalledSyncClients {
        onedrive: true,
        drivefs: true,
    };

    #[test]
    fn installed_client_with_no_registered_overlays_fails() {
        for installed in [
            BOTH,
            InstalledSyncClients {
                onedrive: true,
                drivefs: false,
            },
            InstalledSyncClients {
                onedrive: false,
                drivefs: true,
            },
        ] {
            let p = overlay_problems(installed, &[], &[], &[], true);
            assert_eq!(p.len(), 1, "{p:?}");
            assert!(p[0].contains("overlay_roots"));
        }
    }

    #[test]
    fn nothing_installed_or_overlays_registered_passes() {
        assert!(overlay_problems(Default::default(), &[], &[], &[], true).is_empty());
        let rows = [row(r"C:\Users\example\OneDrive")];
        assert!(overlay_problems(BOTH, &rows, &[], &[local(r"C:\")], true).is_empty());
    }

    #[test]
    fn local_root_that_is_an_overlay_fails() {
        let rows = [row(r"C:\Users\example\OneDrive")];
        let p = overlay_problems(
            BOTH,
            &rows,
            &[],
            &[local(r"C:\Users\example\OneDrive\Videos")],
            true,
        );
        assert_eq!(p.len(), 1);
        assert!(p[0].contains("local-x"));
    }
}
