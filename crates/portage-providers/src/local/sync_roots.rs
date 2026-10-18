//! K6 sync-root (overlay) detectors.
//!
//! Enumerates OneDrive and Google Drive for desktop (DriveFS) roots from the
//! registry plus the always-excluded DriveFS cache. Pure over injected
//! [`RegistryReader`] / [`EnvReader`] so unit tests use fakes. The local
//! walker never descends into these trees; cloud providers own them.

use std::path::PathBuf;

use super::pathcmp::{as_absolute_windows_path, is_within, normalize, path_str, same_path};

/// HKCU-relative key for OneDrive.
pub const ONEDRIVE_KEY: &str = r"Software\Microsoft\OneDrive";
/// HKCU-relative key for OneDrive accounts.
pub const ONEDRIVE_ACCOUNTS_KEY: &str = r"Software\Microsoft\OneDrive\Accounts";
/// HKCU-relative key for DriveFS.
pub const DRIVEFS_KEY: &str = r"Software\Google\DriveFS";

/// Read-only view of `HKEY_CURRENT_USER`. Keys are HKCU-relative.
pub trait RegistryReader {
    /// Whether `key` exists.
    fn key_exists(&self, key: &str) -> bool;
    /// Immediate subkey names of `key` (empty if missing).
    fn subkeys(&self, key: &str) -> Vec<String>;
    /// String-typed values (`REG_SZ` / `REG_EXPAND_SZ`) of `key` as
    /// `(name, data)`. The default value has name `""`.
    fn string_values(&self, key: &str) -> Vec<(String, String)>;
}

/// Environment lookup (`LOCALAPPDATA`).
pub trait EnvReader {
    /// Value of `name`, if set and non-empty.
    fn var(&self, name: &str) -> Option<String>;
}

/// Real process environment.
pub struct ProcessEnv;

impl EnvReader for ProcessEnv {
    fn var(&self, name: &str) -> Option<String> {
        std::env::var(name).ok().filter(|v| !v.is_empty())
    }
}

/// A registry with no keys (non-Windows hosts).
pub struct EmptyRegistry;

impl RegistryReader for EmptyRegistry {
    fn key_exists(&self, _key: &str) -> bool {
        false
    }
    fn subkeys(&self, _key: &str) -> Vec<String> {
        Vec::new()
    }
    fn string_values(&self, _key: &str) -> Vec<(String, String)> {
        Vec::new()
    }
}

/// Which detector found an overlay root. Stored as `overlay_roots.detector`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Detector {
    /// `HKCU\…\OneDrive\Accounts\*\UserFolder`.
    OnedriveUserfolder,
    /// DriveFS `DefaultMountPoint` / `Share` / `SyncTargets` / profile paths.
    DrivefsMount,
    /// `%LOCALAPPDATA%\Google\DriveFS` (always excluded).
    DrivefsCache,
    /// A whole volume that is DriveFS / Cloud Filter.
    CloudFilterVolume,
}

impl Detector {
    /// Catalog string.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OnedriveUserfolder => "onedrive_userfolder",
            Self::DrivefsMount => "drivefs_mount",
            Self::DrivefsCache => "drivefs_cache",
            Self::CloudFilterVolume => "cloud_filter_volume",
        }
    }
}

/// Placeholder provider id used before the matching cloud provider is added.
pub const OVERLAY_ONEDRIVE: &str = "overlay:onedrive";
/// Placeholder provider id for DriveFS overlays.
pub const OVERLAY_GOOGLE_DRIVE: &str = "overlay:google-drive";

/// One detected overlay root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayRoot {
    /// Absolute path of the root.
    pub path: PathBuf,
    /// `overlay:onedrive`, `overlay:google-drive`, or a configured provider id.
    pub provider_id: String,
    /// Detector that found it.
    pub detector: Detector,
}

/// Which sync clients appear installed (for `doctor`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InstalledSyncClients {
    /// `HKCU\Software\Microsoft\OneDrive` exists.
    pub onedrive: bool,
    /// `HKCU\Software\Google\DriveFS` exists.
    pub drivefs: bool,
}

impl InstalledSyncClients {
    /// Any sync client present.
    pub fn any(self) -> bool {
        self.onedrive || self.drivefs
    }
}

/// Probe registry for installed sync clients.
pub fn installed_sync_clients(reg: &dyn RegistryReader) -> InstalledSyncClients {
    InstalledSyncClients {
        onedrive: reg.key_exists(ONEDRIVE_KEY),
        drivefs: reg.key_exists(DRIVEFS_KEY),
    }
}

/// Run every K6 detector. Duplicate paths (same location, any case on
/// Windows) are reported once, first detector wins.
pub fn detect_overlay_roots(
    reg: &dyn RegistryReader,
    env: &dyn EnvReader,
    case_insensitive: bool,
) -> Vec<OverlayRoot> {
    let mut out: Vec<OverlayRoot> = Vec::new();
    let mut push = |path: String, provider_id: &str, detector: Detector| {
        // `%LOCALAPPDATA%\..\OneDrive` must compare like the canonical
        // path a root is recorded under, so fold `..` before storing.
        let path = normalize(&path);
        let same = |o: &OverlayRoot| same_path(&path_str(&o.path), &path, case_insensitive);
        if !out.iter().any(same) {
            out.push(OverlayRoot {
                path: PathBuf::from(path),
                provider_id: provider_id.to_string(),
                detector,
            });
        }
    };

    // OneDrive: Accounts\*\UserFolder.
    for account in reg.subkeys(ONEDRIVE_ACCOUNTS_KEY) {
        let key = format!(r"{ONEDRIVE_ACCOUNTS_KEY}\{account}");
        for (name, data) in reg.string_values(&key) {
            if name.eq_ignore_ascii_case("UserFolder") {
                if let Some(path) = as_absolute_windows_path(&expand_env(&data, env)) {
                    push(path, OVERLAY_ONEDRIVE, Detector::OnedriveUserfolder);
                }
            }
        }
    }

    // DriveFS: DefaultMountPoint, Share, per-profile values and SyncTargets.
    // Every string value in the DriveFS tree whose data is an absolute path
    // is an overlay — over-excluding is safe, under-excluding hydrates.
    let mut stack = vec![(DRIVEFS_KEY.to_string(), 0usize)];
    while let Some((key, depth)) = stack.pop() {
        for (_name, data) in reg.string_values(&key) {
            if let Some(path) = as_absolute_windows_path(&expand_env(&data, env)) {
                push(path, OVERLAY_GOOGLE_DRIVE, Detector::DrivefsMount);
            }
        }
        if depth < DRIVEFS_MAX_DEPTH {
            for sub in reg.subkeys(&key) {
                stack.push((format!(r"{key}\{sub}"), depth + 1));
            }
        }
    }

    // Always exclude the DriveFS cache, installed or not.
    if let Some(lad) = env.var("LOCALAPPDATA") {
        let lad = lad.trim_end_matches(['\\', '/']);
        push(
            format!(r"{lad}\Google\DriveFS"),
            OVERLAY_GOOGLE_DRIVE,
            Detector::DrivefsCache,
        );
    }
    out
}

/// How deep below `HKCU\Software\Google\DriveFS` the detector reads
/// (`<profile>\SyncTargets\<n>` is depth 3).
const DRIVEFS_MAX_DEPTH: usize = 4;

/// Expand `%NAME%` (`REG_EXPAND_SZ`) from `env`. Unknown names stay as-is.
fn expand_env(data: &str, env: &dyn EnvReader) -> String {
    let mut out = String::with_capacity(data.len());
    let mut rest = data;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => {
                let name = &after[..end];
                match env.var(name) {
                    Some(value) if !name.is_empty() => out.push_str(&value),
                    _ => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// The first overlay that `path` is equal to or inside of.
pub fn containing_overlay<'a>(
    path: &str,
    overlays: &'a [OverlayRoot],
    case_insensitive: bool,
) -> Option<&'a OverlayRoot> {
    overlays
        .iter()
        .find(|o| is_within(path, &path_str(&o.path), case_insensitive))
}

/// Overlays strictly inside `root` (to auto-register and skip during walk).
pub fn inner_overlays<'a>(
    root: &str,
    overlays: &'a [OverlayRoot],
    case_insensitive: bool,
) -> Vec<&'a OverlayRoot> {
    overlays
        .iter()
        .filter(|o| {
            let p = path_str(&o.path);
            is_within(&p, root, case_insensitive) && !same_path(&p, root, case_insensitive)
        })
        .collect()
}

/// In-memory fakes for unit and integration tests. Not used in production.
#[doc(hidden)]
pub mod fakes {
    use super::*;
    use std::collections::BTreeMap;

    /// In-memory HKCU. Keys are compared case-insensitively like the real one.
    #[derive(Default)]
    pub struct FakeRegistry {
        keys: BTreeMap<String, Vec<(String, String)>>,
    }

    impl FakeRegistry {
        /// Add `key` (and its ancestors).
        pub fn with_key(mut self, key: &str) -> Self {
            let mut acc = String::new();
            for part in key.split('\\') {
                if !acc.is_empty() {
                    acc.push('\\');
                }
                acc.push_str(part);
                self.keys.entry(acc.to_lowercase()).or_default();
            }
            self
        }

        /// Add a string value under `key`.
        pub fn with_value(mut self, key: &str, name: &str, data: &str) -> Self {
            self = self.with_key(key);
            self.keys
                .get_mut(&key.to_lowercase())
                .expect("key just inserted")
                .push((name.to_string(), data.to_string()));
            self
        }
    }

    impl RegistryReader for FakeRegistry {
        fn key_exists(&self, key: &str) -> bool {
            self.keys.contains_key(&key.to_lowercase())
        }
        fn subkeys(&self, key: &str) -> Vec<String> {
            let prefix = format!("{}\\", key.to_lowercase());
            self.keys
                .keys()
                .filter_map(|k| k.strip_prefix(&prefix))
                .filter(|rest| !rest.contains('\\'))
                .map(str::to_string)
                .collect()
        }
        fn string_values(&self, key: &str) -> Vec<(String, String)> {
            self.keys
                .get(&key.to_lowercase())
                .cloned()
                .unwrap_or_default()
        }
    }

    /// In-memory environment.
    #[derive(Default)]
    pub struct FakeEnv(pub BTreeMap<String, String>);

    impl FakeEnv {
        /// Environment with only `LOCALAPPDATA` set.
        pub fn localappdata(path: &str) -> Self {
            let mut m = BTreeMap::new();
            m.insert("LOCALAPPDATA".to_string(), path.to_string());
            Self(m)
        }
    }

    impl EnvReader for FakeEnv {
        fn var(&self, name: &str) -> Option<String> {
            self.0.get(name).cloned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fakes::{FakeEnv, FakeRegistry};
    use super::*;

    const LAD: &str = r"C:\Users\example\AppData\Local";

    fn paths(roots: &[OverlayRoot]) -> Vec<(String, Detector)> {
        let mut out: Vec<_> = roots
            .iter()
            .map(|r| (path_str(&r.path), r.detector))
            .collect();
        out.sort();
        out
    }

    #[test]
    fn no_clients_only_drivefs_cache_is_excluded() {
        let env = FakeEnv::localappdata(LAD);
        let roots = detect_overlay_roots(&FakeRegistry::default(), &env, true);
        assert_eq!(
            paths(&roots),
            vec![(format!(r"{LAD}\Google\DriveFS"), Detector::DrivefsCache)]
        );
        assert_eq!(roots[0].provider_id, OVERLAY_GOOGLE_DRIVE);
    }

    #[test]
    fn no_env_and_no_registry_detects_nothing() {
        let roots = detect_overlay_roots(&EmptyRegistry, &FakeEnv::default(), false);
        assert!(roots.is_empty());
    }

    #[test]
    fn onedrive_userfolder_per_account() {
        let reg = FakeRegistry::default()
            .with_value(
                r"Software\Microsoft\OneDrive\Accounts\Personal",
                "UserFolder",
                r"C:\Users\example\OneDrive",
            )
            .with_value(
                r"Software\Microsoft\OneDrive\Accounts\Business1",
                "UserFolder",
                r"C:\Users\example\OneDrive - Contoso",
            )
            // Non-account subkey without UserFolder is ignored.
            .with_value(
                r"Software\Microsoft\OneDrive\Accounts\FileCoAuth",
                "Version",
                "1",
            );
        let roots = detect_overlay_roots(&reg, &FakeEnv::default(), true);
        assert_eq!(
            paths(&roots),
            vec![
                (
                    r"C:\Users\example\OneDrive".to_string(),
                    Detector::OnedriveUserfolder
                ),
                (
                    r"C:\Users\example\OneDrive - Contoso".to_string(),
                    Detector::OnedriveUserfolder
                ),
            ]
        );
        assert!(roots.iter().all(|r| r.provider_id == OVERLAY_ONEDRIVE));
    }

    #[test]
    fn drivefs_mount_share_sync_targets_and_profile_paths() {
        let reg = FakeRegistry::default()
            .with_value(DRIVEFS_KEY, "DefaultMountPoint", "G:")
            .with_value(DRIVEFS_KEY, "Share", r"H:\Shared drives")
            .with_value(DRIVEFS_KEY, "PerAccountPreferences", "{json}")
            .with_value(&format!(r"{DRIVEFS_KEY}\Share"), "Team", r"I:\")
            .with_value(
                &format!(r"{DRIVEFS_KEY}\1234567890\SyncTargets"),
                "0",
                r"D:\Photos\Synced",
            )
            .with_value(
                &format!(r"{DRIVEFS_KEY}\1234567890\SyncTargets\0"),
                "path",
                r"E:\Backup\Docs",
            )
            .with_value(
                &format!(r"{DRIVEFS_KEY}\1234567890"),
                "MirrorRoot",
                r"C:\Users\example\My Drive",
            )
            .with_value(&format!(r"{DRIVEFS_KEY}\1234567890"), "Flag", "1");
        let roots = detect_overlay_roots(&reg, &FakeEnv::default(), true);
        let mut want = vec![
            (r"G:\".to_string(), Detector::DrivefsMount),
            (r"H:\Shared drives".to_string(), Detector::DrivefsMount),
            (r"I:\".to_string(), Detector::DrivefsMount),
            (r"D:\Photos\Synced".to_string(), Detector::DrivefsMount),
            (r"E:\Backup\Docs".to_string(), Detector::DrivefsMount),
            (
                r"C:\Users\example\My Drive".to_string(),
                Detector::DrivefsMount,
            ),
        ];
        want.sort();
        assert_eq!(paths(&roots), want);
        assert!(roots.iter().all(|r| r.provider_id == OVERLAY_GOOGLE_DRIVE));
    }

    #[test]
    fn expand_sz_values_are_expanded() {
        let env = FakeEnv::localappdata(LAD);
        assert_eq!(
            expand_env(r"%LOCALAPPDATA%\Google\DriveFS", &env),
            format!(r"{LAD}\Google\DriveFS")
        );
        assert_eq!(expand_env("%NOPE%x", &env), "%NOPE%x");
        assert_eq!(expand_env("50% off", &env), "50% off");
        assert_eq!(expand_env("%%", &env), "%%");
        let reg = FakeRegistry::default().with_value(
            r"Software\Microsoft\OneDrive\Accounts\Personal",
            "UserFolder",
            r"%LOCALAPPDATA%\..\OneDrive",
        );
        let roots = detect_overlay_roots(&reg, &env, true);
        let od = roots
            .iter()
            .find(|r| r.detector == Detector::OnedriveUserfolder)
            .unwrap();
        // Stored folded, so it excludes the canonical target.
        assert_eq!(path_str(&od.path), r"C:\Users\example\AppData\OneDrive");
        let canonical = r"C:\Users\example\AppData\OneDrive\Clips";
        assert!(containing_overlay(canonical, &roots, true).is_some());
    }

    #[test]
    fn duplicates_collapse_case_insensitively() {
        let reg = FakeRegistry::default()
            .with_value(DRIVEFS_KEY, "DefaultMountPoint", "G:")
            .with_value(&format!(r"{DRIVEFS_KEY}\p1"), "MountPoint", r"g:\");
        let roots = detect_overlay_roots(&reg, &FakeEnv::default(), true);
        assert_eq!(roots.len(), 1, "{roots:?}");
    }

    #[test]
    fn installed_clients_follow_registry_keys() {
        assert!(!installed_sync_clients(&EmptyRegistry).any());
        let reg = FakeRegistry::default().with_key(ONEDRIVE_KEY);
        let got = installed_sync_clients(&reg);
        assert!(got.onedrive && !got.drivefs && got.any());
        let reg = FakeRegistry::default().with_key(DRIVEFS_KEY);
        assert!(installed_sync_clients(&reg).drivefs);
    }

    #[test]
    fn containing_and_inner_overlays() {
        let reg = FakeRegistry::default()
            .with_value(
                r"Software\Microsoft\OneDrive\Accounts\Personal",
                "UserFolder",
                r"C:\Users\example\OneDrive",
            )
            .with_value(DRIVEFS_KEY, "DefaultMountPoint", "G:");
        let roots = detect_overlay_roots(&reg, &FakeEnv::localappdata(LAD), true);

        // Root IS an overlay / inside one → refused by caller.
        assert!(containing_overlay(r"G:\", &roots, true).is_some());
        assert!(containing_overlay(r"g:\My Drive\clips", &roots, true).is_some());
        assert!(containing_overlay(r"C:\Users\example\OneDrive\Videos", &roots, true).is_some());
        assert!(containing_overlay(r"C:\", &roots, true).is_none());
        assert!(containing_overlay(r"D:\Clips", &roots, true).is_none());

        // Broad root contains overlays → skipped, not refused.
        let inner: Vec<_> = inner_overlays(r"C:\", &roots, true)
            .into_iter()
            .map(|o| path_str(&o.path))
            .collect();
        assert!(inner.contains(&r"C:\Users\example\OneDrive".to_string()));
        assert!(inner.contains(&format!(r"{LAD}\Google\DriveFS")));
        assert!(!inner.iter().any(|p| p.starts_with('G')));
        assert!(inner_overlays(r"D:\", &roots, true).is_empty());
    }
}
