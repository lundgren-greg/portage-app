//! The `Provider` trait (design "Provider interface").
//!
//! PR 4 ships the read side that a local volume needs: identity, capacity,
//! listing, stat, and a hydrate-refusing `open_read`. Write, delete (behind a
//! `LastCopyGuard` permit), and ACL methods arrive with the executor and the
//! cloud providers. There is no share-link or public-ACL method, ever (K13).

use std::io::Read;
use std::path::PathBuf;
use std::time::SystemTime;

use portage_catalog::{Hydration, ProviderKind};
use portage_core::ids::ProviderId;
use portage_core::Error;

pub use crate::local::volume::Capacity;

/// Metadata for one listed object. Never obtained by reading file bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectMeta {
    /// Provider-relative path with `/` separators, no `..`.
    pub path: String,
    /// Basename.
    pub name: String,
    /// Directory (no blob, no replica).
    pub is_dir: bool,
    /// Size in bytes (files).
    pub size: Option<u64>,
    /// Last write time, if known.
    pub mtime: Option<SystemTime>,
    /// `LocalFull` only if every byte is on this disk.
    pub hydration: Hydration,
    /// Stable local file identity (NTFS file reference / inode), when known.
    pub file_id: Option<String>,
}

/// One step of a provider listing.
#[derive(Debug)]
pub enum ListItem {
    /// A file or directory to catalog.
    Entry(ObjectMeta),
    /// A registered overlay root inside the walked tree; not descended (K6).
    SkippedOverlay(PathBuf),
    /// A symlink / junction; never followed.
    SkippedLink(PathBuf),
    /// Could not be read (permissions, vanished, non-UTF-8 name). Walk continues.
    Unreadable {
        /// Path that failed.
        path: PathBuf,
        /// Why.
        message: String,
    },
}

/// A storage provider. Implementations must never hydrate a placeholder.
pub trait Provider {
    /// Configured id.
    fn id(&self) -> &ProviderId;

    /// Provider kind.
    fn kind(&self) -> ProviderKind;

    /// Free / total space. Offline volumes fail closed with
    /// [`Error::VolumeOffline`].
    fn capacity(&self) -> Result<Capacity, Error>;

    /// Walk the provider. Must not hydrate placeholders; local skips overlay
    /// roots and links.
    fn list(&self) -> Result<Box<dyn Iterator<Item = ListItem> + '_>, Error>;

    /// Metadata for one provider-relative path (no data read).
    fn stat(&self, path: &str) -> Result<ObjectMeta, Error>;

    /// Open a `LocalFull` file for reading. Placeholders →
    /// [`Error::WouldHydrate`]; the file is never opened.
    fn open_read(&self, path: &str) -> Result<Box<dyn Read + Send>, Error>;
}
