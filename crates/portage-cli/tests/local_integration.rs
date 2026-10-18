//! Integration tests for `provider add local`, `index`, `capacity` (PR 4).
//!
//! Only synthetic temp trees are indexed. No real user folder is walked.

use std::path::Path;
use std::process::{Command, Output};

use tempfile::TempDir;

fn portage() -> Command {
    Command::new(env!("CARGO_BIN_EXE_portage"))
}

fn run(args: &[&str], data_dir: &Path) -> Output {
    portage()
        .args(args)
        .arg("--data-dir")
        .arg(data_dir)
        .output()
        .expect("run portage")
}

fn ok(out: &Output) -> String {
    assert!(
        out.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn tree() -> TempDir {
    let tmp = TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join("clips")).unwrap();
    std::fs::write(tmp.path().join("clips/a.mp4"), b"aaaa").unwrap();
    std::fs::write(tmp.path().join("clips/b.mp4"), b"bb").unwrap();
    std::fs::write(tmp.path().join("readme.txt"), b"r").unwrap();
    tmp
}

#[test]
fn help_lists_pr4_commands() {
    let out = portage().arg("--help").output().unwrap();
    let text = ok(&out);
    for cmd in ["provider", "index", "capacity"] {
        assert!(text.contains(cmd), "{cmd} missing from {text}");
    }
}

#[test]
fn add_index_capacity_doctor_on_temp_tree() {
    let files = tree();
    let data = TempDir::new().unwrap();
    let root = files.path().to_str().unwrap();

    let mut add = vec!["provider", "add", "local", "--root", root];
    add.extend(["--id", "tmp-tree", "--role", "both"]);
    let text = ok(&run(&add, data.path()));
    assert!(text.contains("tmp-tree"), "{text}");
    assert!(text.contains("roles    : both"), "{text}");
    assert!(text.contains("serial"), "{text}");

    let text = ok(&run(&["index", "--provider", "tmp-tree"], data.path()));
    assert!(text.contains("3 files, 1 dirs, 0 placeholders"), "{text}");

    // Re-index all providers: idempotent.
    let text = ok(&run(&["index"], data.path()));
    assert!(text.contains("3 files"), "{text}");

    let text = ok(&run(&["capacity"], data.path()));
    assert!(text.contains("tmp-tree"), "{text}");
    assert!(text.contains("free"), "{text}");

    let text = ok(&run(&["doctor"], data.path()));
    assert!(text.contains("files    : 4"), "{text}");
    assert!(text.contains("doctor   : ok"), "{text}");
}

#[test]
fn bad_role_and_missing_provider_fail() {
    let files = tree();
    let data = TempDir::new().unwrap();
    let root = files.path().to_str().unwrap();
    let mut add = vec!["provider", "add", "local", "--root", root];
    add.extend(["--role", "hop"]);
    assert!(!run(&add, data.path()).status.success());

    let out = run(&["index"], data.path());
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("no local providers"), "{err}");

    let out = run(&["index", "--provider", "nope"], data.path());
    assert!(!out.status.success());
}

#[test]
fn capacity_reports_offline_volume_and_still_succeeds() {
    let files = tree();
    let data = TempDir::new().unwrap();
    let root = files.path().join("clips");
    let add = ["provider", "add", "local", "--root", root.to_str().unwrap()];
    ok(&run(&add, data.path()));
    std::fs::remove_dir_all(&root).unwrap();
    let text = ok(&run(&["capacity"], data.path()));
    assert!(text.contains("OFFLINE"), "{text}");
}

/// Every command that writes into the data dir refuses one inside an
/// overlay before creating anything — including `.` from inside it.
/// `LOCALAPPDATA` is pointed at a temp dir, so its `Google\DriveFS` cache
/// overlay is synthetic.
#[cfg(windows)]
#[test]
fn data_dir_inside_overlay_is_refused_by_every_command() {
    let files = tree();
    let lad = TempDir::new().unwrap();
    let cache = lad.path().join(r"Google\DriveFS\sub");
    std::fs::create_dir_all(&cache).unwrap();
    let root = files.path().to_str().unwrap();
    let commands: [&[&str]; 5] = [
        &["init"],
        &["provider", "add", "local", "--root", root],
        &["index"],
        &["capacity"],
        &["doctor"],
    ];
    for args in commands {
        let out = portage()
            .env("LOCALAPPDATA", lad.path())
            .current_dir(&cache)
            .args(args)
            .args(["--data-dir", "."])
            .output()
            .unwrap();
        assert!(!out.status.success(), "{args:?} succeeded");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains("refusing data_dir"), "{args:?}: {err}");
        let created: Vec<_> = std::fs::read_dir(&cache).unwrap().collect();
        assert!(created.is_empty(), "{args:?} created {created:?}");
    }
}

/// The DriveFS cache is always an overlay. The refusal is lexical, so the
/// (possibly real) DriveFS directory is never touched.
#[cfg(windows)]
#[test]
fn add_refuses_drivefs_cache_root() {
    let lad = std::env::var("LOCALAPPDATA").expect("LOCALAPPDATA");
    let root = format!(r"{lad}\Google\DriveFS\portage-test-does-not-exist");
    let data = TempDir::new().unwrap();
    let add = ["provider", "add", "local", "--root", &root];
    let out = run(&add, data.path());
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("overlay"), "{err}");

    // K22: the catalog may not live in an overlay either. Refused before
    // anything is created.
    let out = portage()
        .args(["init", "--data-dir", &root])
        .output()
        .expect("run portage init");
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("refusing data_dir"), "{err}");
    assert!(!Path::new(&root).exists());
}
