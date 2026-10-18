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

use std::collections::VecDeque;
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
    let offline = || Error::VolumeOffline {
        label: config.volume_label.clone(),
        serial: config.volume_serial.clone(),
        root: config.root.clone(),
    };
    if let Some(v) = probed {
        if v.serial.eq_ignore_ascii_case(&config.volume_serial) {
            // A serial collision with a DriveFS / Cloud Filter volume is
            // never our disk, wherever it is mounted.
            if v.cloud_volume_reason().is_some() {
                return Err(offline());
            }
            return Ok(Resolved {
                root: config.root.clone(),
                mount: v.mount.clone(),
                moved: false,
            });
        }
    }
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

    /// `rel` resolved for `stat` / `open_read`: the volume is resolved once
    /// (a different disk at the old letter is never read, K24), the path
    /// must not be (inside) an overlay (K6), and every component from the
    /// root down is `lstat`ed: a symlink / junction anywhere, a placeholder
    /// directory on the way, or (Unix) another filesystem is refused.
    /// Returns `(full path, metadata of the final component)`.
    fn checked(&self, rel: &str) -> Result<(PathBuf, std::fs::Metadata), Error> {
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
        let refuse = |path: &Path, reason: &str| Error::OverlayRoot {
            path: path.to_path_buf(),
            reason: reason.to_string(),
        };
        // Overlay trees belong to their cloud provider; never touch them.
        if self.in_overlay(&full) {
            return Err(refuse(&full, "inside an overlay root"));
        }
        let lstat = |p: &Path| {
            std::fs::symlink_metadata(p).map_err(|source| Error::Io {
                path: p.to_path_buf(),
                source,
            })
        };
        let root_meta = lstat(&root)?;
        let mut current = root.clone();
        let mut meta = root_meta.clone();
        for part in candidate.components() {
            let Component::Normal(part) = part else {
                continue;
            };
            // The previous component is about to be traversed.
            if placeholder::hydration_of(&meta) != Hydration::LocalFull {
                return Err(Error::WouldHydrate { path: current });
            }
            current.push(part);
            // Win32 name aliases (`name.`, `NAME~1`) would otherwise
            // reach an overlay the text check above did not match.
            match alias_of_on_disk_name(&current, part) {
                Ok(None) => {}
                Ok(Some(reason)) => return Err(escape(reason)),
                Err(source) => {
                    return Err(Error::Io {
                        path: current,
                        source,
                    })
                }
            }
            meta = lstat(&current)?;
            if meta.file_type().is_symlink() {
                return Err(refuse(&current, "is (under) a symlink or junction"));
            }
            if !same_filesystem(&root_meta, &meta) {
                return Err(refuse(&current, "is on another mounted filesystem"));
            }
        }
        Ok((full, meta))
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
        Ok(Box::new(self.walk()?))
    }

    fn stat(&self, path: &str) -> Result<ObjectMeta, Error> {
        let (full, meta) = self.checked(path)?;
        Ok(self.meta_for(path, &full, &meta))
    }

    fn open_read(&self, path: &str) -> Result<Box<dyn Read + Send>, Error> {
        let (full, raw) = self.checked(path)?;
        // Classify from metadata first; a placeholder is never opened.
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
        let file = open_no_follow(&full).map_err(|source| Error::Io {
            path: full.clone(),
            source,
        })?;
        // The path may have been swapped since it was checked: the handle
        // must be the very object classified above, and still not a
        // reparse point or placeholder.
        if !opened_matches(&file, &raw, meta.file_id.as_deref()) {
            return Err(Error::OverlayRoot {
                path: full,
                reason: "changed between check and open".into(),
            });
        }
        Ok(Box::new(file))
    }
}

impl LocalProvider {
    /// The walk behind [`Provider::list`]. The root is checked like any
    /// other directory before it is opened.
    fn walk(&self) -> Result<LocalWalk<'_>, Error> {
        let root = self.resolve()?.root;
        if self.in_overlay(&root) {
            return Err(Error::OverlayRoot {
                path: root,
                reason: "root is inside an overlay".into(),
            });
        }
        let io = |source| Error::Io {
            path: root.clone(),
            source,
        };
        let meta = std::fs::symlink_metadata(&root).map_err(io)?;
        if meta.file_type().is_symlink() {
            return Err(Error::OverlayRoot {
                path: root,
                reason: "root is a symlink or junction".into(),
            });
        }
        if placeholder::hydration_of(&meta) != Hydration::LocalFull {
            return Err(Error::WouldHydrate { path: root });
        }
        let entries = std::fs::read_dir(&root).map_err(io)?;
        let mut walk = LocalWalk {
            skip: self.overlays.iter().map(|o| path_str(&o.path)).collect(),
            root: root.clone(),
            root_meta: meta,
            stack: Vec::new(),
            pending: VecDeque::new(),
            provider: self,
            #[cfg(test)]
            opened: Vec::new(),
        };
        walk.push_dir(&root, entries);
        Ok(walk)
    }
}

/// `Some(reason)` if `part`, the last component of `path`, is not literally
/// the name stored on disk (case aside). Windows strips a trailing dot or
/// space and maps 8.3 short names to long names, so such a part names some
/// other entry than its text says. Unix names are literal.
fn alias_of_on_disk_name(
    path: &Path,
    part: &std::ffi::OsStr,
) -> std::io::Result<Option<&'static str>> {
    #[cfg(windows)]
    {
        let text = part.to_string_lossy();
        // Wildcards are not legal in a name and would make the lookup a search.
        if text.contains(['*', '?', '<', '>', '"']) {
            return Ok(Some("contains a wildcard character"));
        }
        let real = win::on_disk_name(path)?;
        if real != part && real.to_string_lossy().to_lowercase() != text.to_lowercase() {
            return Ok(Some(
                "is not the name on disk (trailing dot or space, or an 8.3 short name)",
            ));
        }
        Ok(None)
    }
    #[cfg(not(windows))]
    {
        let _ = (path, part);
        Ok(None)
    }
}

/// True if `b` is on the same filesystem as `a`. Windows mount points are
/// reparse points and are already skipped as links.
fn same_filesystem(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        a.dev() == b.dev()
    }
    #[cfg(not(unix))]
    {
        let _ = (a, b);
        true
    }
}

/// Open for reading without following a final-component link. On Windows
/// `FILE_FLAG_OPEN_REPARSE_POINT` opens a link (or a cloud reparse point)
/// itself rather than what it points at; [`opened_matches`] then rejects it.
/// On Unix `O_NOFOLLOW` fails on a link, and `O_NONBLOCK` keeps a FIFO
/// swapped in after the check from blocking before that rejection.
fn open_no_follow(path: &Path) -> std::io::Result<File> {
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        opts.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    opts.open(path)
}

/// The opened handle is the object `expected` / `expected_id` described:
/// same file identity and, on Windows, no reparse / recall / offline bits.
fn opened_matches(file: &File, expected: &std::fs::Metadata, expected_id: Option<&str>) -> bool {
    #[cfg(windows)]
    {
        let _ = expected;
        match (win::handle_identity(file), expected_id) {
            (Some((attributes, id)), Some(want)) => {
                id == want
                    && placeholder::classify_attributes(attributes, false) == Hydration::LocalFull
            }
            _ => false,
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let _ = expected_id;
        file.metadata()
            .is_ok_and(|m| m.is_file() && m.dev() == expected.dev() && m.ino() == expected.ino())
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = (file, expected, expected_id);
        false
    }
}

/// Depth-first walk of a local tree in name order. Every entry is
/// classified from directory-enumeration metadata (no handle is opened),
/// and a directory is handed to `read_dir` only after it passed the
/// overlay, link, placeholder, and same-filesystem checks, so an overlay or
/// placeholder directory is never opened or enumerated.
struct LocalWalk<'a> {
    root: PathBuf,
    root_meta: std::fs::Metadata,
    skip: Vec<String>,
    /// Sorted, not-yet-visited entries of each open directory level.
    stack: Vec<std::vec::IntoIter<std::fs::DirEntry>>,
    /// Errors to report before continuing.
    pending: VecDeque<ListItem>,
    provider: &'a LocalProvider,
    /// Every directory passed to `read_dir`, for tests.
    #[cfg(test)]
    opened: Vec<PathBuf>,
}

impl LocalWalk<'_> {
    fn push_dir(&mut self, dir: &Path, entries: std::fs::ReadDir) {
        #[cfg(test)]
        self.opened.push(dir.to_path_buf());
        let mut ok = Vec::new();
        for e in entries {
            match e {
                Ok(e) => ok.push(e),
                Err(err) => self.pending.push_back(ListItem::Unreadable {
                    path: dir.to_path_buf(),
                    message: err.to_string(),
                }),
            }
        }
        ok.sort_by_key(|e| e.file_name());
        self.stack.push(ok.into_iter());
    }

    fn visit(&mut self, entry: std::fs::DirEntry) -> ListItem {
        let path = entry.path();
        let unreadable = |path: PathBuf, err: std::io::Error| ListItem::Unreadable {
            path,
            message: err.to_string(),
        };
        let ps = path_str(&path);
        if self
            .skip
            .iter()
            .any(|o| is_within(&ps, o, HOST_CASE_INSENSITIVE))
        {
            return ListItem::SkippedOverlay(path);
        }
        let file_type = match entry.file_type() {
            Ok(t) => t,
            Err(err) => return unreadable(path, err),
        };
        if file_type.is_symlink() {
            return ListItem::SkippedLink(path);
        }
        // Directory-enumeration metadata: no file handle is opened.
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(err) => return unreadable(path, err),
        };
        if !same_filesystem(&self.root_meta, &meta) {
            return ListItem::SkippedMount(path);
        }
        let Some(rel) = relative(&self.root, &path) else {
            return ListItem::Unreadable {
                path,
                message: "name is not valid UTF-8".into(),
            };
        };
        // Only now, with every check passed, is the directory opened.
        if meta.is_dir() && placeholder::hydration_of(&meta) == Hydration::LocalFull {
            match std::fs::read_dir(&path) {
                Ok(entries) => self.push_dir(&path, entries),
                Err(err) => self.pending.push_back(unreadable(path.clone(), err)),
            }
        }
        ListItem::Entry(self.provider.meta_for(&rel, &path, &meta))
    }
}

impl Iterator for LocalWalk<'_> {
    type Item = ListItem;

    fn next(&mut self) -> Option<ListItem> {
        loop {
            if let Some(item) = self.pending.pop_front() {
                return Some(item);
            }
            match self.stack.last_mut()?.next() {
                Some(entry) => return Some(self.visit(entry)),
                None => {
                    self.stack.pop();
                }
            }
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
            resolve_root_with(&c, None, std::slice::from_ref(&fake)),
            Err(Error::VolumeOffline { .. })
        ));
        // Same collision at the recorded letter (the serial fast path).
        let mut at_e = fake.clone();
        at_e.mount = PathBuf::from(r"E:\");
        assert!(matches!(
            resolve_root_with(&c, Some(&at_e), &[]),
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
    fn walk_checks_each_directory_before_opening_it() {
        let tmp = tempfile::tempdir().unwrap();
        let root = strip_verbatim(tmp.path().canonicalize().unwrap());
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        std::fs::create_dir_all(root.join("Overlay/inner")).unwrap();
        std::fs::write(root.join("a/b/f.mp4"), b"x").unwrap();
        std::fs::write(root.join("Overlay/inner/s.mp4"), b"x").unwrap();
        let v = volume::probe(&root).unwrap();
        let config = LocalConfig {
            root: root.clone(),
            roles: Role::Both,
            volume_serial: v.serial,
            volume_label: v.label,
            volume_mount: v.mount,
            removable: false,
            bus: None,
            drive_type: "fixed".into(),
        };
        let overlays = vec![OverlayRoot {
            path: root.join("Overlay"),
            provider_id: "overlay:onedrive".into(),
            detector: Detector::OnedriveUserfolder,
        }];
        let p = LocalProvider::new(ProviderId::new("t"), config, overlays);
        let mut walk = p.walk().unwrap();
        let items: Vec<_> = walk.by_ref().collect();
        // Only directories that passed the overlay / link / placeholder
        // checks were ever handed to `read_dir`; the overlay never was.
        let opened: Vec<_> = walk
            .opened
            .iter()
            .map(|d| relative(&root, d).unwrap())
            .collect();
        assert_eq!(opened, vec!["", "a", "a/b"]);
        let paths: Vec<_> = items
            .iter()
            .filter_map(|i| match i {
                ListItem::Entry(m) => Some(m.path.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(paths, vec!["a", "a/b", "a/b/f.mp4"]);
        assert!(matches!(items[0], ListItem::SkippedOverlay(_)));
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

    /// A path swapped for a link after the check is not followed, and a
    /// swapped-in FIFO does not block the open (L1).
    #[cfg(unix)]
    #[test]
    fn open_no_follow_refuses_links_and_does_not_block_on_fifos() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("target.txt");
        std::fs::write(&target, b"x").unwrap();
        let link = tmp.path().join("link.txt");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(open_no_follow(&link).is_err());

        let fifo = tmp.path().join("fifo");
        let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        // SAFETY: NUL-terminated path.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        // Without O_NONBLOCK this blocks until a writer appears.
        let file = open_no_follow(&fifo).unwrap();
        let meta = std::fs::symlink_metadata(&target).unwrap();
        assert!(!opened_matches(&file, &meta, None));
        assert!(open_no_follow(&target).is_ok());
    }
}
