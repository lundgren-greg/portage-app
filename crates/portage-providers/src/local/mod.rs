//! Local volumes (fixed and removable): overlay detection, placeholders, walk.
//!
//! K6: a root that *is* (or sits inside) an overlay, or lives on a DriveFS /
//! Cloud Filter volume, is refused. Overlays *inside* a broader root are
//! skipped during the walk. Placeholders are classified from directory
//! enumeration metadata and never opened. K24: identity is the volume serial;
//! an unplugged volume fails closed with [`Error::VolumeOffline`].

pub mod pathcmp;
pub mod placeholder;
pub mod sync_roots;
pub mod volume;
#[cfg(windows)]
pub mod win;

use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;

use portage_catalog::{Hydration, ProviderKind};
use portage_core::ids::ProviderId;
use portage_core::Error;

use crate::traits::{Capacity, ListItem, ObjectMeta, Provider};
use pathcmp::{is_within, path_str, strip_verbatim, HOST_CASE_INSENSITIVE};
use sync_roots::{containing_overlay, InstalledSyncClients, OverlayRoot};
use volume::VolumeInfo;

/// How a plan may use a local volume (K24).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Local hop / staging only. Bytes here are never a final replica.
    Shuttle,
    /// Eligible as a `prefer_local` / `required` destination.
    Final,
    /// Both (default for removable disks).
    Both,
}

impl Role {
    /// Config string.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Shuttle => "shuttle",
            Self::Final => "final",
            Self::Both => "both",
        }
    }

    /// May hold a final replica.
    pub fn is_final(self) -> bool {
        matches!(self, Self::Final | Self::Both)
    }

    /// May be used as a shuttle hop.
    pub fn is_shuttle(self) -> bool {
        matches!(self, Self::Shuttle | Self::Both)
    }

    /// Default: removable disks are `both`, internal disks `final`.
    pub fn default_for(volume: &VolumeInfo) -> Self {
        if volume.is_removable() {
            Self::Both
        } else {
            Self::Final
        }
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Role {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "shuttle" => Ok(Self::Shuttle),
            "final" => Ok(Self::Final),
            "both" => Ok(Self::Both),
            other => Err(Error::ProviderConfig(format!(
                "unknown role {other:?} (expected shuttle|final|both)"
            ))),
        }
    }
}

/// Persisted config of a local provider (`providers.config_json`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalConfig {
    /// Canonical walk root.
    pub root: PathBuf,
    /// Roles.
    pub roles: Role,
    /// Volume serial (identity).
    pub volume_serial: String,
    /// Volume label at add time.
    pub volume_label: String,
    /// Volume mount at add time (`E:\`).
    pub volume_mount: PathBuf,
    /// Removable / external.
    pub removable: bool,
    /// Bus type, if reported.
    pub bus: Option<String>,
    /// Drive type string.
    pub drive_type: String,
}

impl LocalConfig {
    /// Serialize to JSON.
    pub fn to_json(&self) -> String {
        serde_json::json!({
            "root": path_str(&self.root),
            "roles": self.roles.as_str(),
            "volume_serial": self.volume_serial,
            "volume_label": self.volume_label,
            "volume_mount": path_str(&self.volume_mount),
            "removable": self.removable,
            "bus": self.bus,
            "drive_type": self.drive_type,
        })
        .to_string()
    }

    /// Parse from JSON.
    pub fn from_json(s: &str) -> Result<Self, Error> {
        let bad = |what: &str| Error::ProviderConfig(format!("local config: missing {what}"));
        let v: serde_json::Value = serde_json::from_str(s)
            .map_err(|e| Error::ProviderConfig(format!("local config is not JSON: {e}")))?;
        let text = |k: &str| v.get(k).and_then(|x| x.as_str()).ok_or_else(|| bad(k));
        Ok(Self {
            root: PathBuf::from(text("root")?),
            roles: text("roles")?.parse()?,
            volume_serial: text("volume_serial")?.to_string(),
            volume_label: text("volume_label").unwrap_or_default().to_string(),
            volume_mount: PathBuf::from(text("volume_mount")?),
            removable: v
                .get("removable")
                .and_then(|x| x.as_bool())
                .unwrap_or(false),
            bus: v.get("bus").and_then(|x| x.as_str()).map(str::to_string),
            drive_type: text("drive_type").unwrap_or("unknown").to_string(),
        })
    }
}

/// Overlay roots and installed sync clients detected on this host.
#[derive(Debug, Clone, Default)]
pub struct HostOverlays {
    /// Every detected overlay root.
    pub roots: Vec<OverlayRoot>,
    /// Which sync clients are installed.
    pub installed: InstalledSyncClients,
}

impl HostOverlays {
    /// Detect from injected readers.
    pub fn from_readers(
        reg: &dyn sync_roots::RegistryReader,
        env: &dyn sync_roots::EnvReader,
        case_insensitive: bool,
    ) -> Self {
        Self {
            roots: sync_roots::detect_overlay_roots(reg, env, case_insensitive),
            installed: sync_roots::installed_sync_clients(reg),
        }
    }

    /// Detect from this host's registry (Windows) and environment.
    pub fn detect() -> Self {
        #[cfg(windows)]
        let reg = win::WinRegistry;
        #[cfg(not(windows))]
        let reg = sync_roots::EmptyRegistry;
        Self::from_readers(&reg, &sync_roots::ProcessEnv, HOST_CASE_INSENSITIVE)
    }
}

/// Pure K6 gate for a candidate root.
///
/// Refuses a root that is (or is inside) an overlay, sits on a DriveFS /
/// Cloud Filter volume, or is itself a Cloud Filter placeholder directory.
pub fn check_root(
    root: &str,
    overlays: &[OverlayRoot],
    volume: Option<&VolumeInfo>,
    root_hydration: Hydration,
    case_insensitive: bool,
) -> Result<(), Error> {
    let refuse = |reason: String| Error::OverlayRoot {
        path: PathBuf::from(root),
        reason,
    };
    if let Some(o) = containing_overlay(root, overlays, case_insensitive) {
        return Err(refuse(format!(
            "inside {} overlay {}",
            o.detector.as_str(),
            o.path.display()
        )));
    }
    if let Some(v) = volume {
        if let Some(reason) = v.cloud_volume_reason() {
            return Err(refuse(reason));
        }
        if let Some(o) = containing_overlay(&path_str(&v.mount), overlays, case_insensitive) {
            return Err(refuse(format!(
                "volume {} is {} overlay {}",
                v.mount.display(),
                o.detector.as_str(),
                o.path.display()
            )));
        }
    }
    if root_hydration != Hydration::LocalFull {
        return Err(refuse(
            "root directory is a Cloud Filter placeholder".into(),
        ));
    }
    Ok(())
}

/// Validate a root and build its config (`portage provider add local`).
/// Touches only metadata: no file under the root is opened.
pub fn prepare_local(
    root: &Path,
    role: Option<Role>,
    host: &HostOverlays,
) -> Result<LocalConfig, Error> {
    let ci = HOST_CASE_INSENSITIVE;
    // Lexical check first, before touching the filesystem at all.
    check_root(&path_str(root), &host.roots, None, Hydration::LocalFull, ci)?;

    let io = |source| Error::Io {
        path: root.to_path_buf(),
        source,
    };
    let meta = std::fs::symlink_metadata(root).map_err(io)?;
    check_root(
        &path_str(root),
        &host.roots,
        None,
        placeholder::hydration_of(&meta),
        ci,
    )?;
    let canonical = strip_verbatim(root.canonicalize().map_err(io)?);
    let meta = std::fs::metadata(&canonical).map_err(io)?;
    if !meta.is_dir() {
        return Err(Error::ProviderConfig(format!(
            "root {} is not a directory",
            canonical.display()
        )));
    }
    let volume = volume::probe(&canonical).map_err(io)?;
    check_root(
        &path_str(&canonical),
        &host.roots,
        Some(&volume),
        placeholder::hydration_of(&meta),
        ci,
    )?;
    Ok(LocalConfig {
        root: canonical,
        roles: role.unwrap_or_else(|| Role::default_for(&volume)),
        volume_serial: volume.serial.clone(),
        volume_label: volume.label.clone(),
        volume_mount: volume.mount.clone(),
        removable: volume.is_removable(),
        bus: volume.bus.clone(),
        drive_type: volume.drive_type.as_str().to_string(),
    })
}

/// Where a configured root is mounted right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// Current root (rebased if the drive letter changed).
    pub root: PathBuf,
    /// Current mount.
    pub mount: PathBuf,
    /// The drive letter changed since the config was written.
    pub moved: bool,
}

/// Pure resolution: the root's own probe if its serial matches, else the
/// same serial at another mount, else [`Error::VolumeOffline`].
pub fn resolve_root_with(
    config: &LocalConfig,
    probed: Option<&VolumeInfo>,
    mounted: &[VolumeInfo],
) -> Result<Resolved, Error> {
    if let Some(v) = probed {
        if v.serial.eq_ignore_ascii_case(&config.volume_serial) {
            return Ok(Resolved {
                root: config.root.clone(),
                mount: v.mount.clone(),
                moved: false,
            });
        }
    }
    let offline = || Error::VolumeOffline {
        label: config.volume_label.clone(),
        serial: config.volume_serial.clone(),
        root: config.root.clone(),
    };
    let found =
        volume::find_mount_for_serial(&config.volume_serial, mounted).ok_or_else(offline)?;
    if found.cloud_volume_reason().is_some() {
        return Err(offline());
    }
    let root = volume::rebase_root(&config.root, &config.volume_mount, &found.mount)
        .ok_or_else(offline)?;
    Ok(Resolved {
        root,
        mount: found.mount.clone(),
        moved: true,
    })
}

/// A configured local volume provider.
#[derive(Debug, Clone)]
pub struct LocalProvider {
    id: ProviderId,
    config: LocalConfig,
    overlays: Vec<OverlayRoot>,
}

impl LocalProvider {
    /// Build from a stored config and the overlays to skip.
    pub fn new(id: ProviderId, config: LocalConfig, overlays: Vec<OverlayRoot>) -> Self {
        Self {
            id,
            config,
            overlays,
        }
    }

    /// Stored config.
    pub fn config(&self) -> &LocalConfig {
        &self.config
    }

    /// Every overlay root this provider knows about (skipped if inside root).
    pub fn overlays(&self) -> &[OverlayRoot] {
        &self.overlays
    }

    /// `locations.id`: `<provider>@<volume serial>`.
    pub fn location_id(&self) -> String {
        location_id(self.id.as_str(), &self.config.volume_serial)
    }

    /// Overlays strictly inside this root (skipped during walk).
    pub fn inner_overlays(&self) -> Vec<&OverlayRoot> {
        sync_roots::inner_overlays(
            &path_str(&self.config.root),
            &self.overlays,
            HOST_CASE_INSENSITIVE,
        )
    }

    /// Where the root is right now, or [`Error::VolumeOffline`].
    pub fn resolve(&self) -> Result<Resolved, Error> {
        let probed = if self.config.root.is_dir() {
            volume::probe(&self.config.root).ok()
        } else {
            None
        };
        let mounted = if probed
            .as_ref()
            .is_some_and(|v| v.serial.eq_ignore_ascii_case(&self.config.volume_serial))
        {
            Vec::new()
        } else {
            volume::mounted_volumes()
        };
        let resolved = resolve_root_with(&self.config, probed.as_ref(), &mounted)?;
        if !resolved.root.is_dir() {
            return Err(Error::VolumeOffline {
                label: self.config.volume_label.clone(),
                serial: self.config.volume_serial.clone(),
                root: self.config.root.clone(),
            });
        }
        Ok(resolved)
    }

    /// Follow a drive-letter change: adopt the resolved root and mount.
    pub fn adopt(&mut self, resolved: &Resolved) {
        self.config.root = resolved.root.clone();
        self.config.volume_mount = resolved.mount.clone();
    }

    /// `rel` joined onto the *resolved* root. Resolving first means a
    /// different disk now mounted at the old letter is never read (K24).
    fn full_path(&self, rel: &str) -> Result<(PathBuf, PathBuf), Error> {
        let root = self.resolve()?.root;
        let escape = |reason: &str| Error::PathEscape {
            root: root.clone(),
            candidate: PathBuf::from(rel),
            reason: reason.to_string(),
        };
        let candidate = Path::new(rel);
        for c in candidate.components() {
            match c {
                Component::Normal(part) if part.to_string_lossy().contains(':') => {
                    return Err(escape("contains `:` (NTFS alternate data stream)"))
                }
                Component::Normal(_) | Component::CurDir => {}
                _ => return Err(escape("must be a relative path without `..`")),
            }
        }
        let full = root.join(candidate);
        Ok((root, full))
    }

    /// True if `path` is (inside) any known overlay root (K6).
    fn in_overlay(&self, path: &Path) -> bool {
        let ps = path_str(path);
        self.overlays
            .iter()
            .any(|o| is_within(&ps, &path_str(&o.path), HOST_CASE_INSENSITIVE))
    }

    fn meta_for(&self, rel: &str, full: &Path, meta: &std::fs::Metadata) -> ObjectMeta {
        let hydration = placeholder::hydration_of(meta);
        let is_dir = meta.is_dir();
        ObjectMeta {
            path: rel.to_string(),
            name: rel.rsplit('/').next().unwrap_or(rel).to_string(),
            is_dir,
            size: (!is_dir).then_some(meta.len()),
            mtime: meta.modified().ok(),
            hydration,
            file_id: if !is_dir && hydration == Hydration::LocalFull {
                file_id(full, meta)
            } else {
                None
            },
        }
    }
}

/// `locations.id` for a local provider on a volume.
pub fn location_id(provider_id: &str, serial: &str) -> String {
    format!("{provider_id}@{serial}")
}

/// Stable file identity without reading data.
fn file_id(full: &Path, meta: &std::fs::Metadata) -> Option<String> {
    #[cfg(windows)]
    {
        let _ = meta;
        win::file_id(full)
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let _ = full;
        Some(format!("{:016x}", meta.ino()))
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = (full, meta);
        None
    }
}

/// `a/b/c` from a path under `root`, or `None` if not valid UTF-8.
fn relative(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let mut parts = Vec::new();
    for c in rel.components() {
        match c {
            Component::Normal(p) => parts.push(p.to_str()?.to_string()),
            _ => return None,
        }
    }
    Some(parts.join("/"))
}

impl Provider for LocalProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::Local
    }

    fn capacity(&self) -> Result<Capacity, Error> {
        let resolved = self.resolve()?;
        volume::capacity(&resolved.mount).map_err(|source| Error::Io {
            path: resolved.mount.clone(),
            source,
        })
    }

    fn list(&self) -> Result<Box<dyn Iterator<Item = ListItem> + '_>, Error> {
        let resolved = self.resolve()?;
        let skip: Vec<String> = self.overlays.iter().map(|o| path_str(&o.path)).collect();
        let it = walkdir::WalkDir::new(&resolved.root)
            .follow_links(false)
            .sort_by_file_name()
            .into_iter();
        Ok(Box::new(LocalWalk {
            it,
            root: resolved.root,
            skip,
            provider: self,
        }))
    }

    fn stat(&self, path: &str) -> Result<ObjectMeta, Error> {
        let (_, full) = self.full_path(path)?;
        let meta = std::fs::symlink_metadata(&full).map_err(|source| Error::Io {
            path: full.clone(),
            source,
        })?;
        Ok(self.meta_for(path, &full, &meta))
    }

    fn open_read(&self, path: &str) -> Result<Box<dyn Read + Send>, Error> {
        let (root, full) = self.full_path(path)?;
        let refuse = |reason: &str| Error::OverlayRoot {
            path: full.clone(),
            reason: reason.to_string(),
        };
        // Overlay trees belong to their cloud provider; never open them (K6).
        if self.in_overlay(&full) {
            return Err(refuse("inside an overlay root"));
        }
        // Classify from metadata first; a placeholder is never opened.
        let raw = std::fs::symlink_metadata(&full).map_err(|source| Error::Io {
            path: full.clone(),
            source,
        })?;
        // `File::open` follows links, so the attributes checked below would
        // be the link's, not the target's. The walk skips links; so does this.
        if raw.file_type().is_symlink() {
            return Err(refuse("is a symlink or junction"));
        }
        let meta = self.meta_for(path, &full, &raw);
        if meta.hydration != Hydration::LocalFull {
            return Err(Error::WouldHydrate { path: full });
        }
        if meta.is_dir {
            return Err(Error::Io {
                path: full,
                source: std::io::Error::new(std::io::ErrorKind::InvalidInput, "is a directory"),
            });
        }
        let checked = portage_core::paths::ensure_inside(&root, Path::new(path))?;
        // An intermediate junction may resolve into an overlay.
        if let Ok(real) = checked.canonicalize() {
            if self.in_overlay(&strip_verbatim(real)) {
                return Err(refuse("resolves into an overlay root"));
            }
        }
        let file = File::open(&checked).map_err(|source| Error::Io {
            path: checked,
            source,
        })?;
        Ok(Box::new(file))
    }
}

/// Iterator over a local tree. Skips overlays and links; never follows
/// reparse points; does not descend into placeholder directories.
struct LocalWalk<'a> {
    it: walkdir::IntoIter,
    root: PathBuf,
    skip: Vec<String>,
    provider: &'a LocalProvider,
}

impl Iterator for LocalWalk<'_> {
    type Item = ListItem;

    fn next(&mut self) -> Option<ListItem> {
        loop {
            let entry = match self.it.next()? {
                Ok(e) => e,
                Err(err) => {
                    return Some(ListItem::Unreadable {
                        path: err.path().map(Path::to_path_buf).unwrap_or_default(),
                        message: err.to_string(),
                    })
                }
            };
            if entry.depth() == 0 {
                continue;
            }
            let path = entry.path().to_path_buf();
            let ps = path_str(&path);
            if self
                .skip
                .iter()
                .any(|o| is_within(&ps, o, HOST_CASE_INSENSITIVE))
            {
                if entry.file_type().is_dir() {
                    self.it.skip_current_dir();
                }
                return Some(ListItem::SkippedOverlay(path));
            }
            if entry.path_is_symlink() || entry.file_type().is_symlink() {
                return Some(ListItem::SkippedLink(path));
            }
            // Directory-enumeration metadata: no file handle is opened.
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(err) => {
                    return Some(ListItem::Unreadable {
                        path,
                        message: err.to_string(),
                    })
                }
            };
            let hydration = placeholder::hydration_of(&meta);
            // Skip at most once per entry: a second `skip_current_dir` would
            // drop the parent's remaining siblings.
            let rel = relative(&self.root, &path);
            if meta.is_dir() && (hydration != Hydration::LocalFull || rel.is_none()) {
                self.it.skip_current_dir();
            }
            let Some(rel) = rel else {
                return Some(ListItem::Unreadable {
                    path,
                    message: "name is not valid UTF-8".into(),
                });
            };
            return Some(ListItem::Entry(self.provider.meta_for(&rel, &path, &meta)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::sync_roots::fakes::{FakeEnv, FakeRegistry};
    use super::sync_roots::Detector;
    use super::volume::DriveType;
    use super::*;

    fn vol(mount: &str, serial: &str, label: &str, dt: DriveType) -> VolumeInfo {
        VolumeInfo {
            mount: PathBuf::from(mount),
            serial: serial.into(),
            label: label.into(),
            fs_name: "NTFS".into(),
            drive_type: dt,
            bus: None,
        }
    }

    fn host() -> HostOverlays {
        let reg = FakeRegistry::default()
            .with_value(
                r"Software\Microsoft\OneDrive\Accounts\Personal",
                "UserFolder",
                r"C:\Users\example\OneDrive",
            )
            .with_value(sync_roots::DRIVEFS_KEY, "DefaultMountPoint", "G:");
        HostOverlays::from_readers(
            &reg,
            &FakeEnv::localappdata(r"C:\Users\example\AppData\Local"),
            true,
        )
    }

    fn cfg(root: &str, mount: &str, serial: &str) -> LocalConfig {
        LocalConfig {
            root: PathBuf::from(root),
            roles: Role::Both,
            volume_serial: serial.into(),
            volume_label: "ext-media".into(),
            volume_mount: PathBuf::from(mount),
            removable: true,
            bus: Some("usb".into()),
            drive_type: "removable".into(),
        }
    }

    #[test]
    fn roles_parse_display_and_default() {
        for r in [Role::Shuttle, Role::Final, Role::Both] {
            assert_eq!(r.to_string().parse::<Role>().unwrap(), r);
        }
        assert_eq!("BOTH".parse::<Role>().unwrap(), Role::Both);
        assert!("hop".parse::<Role>().is_err());
        assert!(Role::Both.is_final() && Role::Both.is_shuttle());
        assert!(!Role::Shuttle.is_final() && Role::Shuttle.is_shuttle());
        assert!(Role::Final.is_final() && !Role::Final.is_shuttle());
        assert_eq!(
            Role::default_for(&vol("E:\\", "1", "", DriveType::Removable)),
            Role::Both
        );
        assert_eq!(
            Role::default_for(&vol("D:\\", "1", "", DriveType::Fixed)),
            Role::Final
        );
    }

    #[test]
    fn config_json_round_trip() {
        let c = cfg(r"E:\Media", r"E:\", "BBBB-0002");
        assert_eq!(LocalConfig::from_json(&c.to_json()).unwrap(), c);
        assert!(LocalConfig::from_json("not json").is_err());
        assert!(LocalConfig::from_json("{}").is_err());
    }

    #[test]
    fn refuses_drivefs_mount_and_paths_inside_overlays() {
        let h = host();
        let fixed = vol("C:\\", "AAAA-0001", "", DriveType::Fixed);
        for root in [
            r"G:\",
            r"G:\My Drive",
            r"C:\Users\example\OneDrive",
            r"c:\users\example\onedrive\Clips",
            r"C:\Users\example\AppData\Local\Google\DriveFS\cache",
        ] {
            match check_root(root, &h.roots, Some(&fixed), Hydration::LocalFull, true) {
                Err(Error::OverlayRoot { .. }) => {}
                other => panic!("{root}: expected OverlayRoot, got {other:?}"),
            }
        }
    }

    #[test]
    fn allows_broad_root_containing_overlays() {
        let h = host();
        let fixed = vol("C:\\", "AAAA-0001", "", DriveType::Fixed);
        check_root(r"C:\", &h.roots, Some(&fixed), Hydration::LocalFull, true).unwrap();
        let d = vol("D:\\", "AAAA-0002", "Data", DriveType::Fixed);
        check_root(r"D:\Clips", &h.roots, Some(&d), Hydration::LocalFull, true).unwrap();
    }

    #[test]
    fn refuses_cloud_volume_and_placeholder_root() {
        let h = HostOverlays::default();
        let gdrive = vol("H:\\", "1", "Google Drive", DriveType::Fixed);
        assert!(matches!(
            check_root(r"H:\x", &h.roots, Some(&gdrive), Hydration::LocalFull, true),
            Err(Error::OverlayRoot { .. })
        ));
        let d = vol("D:\\", "2", "Data", DriveType::Fixed);
        assert!(matches!(
            check_root(r"D:\x", &h.roots, Some(&d), Hydration::Placeholder, true),
            Err(Error::OverlayRoot { .. })
        ));
        // A volume whose mount is itself a registered overlay.
        let g = vol("G:\\", "3", "", DriveType::Fixed);
        assert!(matches!(
            check_root(
                r"\\?\Volume{x}\y",
                &host().roots,
                Some(&g),
                Hydration::LocalFull,
                true
            ),
            Err(Error::OverlayRoot { .. })
        ));
    }

    #[test]
    fn resolve_same_serial_moved_letter_and_offline() {
        let c = cfg(r"E:\Media", r"E:\", "BBBB-0002");
        let here = vol(r"E:\", "BBBB-0002", "ext", DriveType::Removable);
        let r = resolve_root_with(&c, Some(&here), &[]).unwrap();
        assert_eq!(r.root, PathBuf::from(r"E:\Media"));
        assert!(!r.moved);

        // Unplugged: fail closed, never pretend the disk is there.
        match resolve_root_with(&c, None, &[vol(r"C:\", "AAAA-0001", "", DriveType::Fixed)]) {
            Err(Error::VolumeOffline { serial, .. }) => assert_eq!(serial, "BBBB-0002"),
            other => panic!("expected VolumeOffline, got {other:?}"),
        }

        // A different disk now sits at E: → not ours.
        let other = vol(r"E:\", "CCCC-0003", "", DriveType::Removable);
        assert!(matches!(
            resolve_root_with(&c, Some(&other), std::slice::from_ref(&other)),
            Err(Error::VolumeOffline { .. })
        ));

        // A DriveFS letter with a colliding serial is never adopted.
        let mut fake = vol(r"G:\", "BBBB-0002", "Google Drive", DriveType::Fixed);
        fake.fs_name = "FAT32".into();
        assert!(matches!(
            resolve_root_with(&c, None, &[fake]),
            Err(Error::VolumeOffline { .. })
        ));
    }

    #[test]
    fn resolve_rebases_unix_style_mounts() {
        let c = cfg("/media/old/clips", "/media/old", "dev:801");
        let moved = vol("/media/new", "dev:801", "", DriveType::Removable);
        let r = resolve_root_with(&c, None, &[moved]).unwrap();
        assert_eq!(r.root, PathBuf::from("/media/new/clips"));
        assert!(r.moved);
    }

    #[test]
    fn detector_names_are_catalog_strings() {
        let h = host();
        let mut names: Vec<_> = h.roots.iter().map(|r| r.detector.as_str()).collect();
        names.sort();
        assert_eq!(
            names,
            vec!["drivefs_cache", "drivefs_mount", "onedrive_userfolder"]
        );
        assert!(h.installed.drivefs && h.installed.onedrive);
        assert_eq!(Detector::CloudFilterVolume.as_str(), "cloud_filter_volume");
    }

    #[test]
    fn relative_paths_use_forward_slashes() {
        let root = Path::new("root");
        assert_eq!(
            relative(root, &root.join("a").join("b.mp4")).as_deref(),
            Some("a/b.mp4")
        );
        assert_eq!(relative(root, Path::new("elsewhere")), None);
        assert_eq!(location_id("ext-media", "BBBB-0002"), "ext-media@BBBB-0002");
    }
}
