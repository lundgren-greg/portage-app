//! Integration tests for `portage init` (PR 1).
//!
//! Uses temp dirs and cleans up via `tempfile`. No network, no SQLite.

use std::process::Command;

use tempfile::TempDir;

fn portage() -> Command {
    Command::new(env!("CARGO_BIN_EXE_portage"))
}

#[test]
fn help_prints_usage_and_exits_zero() {
    let out = portage().arg("--help").output().expect("run portage");
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("portage"));
    assert!(text.contains("init"));
    assert!(text.contains("doctor"));
}

#[test]
fn init_creates_lock_and_config_in_data_dir() {
    let tmp = TempDir::new().expect("temp dir");
    let data_dir = tmp.path().join("PortageData");

    let out = portage()
        .args(["init", "--data-dir"])
        .arg(&data_dir)
        .output()
        .expect("run portage init");

    // The temp volume may itself be below 8 GiB free; both outcomes are
    // legitimate, but each must match its contract exactly.
    if out.status.success() {
        assert!(data_dir.join("portage.lock").exists(), "lock file missing");
        assert!(
            data_dir.join("catalog.sqlite").exists(),
            "catalog.sqlite missing after init"
        );
        let config = data_dir.join("config.yaml");
        assert!(config.exists(), "config missing");
        let yaml = std::fs::read_to_string(config).expect("read config");
        assert!(
            yaml.contains("collections"),
            "config is not the example policy"
        );
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            err.contains("refusing data_dir"),
            "unexpected failure: {err}"
        );
        assert!(!data_dir.exists(), "refused init must not create the dir");
    }
}

#[test]
fn init_is_idempotent_and_keeps_existing_config() {
    let tmp = TempDir::new().expect("temp dir");
    let data_dir = tmp.path().join("PortageData");

    let first = portage()
        .args(["init", "--data-dir"])
        .arg(&data_dir)
        .output()
        .expect("first init");
    if !first.status.success() {
        // Volume too small to test idempotence; the refusal path is covered above.
        return;
    }

    let config = data_dir.join("config.yaml");
    std::fs::write(&config, "# user-edited\n").expect("edit config");

    let second = portage()
        .args(["init", "--data-dir"])
        .arg(&data_dir)
        .output()
        .expect("second init");
    assert!(second.status.success());

    let yaml = std::fs::read_to_string(&config).expect("read config");
    assert_eq!(
        yaml, "# user-edited\n",
        "init must not overwrite user config"
    );
}

#[test]
fn unknown_subcommand_fails() {
    let out = portage().arg("frobnicate").output().expect("run portage");
    assert!(!out.status.success());
}
