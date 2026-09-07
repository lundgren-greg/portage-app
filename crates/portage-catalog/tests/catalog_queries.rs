//! Integration tests for catalog open, insert, lookup, and doctor checks.

use portage_catalog::lock::LockMode;
use portage_catalog::types::{
    CapacitySnapshot, FileKind, Hydration, LocationKind, ProviderKind, ReplicaState, ScanStatus,
};
use portage_catalog::{Catalog, NewFile};
use portage_core::ContentId;
use tempfile::TempDir;

fn open() -> (TempDir, Catalog) {
    let dir = TempDir::new().unwrap();
    let cat = Catalog::open(dir.path(), LockMode::Exclusive).unwrap();
    (dir, cat)
}

fn seed_local(cat: &Catalog) {
    cat.upsert_provider("local-d", ProviderKind::Local, None)
        .unwrap();
    cat.upsert_location(
        "vol-d",
        "local-d",
        LocationKind::Volume,
        Some("D:"),
        Some("D:\\Clips"),
    )
    .unwrap();
}

fn byte_file(path: &str, name: &str, size: i64, hydration: Hydration) -> NewFile {
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
        volume_serial: Some("ABCD1234".into()),
        mime: Some("video/mp4".into()),
        hydration,
        remote_ref: None,
        last_scan_id: None,
        checksums: Vec::new(),
    }
}

#[test]
fn byte_file_gets_proto_blob_and_suspect_replica() {
    let (_dir, cat) = open();
    seed_local(&cat);
    let inserted = cat
        .insert_file(&byte_file(
            "clips/a.mp4",
            "a.mp4",
            1024,
            Hydration::LocalFull,
        ))
        .unwrap();

    assert_eq!(inserted.file.kind, FileKind::Byte);
    let blob = inserted.blob.expect("proto-blob");
    assert!(blob.content_id.is_none());
    assert_eq!(blob.size, 1024);
    let replica = inserted.replica.expect("replica");
    assert_eq!(replica.state, ReplicaState::Suspect);
    assert_eq!(replica.file_id, inserted.file.id);

    let found = cat.file_by_path("vol-d", "clips/a.mp4").unwrap().unwrap();
    assert_eq!(found.id, inserted.file.id);
    assert_eq!(found.name, "a.mp4");
}

#[test]
fn placeholder_gets_proto_blob_and_partial_replica() {
    let (_dir, cat) = open();
    seed_local(&cat);
    let inserted = cat
        .insert_file(&byte_file(
            "OneDrive/clip.mp4",
            "clip.mp4",
            99,
            Hydration::Placeholder,
        ))
        .unwrap();
    assert!(inserted.blob.is_some());
    // Placeholder links to blob via a Partial replica (not last-copy).
    let replica = inserted
        .replica
        .expect("placeholder should have a Partial replica");
    assert_eq!(replica.state, ReplicaState::Partial);
}

#[test]
fn directory_has_no_blob() {
    let (_dir, cat) = open();
    seed_local(&cat);
    let inserted = cat
        .insert_file(&NewFile {
            location_id: "vol-d".into(),
            parent_id: None,
            path: "clips".into(),
            name: "clips".into(),
            kind: FileKind::Directory,
            shortcut_target_ref: None,
            size: None,
            mtime_utc: None,
            ntfs_file_id: None,
            volume_serial: None,
            mime: None,
            hydration: Hydration::LocalFull,
            remote_ref: None,
            last_scan_id: None,
            checksums: Vec::new(),
        })
        .unwrap();
    assert!(inserted.blob.is_none());
    assert!(inserted.replica.is_none());
}

#[test]
fn lookup_by_content_id_after_hash() {
    let (_dir, cat) = open();
    seed_local(&cat);
    let inserted = cat
        .insert_file(&byte_file("a.bin", "a.bin", 4, Hydration::LocalFull))
        .unwrap();
    let blob = inserted.blob.unwrap();
    let cid = ContentId::from_bytes([0x11; 32]);
    cat.set_content_id(blob.id, &cid).unwrap();
    let found = cat.blob_by_content_id(&cid).unwrap().unwrap();
    assert_eq!(found.id, blob.id);
    assert_eq!(found.content_id, Some(cid));
}

#[test]
fn batched_inserts_and_capacity_and_scan() {
    let (_dir, cat) = open();
    seed_local(&cat);
    let scan = cat.start_scan("local-d").unwrap();
    let batch = cat
        .insert_files(&[
            byte_file("a.mp4", "a.mp4", 1, Hydration::LocalFull),
            byte_file("b.mp4", "b.mp4", 2, Hydration::LocalFull),
        ])
        .unwrap();
    assert_eq!(batch.len(), 2);
    cat.finish_scan(scan, 2, ScanStatus::Ok).unwrap();

    cat.insert_capacity(&CapacitySnapshot {
        location_id: "vol-d".into(),
        total_bytes: Some(1_000_000),
        used_bytes: 100,
        free_bytes: 900,
        quota_bytes: None,
    })
    .unwrap();
    let snap = cat.latest_capacity("vol-d").unwrap().unwrap();
    assert_eq!(snap.free_bytes, 900);

    let counts = cat.counts().unwrap();
    assert_eq!(counts.files, 2);
    assert_eq!(counts.blobs, 2);
    assert_eq!(counts.replicas, 2);
    assert!(cat.integrity().unwrap().is_ok());
    assert!(cat.plans_schema_ready().unwrap());
    assert!(cat.journal_schema_ready().unwrap());
}

#[test]
fn exclusive_blocks_second_open() {
    let dir = TempDir::new().unwrap();
    let _held = Catalog::open(dir.path(), LockMode::Exclusive).unwrap();
    match Catalog::open(dir.path(), LockMode::Exclusive) {
        Err(portage_core::Error::CatalogLocked { .. }) => {}
        Err(e) => panic!("expected CatalogLocked, got {e}"),
        Ok(_) => panic!("expected CatalogLocked, got Ok"),
    }
}
