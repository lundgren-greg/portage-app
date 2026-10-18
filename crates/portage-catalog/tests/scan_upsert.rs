//! PR 4 catalog surface: provider config, re-scan upsert, overlay roots.

use portage_catalog::{
    Catalog, FileKind, Hydration, LocationKind, LockMode, NewFile, ProviderKind, ReplicaState,
};
use tempfile::TempDir;

fn seeded() -> (TempDir, Catalog) {
    let tmp = TempDir::new().unwrap();
    let cat = Catalog::open(tmp.path(), LockMode::Exclusive).unwrap();
    cat.upsert_provider_config(
        "local-d",
        ProviderKind::Local,
        None,
        r#"{"root":"D:\\Clips"}"#,
    )
    .unwrap();
    cat.upsert_location(
        "local-d@1A2B-3C4D",
        "local-d",
        LocationKind::Volume,
        Some("Data"),
        Some(r"D:\Clips"),
    )
    .unwrap();
    (tmp, cat)
}

fn file(path: &str, kind: FileKind, size: Option<i64>, mtime: &str, h: Hydration) -> NewFile {
    NewFile {
        location_id: "local-d@1A2B-3C4D".into(),
        parent_id: None,
        path: path.into(),
        name: path.rsplit('/').next().unwrap().into(),
        kind,
        shortcut_target_ref: None,
        size,
        mtime_utc: Some(mtime.into()),
        ntfs_file_id: Some("0000000100000002".into()),
        volume_serial: Some("1A2B-3C4D".into()),
        mime: None,
        hydration: h,
        remote_ref: None,
        last_scan_id: None,
        checksums: Vec::new(),
    }
}

#[test]
fn provider_config_round_trips_and_updates() {
    let (_tmp, cat) = seeded();
    let row = cat.provider("local-d").unwrap().unwrap();
    assert_eq!(row.kind, ProviderKind::Local);
    assert!(row.config_json.contains("Clips"));
    cat.upsert_provider_config("local-d", ProviderKind::Local, None, "{}")
        .unwrap();
    assert_eq!(cat.provider("local-d").unwrap().unwrap().config_json, "{}");
    assert!(cat.provider("nope").unwrap().is_none());
    assert_eq!(cat.providers().unwrap().len(), 1);
}

#[test]
fn rescan_unchanged_keeps_ids_and_replica() {
    let (_tmp, cat) = seeded();
    let scan1 = cat.start_scan("local-d").unwrap();
    let mut a = file("a.mp4", FileKind::Byte, Some(10), "t1", Hydration::LocalFull);
    a.last_scan_id = Some(scan1);
    let ids1 = cat.upsert_scanned_files(std::slice::from_ref(&a)).unwrap();
    let r1 = cat.replica_for_file(ids1[0]).unwrap().unwrap();
    assert_eq!(r1.state, ReplicaState::Suspect);

    let scan2 = cat.start_scan("local-d").unwrap();
    a.last_scan_id = Some(scan2);
    let ids2 = cat.upsert_scanned_files(&[a]).unwrap();
    assert_eq!(ids1, ids2);
    let r2 = cat.replica_for_file(ids2[0]).unwrap().unwrap();
    assert_eq!(r1, r2, "unchanged file keeps its blob and replica");
    let row = cat.file_by_path("local-d@1A2B-3C4D", "a.mp4").unwrap().unwrap();
    assert_eq!(row.last_scan_id, Some(scan2));
    assert_eq!(cat.counts().unwrap().files, 1);
}

#[test]
fn rescan_changed_file_gets_fresh_proto_blob() {
    let (_tmp, cat) = seeded();
    let a = file("a.mp4", FileKind::Byte, Some(10), "t1", Hydration::LocalFull);
    let id = cat.upsert_scanned_files(&[a]).unwrap()[0];
    let before = cat.replica_for_file(id).unwrap().unwrap();

    let a2 = file("a.mp4", FileKind::Byte, Some(20), "t2", Hydration::LocalFull);
    assert_eq!(cat.upsert_scanned_files(&[a2]).unwrap()[0], id);
    let after = cat.replica_for_file(id).unwrap().unwrap();
    assert_eq!(after.id, before.id);
    assert_ne!(after.blob_id, before.blob_id);
    assert_eq!(after.state, ReplicaState::Suspect);
    let row = cat.file_by_path("local-d@1A2B-3C4D", "a.mp4").unwrap().unwrap();
    assert_eq!(row.size, Some(20));
}

#[test]
fn file_that_dehydrates_becomes_partial_not_replica() {
    let (_tmp, cat) = seeded();
    let a = file("a.mp4", FileKind::Byte, Some(10), "t1", Hydration::LocalFull);
    let id = cat.upsert_scanned_files(&[a]).unwrap()[0];
    let p = file("a.mp4", FileKind::Byte, Some(10), "t1", Hydration::Placeholder);
    cat.upsert_scanned_files(&[p]).unwrap();
    let r = cat.replica_for_file(id).unwrap().unwrap();
    assert_eq!(r.state, ReplicaState::Partial);
    let row = cat.file_by_path("local-d@1A2B-3C4D", "a.mp4").unwrap().unwrap();
    assert_eq!(row.hydration, Hydration::Placeholder);
}

#[test]
fn file_replaced_by_directory_drops_replica() {
    let (_tmp, cat) = seeded();
    let a = file("x", FileKind::Byte, Some(1), "t1", Hydration::LocalFull);
    let id = cat.upsert_scanned_files(&[a]).unwrap()[0];
    let d = file("x", FileKind::Directory, None, "t2", Hydration::LocalFull);
    cat.upsert_scanned_files(&[d]).unwrap();
    assert!(cat.replica_for_file(id).unwrap().is_none());
    let rows = cat.files_for_location("local-d@1A2B-3C4D").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, FileKind::Directory);

    // And back to a byte file: a fresh replica appears.
    let a = file("x", FileKind::Byte, Some(1), "t3", Hydration::LocalFull);
    cat.upsert_scanned_files(&[a]).unwrap();
    assert!(cat.replica_for_file(id).unwrap().is_some());
}

#[test]
fn upsert_rejects_traversal_paths() {
    let (_tmp, cat) = seeded();
    let bad = file("../escape", FileKind::Byte, Some(1), "t", Hydration::LocalFull);
    assert!(cat.upsert_scanned_files(&[bad]).is_err());
    assert_eq!(cat.counts().unwrap().files, 0);
}
