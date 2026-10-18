//! Integration tests for `portage doctor` (PR 3).

use std::process::Command;

use tempfile::TempDir;

fn portage() -> Command {
    Command::new(env!("CARGO_BIN_EXE_portage"))
}

#[test]
fn help_lists_doctor() {
    let out = portage().arg("--help").output().expect("run portage");
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("doctor"), "{text}");
}

#[test]
fn doctor_creates_catalog_and_reports_ok() {
    let tmp = TempDir::new().expect("temp dir");
    let data_dir = tmp.path().join("PortageData");
    std::fs::create_dir_all(&data_dir).unwrap();

    let out = portage()
        .args(["doctor", "--data-dir"])
        .arg(&data_dir)
        .output()
        .expect("run portage doctor");
    // On a host with OneDrive / DriveFS installed, a fresh catalog has no
    // overlay_roots yet, so the K6 audit must fail — but only that audit.
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success() || stderr.contains("overlay_roots"),
        "stderr={stderr}"
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("catalog ok"), "{text}");
    assert!(data_dir.join("catalog.sqlite").exists());
    assert!(data_dir.join("portage.lock").exists());
}

#[test]
fn doctor_passes_after_overlays_registered() {
    let tmp = TempDir::new().expect("temp dir");
    let data_dir = tmp.path().join("PortageData");
    let tree = tmp.path().join("tree");
    std::fs::create_dir_all(&tree).unwrap();
    std::fs::write(tree.join("a.txt"), b"a").unwrap();

    let out = portage()
        .args(["provider", "add", "local", "--id", "t", "--root"])
        .arg(&tree)
        .arg("--data-dir")
        .arg(&data_dir)
        .output()
        .expect("run portage provider add local");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let out = portage()
        .args(["doctor", "--data-dir"])
        .arg(&data_dir)
        .output()
        .expect("run portage doctor");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("overlays :"), "{text}");
    assert!(text.contains("doctor   : ok"), "{text}");
}

#[test]
fn doctor_backup_writes_dated_copy() {
    let tmp = TempDir::new().expect("temp dir");
    let data_dir = tmp.path().join("PortageData");
    std::fs::create_dir_all(&data_dir).unwrap();

    let out = portage()
        .args(["doctor", "--backup", "--data-dir"])
        .arg(&data_dir)
        .output()
        .expect("run portage doctor --backup");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success() || stderr.contains("overlay_roots"),
        "stderr={stderr}"
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("backup"), "{text}");
    let backups: Vec<_> = std::fs::read_dir(&data_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("catalog-") && n.ends_with(".sqlite"))
        .collect();
    assert_eq!(
        backups.len(),
        1,
        "expected one dated backup, got {backups:?}"
    );
}
