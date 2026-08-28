//! Catalog integration: temp DB, lock, proto-blob, lookups.

use portage_catalog::{
    CapacitySnapshot, Catalog, FileKind, Hydration, LocationKind, LockMode, NewChecksum, NewFile,
    ProviderKind, ReplicaState,
};
use portage_core::ids::ContentId;
use portage_core::Error;
use tempfile::TempDir;

fn exclusive() -> (TempDir, Catalog) {
    let tmp = TempDir::new().unwrap();
    let cat = Catalog::open(tmp.path(), LockMode::Exclusive).unwrap();
    (tmp, cat)
}

fn seed(cat: &Catalog) {
    cat.upsert_provider("local-d", ProviderKind::Local, None)
        .unwrap();
    cat.upsert_location(
        "vol-d",
        "local-d",
        LocationKind::Volume,
        Some("D:"),
        Some("D:\\"),
    )
    .unwrap();
}

fn byte_file(path: &str, name: &str, size: i64) -> NewFile {
    NewFile {
        location_id: "vol-d".into(),
        parent_id: None,
        path: path.into(),
        name: name.into(),
        kind: FileKind::Byte,
        shortcut_target_ref: None,
        size: Some(size),
        mtime_utc: None,
        ntfs_file_id: None,
        volume_serial: Some("ABCD".into()),
        mime: Some("video/mp4".into()),
        hydration: Hydration::LocalFull,
        remote_ref: None,
        last_scan_id: None,
        checksums: Vec::new(),
    }
}

#[test]
fn byte_file_gets_proto_blob_and_suspect_replica() {
    let (_tmp, cat) = exclusive();
    seed(&cat);
    let inserted = cat
        .insert_file(&byte_file("Clips/a.mp4", "a.mp4", 100))
        .unwrap();
    let blob = inserted.blob.expect("proto-blob");
    assert!(blob.content_id.is_none());
    assert_eq!(blob.size, 100);
    let replica = inserted.replica.expect("replica");
    assert_eq!(replica.state, ReplicaState::Suspect);
    let row = cat.file_by_path("vol-d", "Clips/a.mp4").unwrap().unwrap();
    assert_eq!(row.name, "a.mp4");
}

#[test]
fn directory_has_no_replica_placeholder_has_partial_replica() {
    let (_tmp, cat) = exclusive();
    seed(&cat);
    let mut dir = byte_file("Clips", "Clips", 0);
    dir.kind = FileKind::Directory;
    dir.size = None;
    let inserted = cat.insert_file(&dir).unwrap();
    assert!(inserted.blob.is_none());
    assert!(inserted.replica.is_none());

    let mut ph = byte_file("Clips/cloud.mp4", "cloud.mp4", 50);
    ph.hydration = Hydration::Placeholder;
    let inserted = cat.insert_file(&ph).unwrap();
    assert!(inserted.blob.is_some());
    // Placeholder gets a Partial replica to keep the file↔blob link intact
    // without counting as a last-copy.
    let replica = inserted.replica.expect("placeholder should have a Partial replica");
    assert_eq!(replica.state, ReplicaState::Partial);
}

#[test]
fn checksum_binding_reuses_blob() {
    let (_tmp, cat) = exclusive();
    cat.upsert_provider("gdrive", ProviderKind::GoogleDrive, None)
        .unwrap();
    cat.upsert_location("gdrive", "gdrive", LocationKind::Cloud, None, None)
        .unwrap();
    let checksum = NewChecksum {
        provider_id: "gdrive".into(),
        remote_ref: "id-a".into(),
        algo: "md5".into(),
        hex: "d41d8cd98f00b204e9800998ecf8427e".into(),
        size: 50,
    };
    let mut a = byte_file("a.mp4", "a.mp4", 50);
    a.location_id = "gdrive".into();
    a.remote_ref = Some("id-a".into());
    a.checksums = vec![checksum.clone()];
    let first = cat.insert_file(&a).unwrap();

    let mut b = byte_file("b.mp4", "b.mp4", 50);
    b.location_id = "gdrive".into();
    b.remote_ref = Some("id-b".into());
    b.checksums = vec![NewChecksum {
        provider_id: "gdrive".into(),
        remote_ref: "id-b".into(),
        ..checksum
    }];
    let second = cat.insert_file(&b).unwrap();
    assert_eq!(
        first.blob.as_ref().map(|b| b.id),
        second.blob.as_ref().map(|b| b.id)
    );
    assert_eq!(cat.counts().unwrap().blobs, 1);
}

#[test]
fn lookup_by_content_id_after_bind() {
    let (_tmp, cat) = exclusive();
    seed(&cat);
    let inserted = cat
        .insert_file(&byte_file("Clips/a.mp4", "a.mp4", 10))
        .unwrap();
    let blob_id = inserted.blob.unwrap().id;
    let cid: ContentId = "b3:af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        .parse()
        .unwrap();
    cat.set_content_id(blob_id, &cid).unwrap();
    let found = cat.files_by_content_id(&cid).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].path, "Clips/a.mp4");
    assert_eq!(cat.blob_by_content_id(&cid).unwrap().unwrap().id, blob_id);
}

#[test]
fn rejects_parent_traversal() {
    let (_tmp, cat) = exclusive();
    seed(&cat);
    let err = cat
        .insert_file(&byte_file("../secret", "secret", 1))
        .unwrap_err();
    assert!(err.to_string().contains(".."), "{err}");
}

#[test]
fn scan_and_capacity_round_trip() {
    let (_tmp, cat) = exclusive();
    seed(&cat);
    let scan = cat.start_scan("local-d").unwrap();
    cat.finish_scan(scan, 3, portage_catalog::ScanStatus::Ok)
        .unwrap();
    cat.insert_capacity(&CapacitySnapshot {
        location_id: "vol-d".into(),
        total_bytes: Some(1000),
        used_bytes: 400,
        free_bytes: 600,
        quota_bytes: None,
    })
    .unwrap();
    let latest = cat.latest_capacity("vol-d").unwrap().unwrap();
    assert_eq!(latest.free_bytes, 600);
}

#[test]
fn exclusive_blocks_second_open() {
    let tmp = TempDir::new().unwrap();
    let _held = Catalog::open(tmp.path(), LockMode::Exclusive).unwrap();
    assert!(matches!(
        Catalog::open(tmp.path(), LockMode::Exclusive),
        Err(Error::CatalogLocked { .. })
    ));
}

#[test]
fn backup_writes_a_copy() {
    let (_tmp, cat) = exclusive();
    let dest = cat.backup_path_today().unwrap();
    cat.checkpoint_and_backup(&dest).unwrap();
    assert!(dest.exists());
    assert!(dest.metadata().unwrap().len() > 0);
}

#[test]
fn batched_insert_and_schema_flags() {
    let (_tmp, cat) = exclusive();
    seed(&cat);
    let out = cat
        .insert_files(&[
            byte_file("a.mp4", "a.mp4", 1),
            byte_file("b.mp4", "b.mp4", 2),
        ])
        .unwrap();
    assert_eq!(out.len(), 2);
    assert!(cat.plans_schema_ready().unwrap());
    assert!(cat.journal_schema_ready().unwrap());
}
