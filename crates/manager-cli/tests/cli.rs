use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const VALID_MANIFEST: &str = r#"{
  "schemaVersion": 1,
  "id": "11111111-1111-1111-1111-111111111111",
  "name": "Smoke Test Mod",
  "version": "1.0.0",
  "author": "tester",
  "assets": [
    { "source": "a.bin", "target": "data/a.bin", "wad": "DATA/Menu.wad.client" }
  ]
}"#;

const INVALID_MANIFEST: &str = r#"{
  "schemaVersion": 1,
  "id": "22222222-2222-2222-2222-222222222222",
  "name": "Broken Mod",
  "version": "1.0.0",
  "author": "tester",
  "assets": []
}"#;

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_manager-cli"))
}

/// Write a `manifest.json` into a fresh package directory and return its path.
fn package_dir(root: &Path, name: &str, manifest: &str) -> PathBuf {
    let dir = root.join(name);
    fs::create_dir_all(&dir).expect("create package dir");
    fs::write(dir.join("manifest.json"), manifest).expect("write manifest");
    dir
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn validate_accepts_a_valid_package() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = package_dir(tmp.path(), "valid", VALID_MANIFEST);

    let output = cli().arg("validate").arg(&dir).output().expect("run cli");

    assert!(output.status.success(), "validate should succeed");
    assert!(stdout(&output).contains("\"ok\": true"));
}

#[test]
fn validate_rejects_a_package_without_assets() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = package_dir(tmp.path(), "invalid", INVALID_MANIFEST);

    let output = cli().arg("validate").arg(&dir).output().expect("run cli");

    assert!(!output.status.success(), "validate should fail");
}

#[test]
fn import_reports_validation_and_policy() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = package_dir(tmp.path(), "valid", VALID_MANIFEST);

    let output = cli().arg("import").arg(&dir).output().expect("run cli");

    assert!(output.status.success(), "import should succeed");
    assert!(stdout(&output).contains("\"ok\": true"));
}

#[test]
fn patch_produces_a_dry_run_report() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = package_dir(tmp.path(), "valid", VALID_MANIFEST);
    let league_root = tmp.path().join("league");
    fs::create_dir_all(&league_root).expect("create league root");

    let output = cli()
        .arg("patch")
        .arg("--league-root")
        .arg(&league_root)
        .arg("--manifest")
        .arg(dir.join("manifest.json"))
        .output()
        .expect("run cli");

    assert!(output.status.success(), "patch dry-run should succeed");
    let text = stdout(&output);
    assert!(text.contains("\"dryRun\": true"));
    assert!(text.contains("\"status\""));
}

#[test]
fn patch_with_out_stages_overlay_wads() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = package_dir(tmp.path(), "valid", VALID_MANIFEST);
    // The manifest references a.bin -> DATA/Menu.wad.client; create that asset.
    fs::write(dir.join("a.bin"), b"asset-bytes").expect("write asset");
    let out = tmp.path().join("staging");

    let output = cli()
        .arg("patch")
        .arg("--manifest")
        .arg(dir.join("manifest.json"))
        .arg("--out")
        .arg(&out)
        .output()
        .expect("run cli");

    assert!(output.status.success(), "stage should succeed");
    assert!(stdout(&output).contains("\"status\""));
    assert!(out.join("DATA_Menu.wad.client").exists());
}

#[test]
fn doctor_runs_and_emits_json() {
    let output = cli().arg("doctor").output().expect("run cli");

    assert!(output.status.success(), "doctor should succeed");
    // Output is a JSON array of installations (possibly empty on CI machines).
    assert!(stdout(&output).trim_start().starts_with('['));
}
