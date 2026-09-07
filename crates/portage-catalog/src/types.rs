//! Row types and the small closed enums stored as TEXT in SQLite.

use std::fmt;
use std::str::FromStr;

use portage_core::ids::{BlobId, ContentId};
use portage_core::Error;

/// `files.kind`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    /// Regular byte stream. Gets a proto-blob on insert.
    Byte,
    /// Directory. No blob, no replica.
    Directory,
    /// Cloud shortcut. Not a replica.
    Shortcut,
}

impl FileKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Byte => "byte",
            Self::Directory => "directory",
            Self::Shortcut => "shortcut",
        }
    }

    pub(crate) fn is_dir(self) -> bool {
        matches!(self, Self::Directory)
    }
}

impl fmt::Display for FileKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for FileKind {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "byte" => Ok(Self::Byte),
            "directory" => Ok(Self::Directory),
            "shortcut" => Ok(Self::Shortcut),
            other => Err(Error::Catalog(format!("unknown file kind: {other}"))),
        }
    }
}

/// `files.hydration`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hydration {
    /// Local bytes we may open and hash.
    LocalFull,
    /// Overlay / on-demand placeholder. Never opened. Not a replica.
    Placeholder,
    /// Cloud-native object (Drive / OneDrive item).
    CloudNative,
}

impl Hydration {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::LocalFull => "local_full",
            Self::Placeholder => "placeholder",
            Self::CloudNative => "cloud_native",
        }
    }
}

impl fmt::Display for Hydration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Hydration {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "local_full" => Ok(Self::LocalFull),
            "placeholder" => Ok(Self::Placeholder),
            "cloud_native" => Ok(Self::CloudNative),
            other => Err(Error::Catalog(format!("unknown hydration: {other}"))),
        }
    }
}

/// `replicas.state`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplicaState {
    /// Dual-hashed or locally BLAKE3'd. Counts as last-copy.
    Verified,
    /// Proto-blob: listed but not yet hashed. Does not count as last-copy.
    Suspect,
    /// Partial transfer. Not a replica for last-copy.
    Partial,
}

impl ReplicaState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Suspect => "suspect",
            Self::Partial => "partial",
        }
    }
}

impl fmt::Display for ReplicaState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ReplicaState {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "verified" => Ok(Self::Verified),
            "suspect" => Ok(Self::Suspect),
            "partial" => Ok(Self::Partial),
            other => Err(Error::Catalog(format!("unknown replica state: {other}"))),
        }
    }
}

/// `providers.kind`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    /// Local volume (fixed or removable).
    Local,
    /// Google Drive.
    GoogleDrive,
    /// Personal OneDrive.
    OneDrive,
}

impl ProviderKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::GoogleDrive => "google_drive",
            Self::OneDrive => "onedrive",
        }
    }
}

impl fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ProviderKind {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "local" => Ok(Self::Local),
            "google_drive" => Ok(Self::GoogleDrive),
            "onedrive" => Ok(Self::OneDrive),
            other => Err(Error::Catalog(format!("unknown provider kind: {other}"))),
        }
    }
}

/// `locations.kind`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocationKind {
    /// A local volume (identified by serial).
    Volume,
    /// A cloud account root.
    Cloud,
}

impl LocationKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Volume => "volume",
            Self::Cloud => "cloud",
        }
    }
}

impl fmt::Display for LocationKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for LocationKind {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "volume" => Ok(Self::Volume),
            "cloud" => Ok(Self::Cloud),
            other => Err(Error::Catalog(format!("unknown location kind: {other}"))),
        }
    }
}

/// `scans.status`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanStatus {
    /// Scan is in progress.
    Running,
    /// Scan finished cleanly.
    Ok,
    /// Scan finished with an error.
    Error,
}

impl ScanStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Ok => "ok",
            Self::Error => "error",
        }
    }
}

impl fmt::Display for ScanStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ScanStatus {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "running" => Ok(Self::Running),
            "ok" => Ok(Self::Ok),
            "error" => Ok(Self::Error),
            other => Err(Error::Catalog(format!("unknown scan status: {other}"))),
        }
    }
}

/// A `files` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRow {
    /// Row id.
    pub id: i64,
    /// Owning location.
    pub location_id: String,
    /// Parent directory file id, if any.
    pub parent_id: Option<i64>,
    /// Provider-relative path, no `..`.
    pub path: String,
    /// Basename.
    pub name: String,
    /// Byte / directory / shortcut.
    pub kind: FileKind,
    /// Set iff `kind == Shortcut`.
    pub shortcut_target_ref: Option<String>,
    /// Size in bytes (byte files).
    pub size: Option<i64>,
    /// Last-write UTC (RFC3339) if known.
    pub mtime_utc: Option<String>,
    /// NTFS file id (local only).
    pub ntfs_file_id: Option<String>,
    /// Volume serial (local only).
    pub volume_serial: Option<String>,
    /// MIME if known.
    pub mime: Option<String>,
    /// Hydration / overlay state.
    pub hydration: Hydration,
    /// Provider item id if known.
    pub remote_ref: Option<String>,
    /// Last scan that saw this path.
    pub last_scan_id: Option<i64>,
}

/// A `blobs` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobRow {
    /// Row id.
    pub id: BlobId,
    /// BLAKE3 identity; `None` for a proto-blob.
    pub content_id: Option<ContentId>,
    /// Size in bytes.
    pub size: i64,
    /// MIME if known.
    pub mime: Option<String>,
}

/// A `replicas` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaRow {
    /// Row id.
    pub id: i64,
    /// Blob this replica claims to be.
    pub blob_id: BlobId,
    /// File this replica is attached to.
    pub file_id: i64,
    /// verified / suspect / partial.
    pub state: ReplicaState,
}

/// A `capacity_snapshots` insert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapacitySnapshot {
    /// Location measured.
    pub location_id: String,
    /// Volume / quota total, if known.
    pub total_bytes: Option<i64>,
    /// Used bytes.
    pub used_bytes: i64,
    /// Free bytes (what the planner consumes).
    pub free_bytes: i64,
    /// Cloud quota, if known.
    pub quota_bytes: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enums_round_trip() {
        for kind in [FileKind::Byte, FileKind::Directory, FileKind::Shortcut] {
            assert_eq!(kind.to_string().parse::<FileKind>().unwrap(), kind);
        }
        for h in [
            Hydration::LocalFull,
            Hydration::Placeholder,
            Hydration::CloudNative,
        ] {
            assert_eq!(h.to_string().parse::<Hydration>().unwrap(), h);
        }
        for s in [
            ReplicaState::Verified,
            ReplicaState::Suspect,
            ReplicaState::Partial,
        ] {
            assert_eq!(s.to_string().parse::<ReplicaState>().unwrap(), s);
        }
        assert!("nope".parse::<FileKind>().is_err());
        assert!("nope".parse::<Hydration>().is_err());
        assert!("nope".parse::<ReplicaState>().is_err());
        assert!("nope".parse::<ProviderKind>().is_err());
        assert!("nope".parse::<LocationKind>().is_err());
        assert!("nope".parse::<ScanStatus>().is_err());
        assert_eq!(ProviderKind::GoogleDrive.to_string(), "google_drive");
        assert_eq!(LocationKind::Volume.to_string(), "volume");
        assert_eq!(ScanStatus::Running.to_string(), "running");
    }
}
