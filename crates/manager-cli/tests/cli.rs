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

/// Build a minimal real WAD v3 archive of Raw `(in-wad path, bytes)` entries so
/// staging has a genuine installation archive to patch.
fn build_real_wad(entries: &[(&str, &[u8])]) -> Vec<u8> {
    use manager_patcher::league_wad::{path_hash, ENTRY_LEN, HEADER_LEN, MAGIC};
    let count = entries.len();
    let toc_end = HEADER_LEN + count * ENTRY_LEN;
    let mut buf = vec![0u8; toc_end];
    buf[0..2].copy_from_slice(&MAGIC);
    buf[2] = 3;
    buf[3] = 4;
    buf[4 + 256 + 8..4 + 256 + 12].copy_from_slice(&(count as u32).to_le_bytes());
    let mut cursor = toc_end as u64;
    for (i, (path, data)) in entries.iter().enumerate() {
        let base = HEADER_LEN + i * ENTRY_LEN;
        buf[base..base + 8].copy_from_slice(&path_hash(path).to_le_bytes());
        buf[base + 8..base + 12].copy_from_slice(&(cursor as u32).to_le_bytes());
        buf[base + 12..base + 16].copy_from_slice(&(data.len() as u32).to_le_bytes());
        buf[base + 16..base + 20].copy_from_slice(&(data.len() as u32).to_le_bytes());
        buf[base + 20] = 0;
        buf.extend_from_slice(data);
        cursor += data.len() as u64;
    }
    buf
}

#[test]
fn patch_with_out_stages_patched_wads() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = package_dir(tmp.path(), "valid", VALID_MANIFEST);
    // The manifest references a.bin -> data/a.bin inside DATA/Menu.wad.client.
    fs::write(dir.join("a.bin"), b"asset-bytes").expect("write asset");

    // A real installation archive that already contains the target entry.
    let league_root = tmp.path().join("league");
    let wad_dir = league_root.join("Game/DATA/FINAL");
    fs::create_dir_all(&wad_dir).expect("create wad dir");
    let real_wad = build_real_wad(&[("data/a.bin", b"original")]);
    fs::write(wad_dir.join("Menu.wad.client"), &real_wad).expect("write real wad");

    let out = tmp.path().join("staging");

    let output = cli()
        .arg("patch")
        .arg("--league-root")
        .arg(&league_root)
        .arg("--manifest")
        .arg(dir.join("manifest.json"))
        .arg("--out")
        .arg(&out)
        .output()
        .expect("run cli");

    assert!(output.status.success(), "stage should succeed");
    assert!(stdout(&output).contains("\"status\""));
    assert!(out.join("DATA_Menu.wad.client").exists());
    assert!(out.join("redirections.json").exists());
}

#[test]
fn doctor_runs_and_emits_json() {
    let output = cli().arg("doctor").output().expect("run cli");

    assert!(output.status.success(), "doctor should succeed");
    // Output is a JSON array of installations (possibly empty on CI machines).
    assert!(stdout(&output).trim_start().starts_with('['));
}
