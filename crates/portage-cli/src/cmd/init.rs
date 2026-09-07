//! `portage init` — create the data directory, config, lock file, and catalog.
//!
//! Design (`docs/design.md`, UX table + K22): measure the system drive's free
//! space. If it is below 8 GiB, *recommend* the largest other volume (e.g.
//! `D:\PortageData`) and stop — never relocate silently. Reject a chosen
//! `--data-dir` whose volume has less than 8 GiB free. Overlay / Cloud Filter
//! detection arrives in PR 4; PR 1 gates on free space only.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::Args;
use portage_core::units::GIB;
use portage_core::ByteSize;

/// Minimum free space on the volume that hosts the catalog.
pub const MIN_DATA_DIR_FREE: u64 = 8 * GIB;

/// Arguments for `portage init`.
#[derive(Args)]
pub struct InitArgs {
    /// Directory for the catalog, lock file, logs, and staging metadata.
    /// Defaults to %LOCALAPPDATA%\Portage (Windows) or ~/.local/share/portage.
    #[arg(long)]
    pub data_dir: Option<PathBuf>,
}

/// A volume candidate for hosting the data dir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Volume {
    /// Root path, e.g. `C:\`.
    pub root: PathBuf,
    /// Free bytes available to the current user.
    pub free: ByteSize,
}

/// Decision produced by the pure recommendation logic (unit-testable).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recommendation {
    /// The default location is fine.
    UseDefault,
    /// The system volume is tight; recommend this root instead.
    Recommend {
        /// Root of the largest non-system volume.
        root: PathBuf,
        /// Its free space.
        free: ByteSize,
    },
}

/// Pure decision: given the system volume and the other candidates, decide
/// whether to recommend relocating the data dir. Never relocates by itself.
pub fn recommend(system: &Volume, others: &[Volume]) -> Recommendation {
    if system.free.bytes() >= MIN_DATA_DIR_FREE {
        return Recommendation::UseDefault;
    }
    match others.iter().max_by_key(|v| v.free) {
        Some(best) if best.free.bytes() >= MIN_DATA_DIR_FREE => Recommendation::Recommend {
            root: best.root.clone(),
            free: best.free,
        },
        _ => Recommendation::UseDefault,
    }
}

/// Run `portage init`.
pub fn run(args: &InitArgs, verbose: bool) -> Result<()> {
    match &args.data_dir {
        Some(dir) => init_at(dir, verbose),
        None => {
            let system = system_volume()?;
            match recommend(&system, &other_volumes()) {
                Recommendation::UseDefault => init_at(&default_data_dir()?, verbose),
                Recommendation::Recommend { root, free } => {
                    let suggested = root.join("PortageData");
                    println!(
                        "The system volume has only {} free (minimum is {}).",
                        system.free,
                        ByteSize::new(MIN_DATA_DIR_FREE)
                    );
                    println!(
                        "Recommendation: put the Portage data dir on {} ({} free), e.g.:",
                        root.display(),
                        free
                    );
                    println!("\n    portage init --data-dir {}\n", suggested.display());
                    println!("Nothing was created. Portage never relocates silently.");
                    Ok(())
                }
            }
        }
    }
}

/// Create the data dir, lock file, and config at an explicit location.
fn init_at(dir: &Path, verbose: bool) -> Result<()> {
    let free = volume_free(dir)?;
    if free.bytes() < MIN_DATA_DIR_FREE {
        bail!(
            "refusing data_dir {}: volume has {} free, minimum is {}",
            dir.display(),
            free,
            ByteSize::new(MIN_DATA_DIR_FREE)
        );
    }

    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;

    // From here on the data dir exists, so structured logging can start.
    let _guard = portage_core::obs::init_tracing(dir, verbose)
        .with_context(|| format!("initializing logging in {}", dir.display()))?;

    let lock = dir.join("portage.lock");
    if !lock.exists() {
        fs::write(&lock, b"").with_context(|| format!("creating {}", lock.display()))?;
    }

    {
        let catalog = portage_catalog::Catalog::open(dir, portage_catalog::LockMode::Exclusive)
            .with_context(|| format!("opening catalog in {}", dir.display()))?;
        println!("created {}", catalog.paths().catalog().display());
    }

    let config = dir.join("config.yaml");
    if !config.exists() {
        fs::write(&config, example_config())
            .with_context(|| format!("creating {}", config.display()))?;
        println!("wrote   {}", config.display());
    } else {
        println!("kept    {} (already exists)", config.display());
    }

    println!("created {}", lock.display());
    println!("data dir ready: {}", dir.display());
    println!("next: portage doctor");
    tracing::info!(data_dir = %dir.display(), free = free.bytes(), "data dir initialized");
    Ok(())
}

/// The example policy copied into a fresh config. Embedded at compile time so
/// the binary works outside the repo checkout.
fn example_config() -> &'static str {
    include_str!("../../../../configs/examples/gaming-clips.yaml")
}

/// Default data dir: `%LOCALAPPDATA%\Portage` or `~/.local/share/portage`.
pub fn default_data_dir() -> Result<PathBuf> {
    #[cfg(windows)]
    {
        let base = std::env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is not set")?;
        Ok(PathBuf::from(base).join("Portage"))
    }
    #[cfg(not(windows))]
    {
        let home = std::env::var_os("HOME").context("HOME is not set")?;
        Ok(PathBuf::from(home).join(".local/share/portage"))
    }
}

/// Walk up until an existing ancestor is found (the dir may not exist yet).
fn existing_ancestor(dir: &Path) -> &Path {
    let mut current = dir;
    while !current.exists() {
        match current.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => current = parent,
            _ => break,
        }
    }
    current
}

/// Free space on the volume containing `path`.
pub fn volume_free(path: &Path) -> Result<ByteSize> {
    let path = existing_ancestor(path);
    let free = fs2::available_space(path)
        .with_context(|| format!("measuring free space at {}", path.display()))?;
    Ok(ByteSize::new(free))
}

/// The volume the OS lives on (`C:\` on Windows, `/` elsewhere).
fn system_volume() -> Result<Volume> {
    let root = system_root();
    let free = volume_free(&root)?;
    Ok(Volume { root, free })
}

fn system_root() -> PathBuf {
    #[cfg(windows)]
    {
        let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
        PathBuf::from(format!("{drive}\\"))
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("/")
    }
}

/// Enumerate other fixed volumes. Windows probes drive letters; overlay and
/// removable classification arrives in PR 4, so this is best-effort.
fn other_volumes() -> Vec<Volume> {
    #[cfg(windows)]
    {
        let system = system_root();
        (b'A'..=b'Z')
            .map(|letter| PathBuf::from(format!("{}:\\", letter as char)))
            .filter(|root| *root != system && root.exists())
            .filter_map(|root| {
                fs2::available_space(&root).ok().map(|free| Volume {
                    root,
                    free: ByteSize::new(free),
                })
            })
            .collect()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vol(root: &str, free_gib: u64) -> Volume {
        Volume {
            root: PathBuf::from(root),
            free: ByteSize::from_gib(free_gib),
        }
    }

    #[test]
    fn roomy_system_volume_uses_default() {
        let system = vol("C:\\", 100);
        let others = [vol("D:\\", 500)];
        assert_eq!(recommend(&system, &others), Recommendation::UseDefault);
    }

    #[test]
    fn tight_system_volume_recommends_largest_other() {
        let system = vol("C:\\", 4);
        let others = [vol("E:\\", 50), vol("D:\\", 500)];
        assert_eq!(
            recommend(&system, &others),
            Recommendation::Recommend {
                root: PathBuf::from("D:\\"),
                free: ByteSize::from_gib(500),
            }
        );
    }

    #[test]
    fn tight_system_volume_with_no_better_option_uses_default() {
        let system = vol("C:\\", 4);
        // The only other volume is also below the minimum.
        let others = [vol("D:\\", 2)];
        assert_eq!(recommend(&system, &others), Recommendation::UseDefault);
        assert_eq!(recommend(&system, &[]), Recommendation::UseDefault);
    }

    #[test]
    fn boundary_exactly_8_gib_is_enough() {
        let system = vol("C:\\", 8);
        assert_eq!(recommend(&system, &[]), Recommendation::UseDefault);
    }

    #[test]
    fn existing_ancestor_walks_up() {
        let tmp = std::env::temp_dir();
        let missing = tmp.join("portage-test-nonexistent").join("deep");
        assert_eq!(existing_ancestor(&missing), tmp.as_path());
        assert_eq!(existing_ancestor(&tmp), tmp.as_path());
    }
}
