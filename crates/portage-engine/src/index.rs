//! Local index (PR 4): walk a local provider into the catalog.
//!
//! Metadata only — no file body is read and no placeholder is opened. Every
//! byte file gets a proto-blob and a `suspect` replica (`partial` for a
//! placeholder); hashing to `verified` is PR 5. Rows are written in
//! [`BATCH`]-row transactions.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;

use portage_catalog::{
    CapacitySnapshot, Catalog, FileKind, Hydration, NewFile, ReplicaState, ScanStatus,
};
use portage_core::Error;
use portage_providers::registry::{record_resolved, register_overlays};
use portage_providers::{ListItem, LocalProvider, ObjectMeta, Provider};

/// Rows per catalog transaction.
pub const BATCH: usize = 1000;

/// What one `index` run saw.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IndexReport {
    /// Scan row id.
    pub scan_id: i64,
    /// Location written.
    pub location_id: String,
    /// Byte files cataloged (any hydration).
    pub files: u64,
    /// Directories cataloged.
    pub dirs: u64,
    /// Byte files or directories recorded as `placeholder` (never opened).
    pub placeholders: u64,
    /// Overlay roots inside the tree that were skipped.
    pub skipped_overlays: u64,
    /// Symlinks / junctions skipped.
    pub skipped_links: u64,
    /// Entries that could not be read.
    pub unreadable: u64,
    /// New root if the volume moved to another drive letter.
    pub moved_to: Option<PathBuf>,
}

/// Whether a replica may count toward last-copy protection.
///
/// Only a `verified` replica of fully-local bytes on a mounted volume
/// counts. Suspect / partial replicas, placeholders, and anything on an
/// unplugged volume never do (K5, K6, K24).
pub fn counts_toward_last_copy(
    state: ReplicaState,
    hydration: Hydration,
    volume_online: bool,
) -> bool {
    state == ReplicaState::Verified && hydration == Hydration::LocalFull && volume_online
}

/// Walk `provider` into the catalog. Fails closed with
/// [`Error::VolumeOffline`] before writing anything if the volume is gone.
pub fn index_local(cat: &Catalog, provider: &mut LocalProvider) -> Result<IndexReport, Error> {
    let resolved = provider.resolve()?;
    record_resolved(cat, provider, &resolved)?;
    register_overlays(cat, provider.overlays())?;

    let provider_id = provider.id().as_str().to_string();
    let scan_id = cat.start_scan(&provider_id)?;
    let mut report = IndexReport {
        scan_id,
        location_id: provider.location_id(),
        moved_to: resolved.moved.then(|| resolved.root.clone()),
        ..IndexReport::default()
    };
    match walk(cat, provider, &mut report) {
        Ok(()) => {
            let seen = (report.files + report.dirs) as i64;
            cat.finish_scan(scan_id, seen, ScanStatus::Ok)?;
            tracing::info!(
                provider = %provider_id,
                files = report.files,
                dirs = report.dirs,
                placeholders = report.placeholders,
                skipped_overlays = report.skipped_overlays,
                "local index done"
            );
            Ok(report)
        }
        Err(err) => {
            let seen = (report.files + report.dirs) as i64;
            // Best effort: the original error is what the caller needs.
            let _ = cat.finish_scan(scan_id, seen, ScanStatus::Error);
            Err(err)
        }
    }
}

struct Batch {
    rows: Vec<NewFile>,
    /// `(index in rows, relative path)` of directories awaiting an id.
    dirs: Vec<(usize, String)>,
}

impl Batch {
    fn flush(&mut self, cat: &Catalog, dir_ids: &mut HashMap<String, i64>) -> Result<(), Error> {
        if self.rows.is_empty() {
            return Ok(());
        }
        let ids = cat.upsert_scanned_files(&self.rows)?;
        for (i, rel) in self.dirs.drain(..) {
            dir_ids.insert(rel, ids[i]);
        }
        self.rows.clear();
        Ok(())
    }
}

fn walk(cat: &Catalog, provider: &LocalProvider, report: &mut IndexReport) -> Result<(), Error> {
    let location_id = provider.location_id();
    let serial = provider.config().volume_serial.clone();
    let mut dir_ids: HashMap<String, i64> = HashMap::new();
    let mut batch = Batch {
        rows: Vec::with_capacity(BATCH),
        dirs: Vec::new(),
    };

    for item in provider.list()? {
        let meta = match item {
            ListItem::Entry(meta) => meta,
            ListItem::SkippedOverlay(path) => {
                tracing::info!(path = %path.display(), "skipped overlay root (K6)");
                report.skipped_overlays += 1;
                continue;
            }
            ListItem::SkippedLink(_) => {
                report.skipped_links += 1;
                continue;
            }
            ListItem::Unreadable { path, message } => {
                tracing::warn!(path = %path.display(), %message, "unreadable entry");
                report.unreadable += 1;
                continue;
            }
        };

        let parent_rel = meta.path.rsplit_once('/').map(|(p, _)| p.to_string());
        if let Some(parent) = &parent_rel {
            if batch.dirs.iter().any(|(_, d)| d == parent) {
                batch.flush(cat, &mut dir_ids)?;
            }
        }
        let parent_id = parent_rel.as_ref().and_then(|p| dir_ids.get(p).copied());

        if meta.hydration != Hydration::LocalFull {
            report.placeholders += 1;
        }
        if meta.is_dir {
            report.dirs += 1;
            batch.dirs.push((batch.rows.len(), meta.path.clone()));
        } else {
            report.files += 1;
        }
        batch
            .rows
            .push(new_file(&location_id, &serial, parent_id, report.scan_id, meta)?);
        if batch.rows.len() >= BATCH {
            batch.flush(cat, &mut dir_ids)?;
        }
    }
    batch.flush(cat, &mut dir_ids)
}

fn new_file(
    location_id: &str,
    serial: &str,
    parent_id: Option<i64>,
    scan_id: i64,
    meta: ObjectMeta,
) -> Result<NewFile, Error> {
    let size = match meta.size {
        Some(s) => Some(i64::try_from(s).map_err(|_| {
            Error::Invariant(format!("file size overflows i64: {}", meta.path))
        })?),
        None => None,
    };
    Ok(NewFile {
        location_id: location_id.to_string(),
        parent_id,
        name: meta.name,
        kind: if meta.is_dir {
            FileKind::Directory
        } else {
            FileKind::Byte
        },
        shortcut_target_ref: None,
        size,
        mtime_utc: meta.mtime.and_then(rfc3339),
        ntfs_file_id: meta.file_id,
        volume_serial: Some(serial.to_string()),
        mime: None,
        hydration: meta.hydration,
        remote_ref: None,
        last_scan_id: Some(scan_id),
        checksums: Vec::new(),
        path: meta.path,
    })
}

fn rfc3339(t: SystemTime) -> Option<String> {
    time::OffsetDateTime::from(t)
        .format(&time::format_description::well_known::Rfc3339)
        .ok()
}

/// Measure a local provider's volume and store a capacity snapshot.
pub fn record_capacity(
    cat: &Catalog,
    provider: &LocalProvider,
) -> Result<portage_providers::Capacity, Error> {
    let cap = provider.capacity()?;
    let to_i64 = |v: u64| i64::try_from(v).unwrap_or(i64::MAX);
    cat.insert_capacity(&CapacitySnapshot {
        location_id: provider.location_id(),
        total_bytes: cap.total_bytes.map(to_i64),
        used_bytes: to_i64(cap.used_bytes),
        free_bytes: to_i64(cap.free_bytes),
        quota_bytes: cap.quota_bytes.map(to_i64),
    })?;
    Ok(cap)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_verified_local_full_online_counts_as_last_copy() {
        assert!(counts_toward_last_copy(
            ReplicaState::Verified,
            Hydration::LocalFull,
            true
        ));
        // Unplugged removable volume: never a last copy.
        assert!(!counts_toward_last_copy(
            ReplicaState::Verified,
            Hydration::LocalFull,
            false
        ));
        for h in [Hydration::Placeholder, Hydration::CloudNative] {
            assert!(!counts_toward_last_copy(ReplicaState::Verified, h, true));
        }
        for s in [ReplicaState::Suspect, ReplicaState::Partial] {
            assert!(!counts_toward_last_copy(s, Hydration::LocalFull, true));
        }
    }

    #[test]
    fn rfc3339_formats_epoch() {
        assert_eq!(
            rfc3339(SystemTime::UNIX_EPOCH).as_deref(),
            Some("1970-01-01T00:00:00Z")
        );
    }

    #[test]
    fn new_file_maps_directories_and_placeholders() {
        let meta = ObjectMeta {
            path: "a/b".into(),
            name: "b".into(),
            is_dir: true,
            size: None,
            mtime: None,
            hydration: Hydration::LocalFull,
            file_id: None,
        };
        let f = new_file("loc", "1A2B-3C4D", Some(7), 3, meta).unwrap();
        assert_eq!(f.kind, FileKind::Directory);
        assert_eq!(f.parent_id, Some(7));
        assert_eq!(f.last_scan_id, Some(3));
        assert_eq!(f.volume_serial.as_deref(), Some("1A2B-3C4D"));

        let meta = ObjectMeta {
            path: "c.mp4".into(),
            name: "c.mp4".into(),
            is_dir: false,
            size: Some(u64::MAX),
            mtime: None,
            hydration: Hydration::Placeholder,
            file_id: None,
        };
        assert!(matches!(
            new_file("loc", "s", None, 1, meta),
            Err(Error::Invariant(_))
        ));
    }
}
