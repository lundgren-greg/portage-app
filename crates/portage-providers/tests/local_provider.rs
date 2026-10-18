//! Local provider over a synthetic temp tree: add, overlay refusal and skip,
//! list, stat, open_read, capacity.

use std::io::Read;
use std::path::Path;

use portage_catalog::{Catalog, Hydration, LockMode};
use portage_core::Error;
use portage_providers::local::pathcmp::{path_str, strip_verbatim};
use portage_providers::local::sync_roots::fakes::{FakeEnv, FakeRegistry};
use portage_providers::registry::{add_local, load_local, load_local_all};
use portage_providers::{HostOverlays, ListItem, Provider, Role};
use tempfile::TempDir;

fn tree() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    std::fs::create_dir_all(root.join("clips/2026")).unwrap();
    std::fs::create_dir_all(root.join("FakeOneDrive/inner")).unwrap();
    std::fs::write(root.join("clips/2026/a.mp4"), b"aaaa").unwrap();
    std::fs::write(root.join("clips/b.mkv"), b"bb").unwrap();
    std::fs::write(root.join("top.txt"), b"t").unwrap();
    std::fs::write(root.join("FakeOneDrive/inner/secret.mp4"), b"never").unwrap();
    tmp
}

/// Host whose OneDrive UserFolder is `<tmp>/FakeOneDrive`.
fn host_with_overlay(root: &Path) -> HostOverlays {
    let canonical = strip_verbatim(root.canonicalize().unwrap());
    let overlay = path_str(&canonical.join("FakeOneDrive"));
    let reg = FakeRegistry::default().with_value(
        r"Software\Microsoft\OneDrive\Accounts\Personal",
        "UserFolder",
        &overlay,
    );
    // On Linux the fake path is `/tmp/...`, which is not a Windows absolute
    // path; inject it directly so the skip logic is exercised on every OS.
    let mut host = HostOverlays::from_readers(&reg, &FakeEnv::default(), cfg!(windows));
    if host.roots.is_empty() {
        host.roots
            .push(portage_providers::local::sync_roots::OverlayRoot {
                path: canonical.join("FakeOneDrive"),
                provider_id: "overlay:onedrive".into(),
                detector: portage_providers::local::sync_roots::Detector::OnedriveUserfolder,
            });
    }
    host
}

fn catalog() -> (TempDir, Catalog) {
    let tmp = TempDir::new().unwrap();
    let cat = Catalog::open(tmp.path(), LockMode::Exclusive).unwrap();
    (tmp, cat)
}

#[test]
fn add_list_stat_open_and_capacity() {
    let files = tree();
    let host = host_with_overlay(files.path());
    let (_db, cat) = catalog();

    let p = add_local(&cat, "local-tmp", files.path(), Some(Role::Both), &host).unwrap();
    assert_eq!(p.config().roles, Role::Both);
    assert!(!p.config().volume_serial.is_empty());
    assert_eq!(cat.overlay_roots().unwrap().len(), host.roots.len());
    assert_eq!(p.inner_overlays().len(), 1);

    // Reload from the catalog: same config.
    let loaded = load_local(&cat, "local-tmp", &host).unwrap();
    assert_eq!(loaded.config(), p.config());
    assert_eq!(load_local_all(&cat, &host).unwrap().len(), 1);

    let mut entries = Vec::new();
    let mut skipped = Vec::new();
    for item in loaded.list().unwrap() {
        match item {
            ListItem::Entry(m) => entries.push(m),
            ListItem::SkippedOverlay(path) => skipped.push(path),
            other => panic!("unexpected {other:?}"),
        }
    }
    let paths: Vec<_> = entries.iter().map(|m| m.path.as_str()).collect();
    assert_eq!(
        paths,
        vec![
            "clips",
            "clips/2026",
            "clips/2026/a.mp4",
            "clips/b.mkv",
            "top.txt"
        ]
    );
    assert_eq!(
        skipped.len(),
        1,
        "inner overlay reported once, not descended"
    );
    assert!(entries.iter().all(|m| m.hydration == Hydration::LocalFull));
    let a = entries
        .iter()
        .find(|m| m.path == "clips/2026/a.mp4")
        .unwrap();
    assert_eq!(
        (a.size, a.is_dir, a.name.as_str()),
        (Some(4), false, "a.mp4")
    );
    assert!(a.file_id.is_some());
    assert!(entries.iter().find(|m| m.path == "clips").unwrap().is_dir);

    let st = loaded.stat("clips/b.mkv").unwrap();
    assert_eq!(st.size, Some(2));
    let mut buf = String::new();
    loaded
        .open_read("clips/b.mkv")
        .unwrap()
        .read_to_string(&mut buf)
        .unwrap();
    assert_eq!(buf, "bb");
    assert!(matches!(
        loaded.open_read("../escape"),
        Err(Error::PathEscape { .. })
    ));
    assert!(loaded.open_read("clips").is_err());

    let cap = loaded.capacity().unwrap();
    assert!(cap.total_bytes.unwrap() >= cap.free_bytes);
}

#[test]
fn add_refuses_overlay_root_and_inner_path() {
    let files = tree();
    let host = host_with_overlay(files.path());
    let (_db, cat) = catalog();
    for root in [
        files.path().join("FakeOneDrive"),
        files.path().join("FakeOneDrive").join("inner"),
    ] {
        match add_local(&cat, "od", &root, None, &host) {
            Err(Error::OverlayRoot { .. }) => {}
            other => panic!("{}: expected OverlayRoot, got {other:?}", root.display()),
        }
    }
    assert!(cat.provider("od").unwrap().is_none());
    // Overlays were still registered for doctor.
    assert!(!cat.overlay_roots().unwrap().is_empty());
}

#[test]
fn add_refuses_overlapping_roots_and_bad_ids() {
    let files = tree();
    let host = HostOverlays::default();
    let (_db, cat) = catalog();
    add_local(&cat, "outer", files.path(), None, &host).unwrap();
    assert!(matches!(
        add_local(&cat, "inner", &files.path().join("clips"), None, &host),
        Err(Error::ProviderConfig(_))
    ));
    // Re-adding the same id is an update, not an overlap.
    add_local(&cat, "outer", files.path(), Some(Role::Shuttle), &host).unwrap();
    assert_eq!(
        load_local(&cat, "outer", &host).unwrap().config().roles,
        Role::Shuttle
    );
    assert!(add_local(&cat, "Bad Id", files.path(), None, &host).is_err());
    assert!(add_local(&cat, "file", &files.path().join("top.txt"), None, &host).is_err());
    assert!(add_local(&cat, "gone", &files.path().join("missing"), None, &host).is_err());
}

#[test]
fn unplugged_volume_fails_closed() {
    let files = tree();
    let host = HostOverlays::default();
    let (_db, cat) = catalog();
    let mut cfg = add_local(&cat, "ext", files.path(), None, &host)
        .unwrap()
        .config()
        .clone();
    cfg.volume_serial = "DEAD-BEEF".into();
    cfg.root = files.path().join("not-mounted");
    let p = portage_providers::LocalProvider::new(
        portage_core::ids::ProviderId::new("ext"),
        cfg,
        Vec::new(),
    );
    assert!(matches!(p.capacity(), Err(Error::VolumeOffline { .. })));
    assert!(matches!(p.list().err(), Some(Error::VolumeOffline { .. })));
}

#[test]
fn open_read_refuses_overlays_links_and_other_disks() {
    let files = tree();
    let host = host_with_overlay(files.path());
    let (_db, cat) = catalog();
    let p = add_local(&cat, "local-tmp", files.path(), None, &host).unwrap();

    // The walk skips the overlay; a direct open must not bypass that.
    assert!(matches!(
        p.open_read("FakeOneDrive/inner/secret.mp4"),
        Err(Error::OverlayRoot { .. })
    ));

    // A link's attributes are not its target's; links are never opened.
    let link = files.path().join("clips/link.mp4");
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(files.path().join("top.txt"), &link).is_ok();
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_file(files.path().join("top.txt"), &link).is_ok();
    if made {
        assert!(matches!(
            p.open_read("clips/link.mp4"),
            Err(Error::OverlayRoot { .. })
        ));
    }

    // Same path, different disk serial: fail closed, never read it (K24).
    let mut cfg = p.config().clone();
    cfg.volume_serial = "DEAD-BEEF".into();
    let other = portage_providers::LocalProvider::new(
        portage_core::ids::ProviderId::new("local-tmp"),
        cfg,
        Vec::new(),
    );
    assert!(matches!(
        other.open_read("top.txt").err(),
        Some(Error::VolumeOffline { .. })
    ));
    assert!(matches!(
        other.stat("top.txt"),
        Err(Error::VolumeOffline { .. })
    ));
}
