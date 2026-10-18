//! PR 4 integration: temp dir of files → index → catalog rows.

use std::path::Path;

use portage_catalog::{Catalog, FileKind, Hydration, LockMode, ReplicaState};
use portage_core::ids::ProviderId;
use portage_core::Error;
use portage_engine::index::{index_local, record_capacity, BATCH};
use portage_providers::local::pathcmp::strip_verbatim;
use portage_providers::local::sync_roots::{Detector, OverlayRoot};
use portage_providers::registry::{add_local, load_local};
use portage_providers::{HostOverlays, LocalProvider};
use tempfile::TempDir;

fn write(root: &Path, rel: &str, bytes: &[u8]) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, bytes).unwrap();
}

fn files_tree() -> TempDir {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "clips/2026/run.mp4", b"0123456789");
    write(tmp.path(), "clips/2026/boss.mp4", b"01234");
    write(tmp.path(), "notes.txt", b"n");
    write(tmp.path(), "Synced/cloud.mp4", b"must-not-be-indexed");
    tmp
}

/// A host whose only overlay is `<tree>/Synced` (synthetic OneDrive root).
fn host(tree: &Path) -> HostOverlays {
    let canonical = strip_verbatim(tree.canonicalize().unwrap());
    HostOverlays {
        roots: vec![OverlayRoot {
            path: canonical.join("Synced"),
            provider_id: "overlay:onedrive".into(),
            detector: Detector::OnedriveUserfolder,
        }],
        installed: Default::default(),
    }
}

fn open_catalog() -> (TempDir, Catalog) {
    let tmp = TempDir::new().unwrap();
    let cat = Catalog::open(tmp.path(), LockMode::Exclusive).unwrap();
    (tmp, cat)
}

#[test]
fn index_temp_dir_writes_file_rows_and_skips_overlay() {
    let tree = files_tree();
    let host = host(tree.path());
    let (_db, cat) = open_catalog();
    let mut p = add_local(&cat, "local-tmp", tree.path(), None, &host).unwrap();

    let report = index_local(&cat, &mut p).unwrap();
    assert_eq!(report.files, 3);
    assert_eq!(report.dirs, 2, "clips, clips/2026");
    assert_eq!(report.skipped_overlays, 1);
    assert_eq!(report.placeholders, 0);
    assert_eq!(report.moved_to, None);

    let loc = p.location_id();
    let rows = cat.files_for_location(&loc).unwrap();
    let paths: Vec<_> = rows.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(
        paths,
        vec![
            "clips",
            "clips/2026",
            "clips/2026/boss.mp4",
            "clips/2026/run.mp4",
            "notes.txt"
        ]
    );
    assert!(!paths.iter().any(|p| p.contains("Synced")));

    let run = cat.file_by_path(&loc, "clips/2026/run.mp4").unwrap().unwrap();
    let dir = cat.file_by_path(&loc, "clips/2026").unwrap().unwrap();
    let clips = cat.file_by_path(&loc, "clips").unwrap().unwrap();
    assert_eq!(run.kind, FileKind::Byte);
    assert_eq!(run.size, Some(10));
    assert_eq!(run.hydration, Hydration::LocalFull);
    assert_eq!(run.parent_id, Some(dir.id));
    assert_eq!(dir.parent_id, Some(clips.id));
    assert_eq!(clips.parent_id, None);
    assert_eq!(run.last_scan_id, Some(report.scan_id));
    assert_eq!(
        run.volume_serial.as_deref(),
        Some(p.config().volume_serial.as_str())
    );
    assert!(run.mtime_utc.is_some());
    assert!(run.ntfs_file_id.is_some());

    // Proto-blob + suspect replica: listed, not verified, not last-copy.
    let rep = cat.replica_for_file(run.id).unwrap().unwrap();
    assert_eq!(rep.state, ReplicaState::Suspect);
    assert!(cat.replica_for_file(dir.id).unwrap().is_none());
    let counts = cat.counts().unwrap();
    assert_eq!((counts.files, counts.blobs, counts.replicas), (5, 3, 3));

    // Overlay recorded for doctor.
    assert_eq!(cat.overlay_roots().unwrap().len(), 1);
}

#[test]
fn reindex_is_idempotent_and_picks_up_changes() {
    let tree = files_tree();
    let host = host(tree.path());
    let (_db, cat) = open_catalog();
    let mut p = add_local(&cat, "local-tmp", tree.path(), None, &host).unwrap();
    let first = index_local(&cat, &mut p).unwrap();
    let before = cat.counts().unwrap();

    let second = index_local(&cat, &mut p).unwrap();
    assert_ne!(first.scan_id, second.scan_id);
    assert_eq!(cat.counts().unwrap(), before, "unchanged tree, same rows");

    write(tree.path(), "clips/new.mp4", b"new");
    write(tree.path(), "notes.txt", b"longer notes");
    let third = index_local(&cat, &mut p).unwrap();
    assert_eq!(third.files, 4);
    let loc = p.location_id();
    let notes = cat.file_by_path(&loc, "notes.txt").unwrap().unwrap();
    assert_eq!(notes.size, Some(12));
    assert_eq!(notes.last_scan_id, Some(third.scan_id));
    assert!(cat.file_by_path(&loc, "clips/new.mp4").unwrap().is_some());
}

#[test]
fn index_more_than_one_batch() {
    let tree = TempDir::new().unwrap();
    for d in 0..3 {
        for f in 0..(BATCH / 2 + 7) {
            write(tree.path(), &format!("d{d}/f{f:04}.bin"), b"x");
        }
    }
    let (_db, cat) = open_catalog();
    let mut p = add_local(&cat, "bulk", tree.path(), None, &HostOverlays::default()).unwrap();
    let report = index_local(&cat, &mut p).unwrap();
    let expected = 3 * (BATCH / 2 + 7);
    assert_eq!(report.files as usize, expected);
    let rows = cat.files_for_location(&p.location_id()).unwrap();
    assert_eq!(rows.len(), expected + 3);
    let dirs: std::collections::HashMap<_, _> = rows
        .iter()
        .filter(|r| r.kind == FileKind::Directory)
        .map(|r| (r.path.clone(), r.id))
        .collect();
    for r in rows.iter().filter(|r| r.kind == FileKind::Byte) {
        let parent = r.path.rsplit_once('/').unwrap().0;
        assert_eq!(r.parent_id, Some(dirs[parent]), "{}", r.path);
    }
}

#[test]
fn offline_volume_fails_closed_without_a_scan_row() {
    let tree = files_tree();
    let (_db, cat) = open_catalog();
    let host = HostOverlays::default();
    let added = add_local(&cat, "ext", tree.path(), None, &host).unwrap();
    let mut cfg = added.config().clone();
    cfg.volume_serial = "DEAD-BEEF".into();
    cfg.root = tree.path().join("unplugged");
    let mut gone = LocalProvider::new(ProviderId::new("ext"), cfg, Vec::new());
    assert!(matches!(
        index_local(&cat, &mut gone),
        Err(Error::VolumeOffline { .. })
    ));
    assert!(matches!(
        record_capacity(&cat, &gone),
        Err(Error::VolumeOffline { .. })
    ));
    assert_eq!(cat.counts().unwrap().files, 0);
}

#[test]
fn capacity_snapshot_is_recorded() {
    let tree = files_tree();
    let (_db, cat) = open_catalog();
    add_local(&cat, "local-tmp", tree.path(), None, &HostOverlays::default()).unwrap();
    let p = load_local(&cat, "local-tmp", &HostOverlays::default()).unwrap();
    let cap = record_capacity(&cat, &p).unwrap();
    let snap = cat.latest_capacity(&p.location_id()).unwrap().unwrap();
    assert_eq!(snap.free_bytes as u64, cap.free_bytes);
    assert!(snap.total_bytes.unwrap() > 0);
}
