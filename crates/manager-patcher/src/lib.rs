pub mod league_wad;
pub mod wad;

use manager_core::{
    build_patch_plan, read_package_asset, LibraryItem, ModId, PatchOperation, PatchPlan, Profile,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;
use time::OffsetDateTime;

/// Sentinel `wad` value for raw, non-archived files dropped directly into the
/// game folder (Fantome `RAW/` entries). These are redirected by their target
/// path rather than patched into a WAD.
const RAW_WAD: &str = "RAW";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PatchRequest {
    pub league_root: PathBuf,
    pub dry_run: bool,
    pub profile: Profile,
    pub library: Vec<LibraryItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PatchReport {
    pub dry_run: bool,
    pub started_at: OffsetDateTime,
    pub finished_at: OffsetDateTime,
    pub plan: PatchPlan,
    pub status: PatchStatus,
    pub messages: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PatchStatus {
    Ready,
    Blocked,
    Applied,
}

/// A single overlay WAD written to the staging directory.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StagedWad {
    pub wad: String,
    pub output_path: PathBuf,
    pub entry_count: usize,
}

/// Result of staging a profile's active mods into overlay WADs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StageReport {
    pub status: PatchStatus,
    pub plan: PatchPlan,
    pub staged: Vec<StagedWad>,
    pub messages: Vec<String>,
}

#[derive(Debug, Error)]
pub enum PatchError {
    #[error("League root does not exist: {0}")]
    MissingLeagueRoot(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("wad error: {0}")]
    Wad(#[from] wad::WadError),
    #[error("failed to stage overlay: {0}")]
    Stage(String),
}

pub struct PatchEngine;

impl PatchEngine {
    pub fn plan(request: PatchRequest) -> Result<PatchReport, PatchError> {
        if !request.league_root.exists() {
            return Err(PatchError::MissingLeagueRoot(
                request.league_root.display().to_string(),
            ));
        }

        let started_at = OffsetDateTime::now_utc();
        let plan = build_patch_plan(&request.profile, &request.library);
        let blocked = !plan.conflicts.is_empty();
        let status = if blocked {
            PatchStatus::Blocked
        } else if request.dry_run {
            PatchStatus::Ready
        } else {
            PatchStatus::Applied
        };

        let mut messages = Vec::new();
        if blocked {
            messages.push("Patch plan has conflicts and cannot be applied automatically.".to_string());
        } else if request.dry_run {
            messages.push("Dry-run completed; no files were modified.".to_string());
        } else {
            messages.push(
                "Patch engine accepted the plan. Binary WAD writing is not implemented yet."
                    .to_string(),
            );
        }

        Ok(PatchReport {
            dry_run: request.dry_run,
            started_at,
            finished_at: OffsetDateTime::now_utc(),
            plan,
            status,
            messages,
        })
    }

    /// Stage a profile's active mods into game-loadable files plus a
    /// `redirections.json` the hook DLL consumes at runtime.
    ///
    /// For every target WAD this locates the *real* archive inside the League
    /// installation, applies the mod's overrides and additions with
    /// [`league_wad::patch_or_add_wad`], and writes the full patched archive to
    /// `staging_dir`. Because the original archive is kept whole, the game still
    /// finds every untouched asset — only the modded entries change. Raw
    /// (`RAW/`) files are copied verbatim and redirected by their target path.
    ///
    /// The live installation is never modified: the hook redirects the game's
    /// file opens to these staged copies via `redirections.json`. Plans with
    /// conflicts are refused and nothing is written.
    pub fn stage(request: &PatchRequest, staging_dir: &Path) -> Result<StageReport, PatchError> {
        if !request.league_root.exists() {
            return Err(PatchError::MissingLeagueRoot(
                request.league_root.display().to_string(),
            ));
        }

        let plan = build_patch_plan(&request.profile, &request.library);

        if !plan.conflicts.is_empty() {
            return Ok(StageReport {
                status: PatchStatus::Blocked,
                staged: Vec::new(),
                plan,
                messages: vec!["Patch plan has conflicts; nothing was written.".to_string()],
            });
        }

        let by_id: HashMap<ModId, &LibraryItem> = request
            .library
            .iter()
            .map(|item| (item.manifest.id, item))
            .collect();

        // Group operations by their destination WAD; BTreeMap keeps the staged
        // output deterministic regardless of profile ordering.
        let mut groups: BTreeMap<String, Vec<&PatchOperation>> = BTreeMap::new();
        for operation in &plan.operations {
            groups.entry(operation.wad.clone()).or_default().push(operation);
        }

        fs::create_dir_all(staging_dir)?;

        let read_asset = |operation: &PatchOperation| -> Result<Vec<u8>, PatchError> {
            let item = by_id.get(&operation.mod_id).ok_or_else(|| {
                PatchError::Stage(format!("mod {} is missing from the library", operation.mod_id))
            })?;
            read_package_asset(&item.package_path, &operation.source)
                .map_err(|error| PatchError::Stage(format!("{}: {error}", operation.source)))
        };

        let mut staged = Vec::new();
        let mut messages = Vec::new();
        // Normalized suffix of the game's file open -> absolute staged replacement.
        let mut redirections: BTreeMap<String, String> = BTreeMap::new();

        for (wad, operations) in groups {
            if wad == RAW_WAD {
                for operation in &operations {
                    let bytes = read_asset(operation)?;
                    let output_path = staging_dir.join(overlay_file_name(&operation.target));
                    fs::write(&output_path, &bytes)?;
                    redirections.insert(
                        normalize_key(&operation.target),
                        absolute_string(&output_path),
                    );
                    staged.push(StagedWad {
                        wad: RAW_WAD.to_string(),
                        output_path,
                        entry_count: 1,
                    });
                }
                continue;
            }

            // Locate the real archive in the installation so the patch keeps every
            // untouched asset. Without it we cannot produce a loadable WAD, so we
            // skip this target and report it rather than ship a broken overlay.
            let Some(real_wad) = find_game_wad(&request.league_root, &wad) else {
                messages.push(format!(
                    "could not find '{wad}' under {}; skipped",
                    request.league_root.display()
                ));
                continue;
            };

            let original = fs::read(&real_wad)?;
            let payloads: Vec<(u64, Vec<u8>)> = operations
                .iter()
                .map(|operation| Ok((league_wad::path_hash(&operation.target), read_asset(operation)?)))
                .collect::<Result<_, PatchError>>()?;
            let overrides: Vec<(u64, &[u8])> = payloads
                .iter()
                .map(|(hash, bytes)| (*hash, bytes.as_slice()))
                .collect();

            let patched = league_wad::patch_or_add_wad(&original, &overrides)
                .map_err(|error| PatchError::Stage(format!("{wad}: {error}")))?;

            let output_path = staging_dir.join(overlay_file_name(&wad));
            fs::write(&output_path, &patched)?;

            redirections.insert(redirect_key(&request.league_root, &real_wad), absolute_string(&output_path));

            staged.push(StagedWad {
                wad,
                output_path,
                entry_count: operations.len(),
            });
        }

        // Write the config the hook DLL reads to know what to redirect at runtime.
        let redirections_path = staging_dir.join("redirections.json");
        fs::write(
            &redirections_path,
            serde_json::to_vec_pretty(&redirections)
                .map_err(|error| PatchError::Stage(error.to_string()))?,
        )?;

        messages.push(format!(
            "Staged {} file(s) and {} redirection(s); the live League installation was not modified.",
            staged.len(),
            redirections.len()
        ));

        Ok(StageReport {
            status: PatchStatus::Applied,
            staged,
            plan,
            messages,
        })
    }
}

/// Map a WAD path to a flat, collision-free staging file name.
fn overlay_file_name(wad: &str) -> String {
    wad.replace(['/', '\\'], "_")
}

/// Normalize a path the same way the hook DLL does for matching: forward
/// slashes, lowercase.
fn normalize_path(path: &Path) -> String {
    normalize_key(&path.to_string_lossy())
}

fn normalize_key(value: &str) -> String {
    value.replace('\\', "/").to_ascii_lowercase()
}

fn absolute_string(path: &Path) -> String {
    fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .to_string()
}

/// Build the hook's redirection key for a real archive: its path relative to the
/// game's `Game` directory, normalized.
///
/// The game runs with `Game` as its working directory and opens WADs with paths
/// rooted there — sometimes absolute (`…/Game/DATA/FINAL/…`), sometimes relative
/// to it (`DATA/FINAL/…`). A `Game`-relative key is a path-boundary suffix of
/// *both* forms; an install-root key would carry a leading `game/` segment that
/// the relative form lacks, so the hook's suffix match would miss it entirely.
fn redirect_key(league_root: &Path, real_wad: &Path) -> String {
    let game_dir = league_root.join("Game");
    real_wad
        .strip_prefix(&game_dir)
        .or_else(|_| real_wad.strip_prefix(league_root))
        .map(normalize_path)
        .unwrap_or_else(|_| {
            real_wad
                .file_name()
                .map(|name| normalize_key(&name.to_string_lossy()))
                .unwrap_or_default()
        })
}

/// Find a real `.wad.client` archive in a League installation by file name.
///
/// The manifest's `wad` field carries a logical path (e.g.
/// `Characters/Aatrox.wad.client`) whose folder need not match the on-disk
/// layout (`Game/DATA/FINAL/Champions/...`), so we match on the final path
/// component — WAD file names are unique within an install. When several copies
/// share a name, we pick deterministically and prefer the canonical `DATA/FINAL`
/// asset tree over any stray copy, so the choice never depends on `read_dir`
/// ordering.
fn find_game_wad(league_root: &Path, wad: &str) -> Option<PathBuf> {
    let file_name = wad
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(wad)
        .to_ascii_lowercase();

    // Real archives live under the `Game` subtree; fall back to the whole root.
    let game_dir = league_root.join("Game");
    let search_root = if game_dir.is_dir() { game_dir } else { league_root.to_path_buf() };

    let mut matches = Vec::new();
    collect_files_named(&search_root, &file_name, &mut matches);

    matches.sort_by_key(|path| {
        let normalized = normalize_path(path);
        // Prefer the canonical asset tree, then the least-nested path, then a
        // lexical tiebreak so the result is fully deterministic.
        let outside_final = u8::from(!normalized.contains("/data/final/"));
        (outside_final, path.components().count(), normalized)
    });
    matches.into_iter().next()
}

/// Recursively collect every file under `dir` whose name equals `target`
/// (case-insensitive).
fn collect_files_named(dir: &Path, target: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            subdirs.push(path);
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.eq_ignore_ascii_case(target))
            .unwrap_or(false)
        {
            out.push(path);
        }
    }
    for subdir in subdirs {
        collect_files_named(&subdir, target, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use manager_core::{ModAsset, ModManifest};
    use uuid::Uuid;

    fn library_item(id: Uuid, target: &str) -> LibraryItem {
        LibraryItem {
            manifest: ModManifest {
                schema_version: 1,
                id,
                name: format!("Mod {id}"),
                version: "1.0.0".to_string(),
                author: "tester".to_string(),
                description: String::new(),
                tags: Vec::new(),
                preview_image: None,
                assets: vec![ModAsset {
                    source: "asset.bin".to_string(),
                    target: target.to_string(),
                    wad: "Characters/Aatrox.wad.client".to_string(),
                    layer: None,
                    sha256: None,
                }],
            },
            package_path: PathBuf::from("."),
            imported_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    fn profile_with(mods: &[Uuid]) -> Profile {
        let mut profile = Profile::new("Test");
        for mod_id in mods {
            profile.enable_mod(*mod_id);
        }
        profile
    }

    #[test]
    fn missing_league_root_is_an_error() {
        let request = PatchRequest {
            league_root: PathBuf::from("this/path/should/not/exist/zzz"),
            dry_run: true,
            profile: Profile::new("Test"),
            library: Vec::new(),
        };

        let error = PatchEngine::plan(request).expect_err("missing root should fail");
        assert!(matches!(error, PatchError::MissingLeagueRoot(_)));
    }

    #[test]
    fn dry_run_without_conflicts_is_ready() {
        let dir = tempfile::tempdir().unwrap();
        let id = Uuid::new_v4();
        let report = PatchEngine::plan(PatchRequest {
            league_root: dir.path().to_path_buf(),
            dry_run: true,
            profile: profile_with(&[id]),
            library: vec![library_item(id, "skin01.bin")],
        })
        .unwrap();

        assert_eq!(report.status, PatchStatus::Ready);
        assert!(report.dry_run);
        assert!(report.plan.conflicts.is_empty());
        assert_eq!(report.plan.operations.len(), 1);
    }

    #[test]
    fn conflicting_targets_block_the_patch() {
        let dir = tempfile::tempdir().unwrap();
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let report = PatchEngine::plan(PatchRequest {
            league_root: dir.path().to_path_buf(),
            dry_run: true,
            profile: profile_with(&[first, second]),
            library: vec![
                library_item(first, "same.bin"),
                library_item(second, "same.bin"),
            ],
        })
        .unwrap();

        assert_eq!(report.status, PatchStatus::Blocked);
        assert_eq!(report.plan.conflicts.len(), 1);
    }

    #[test]
    fn apply_without_conflicts_reports_unimplemented_writing() {
        let dir = tempfile::tempdir().unwrap();
        let id = Uuid::new_v4();
        let report = PatchEngine::plan(PatchRequest {
            league_root: dir.path().to_path_buf(),
            dry_run: false,
            profile: profile_with(&[id]),
            library: vec![library_item(id, "skin01.bin")],
        })
        .unwrap();

        assert_eq!(report.status, PatchStatus::Applied);
        assert!(report
            .messages
            .iter()
            .any(|message| message.contains("not implemented")));
    }

    /// Build a minimal real WAD v3 archive of Raw `(in-wad path, bytes)` entries,
    /// matching the on-disk layout `league_wad` parses.
    fn build_real_wad(entries: &[(&str, &[u8])]) -> Vec<u8> {
        use league_wad::{ENTRY_LEN, HEADER_LEN, MAGIC};
        let count = entries.len();
        let toc_end = HEADER_LEN + count * ENTRY_LEN;
        let mut buf = vec![0u8; toc_end];
        buf[0..2].copy_from_slice(&MAGIC);
        buf[2] = 3; // major
        buf[3] = 4; // minor
        buf[4 + 256 + 8..4 + 256 + 12].copy_from_slice(&(count as u32).to_le_bytes());

        let mut cursor = toc_end as u64;
        for (i, (path, data)) in entries.iter().enumerate() {
            let base = HEADER_LEN + i * ENTRY_LEN;
            buf[base..base + 8].copy_from_slice(&league_wad::path_hash(path).to_le_bytes());
            buf[base + 8..base + 12].copy_from_slice(&(cursor as u32).to_le_bytes());
            buf[base + 12..base + 16].copy_from_slice(&(data.len() as u32).to_le_bytes());
            buf[base + 16..base + 20].copy_from_slice(&(data.len() as u32).to_le_bytes());
            buf[base + 20] = 0; // Raw
            buf.extend_from_slice(data);
            cursor += data.len() as u64;
        }
        buf
    }

    #[test]
    fn stage_patches_the_real_wad_and_writes_redirections() {
        let dir = tempfile::tempdir().unwrap();

        // A real installation archive with one original asset we will override and
        // one untouched asset that must survive the patch.
        let target = "data/characters/aatrox/skin01.bin";
        let real_wad = build_real_wad(&[
            (target, b"original-skin"),
            ("data/characters/aatrox/base.bin", b"keep-me"),
        ]);
        let wad_dir = dir.path().join("Game/DATA/FINAL/Champions");
        std::fs::create_dir_all(&wad_dir).unwrap();
        let wad_path = wad_dir.join("Aatrox.wad.client");
        std::fs::write(&wad_path, &real_wad).unwrap();

        // A mod package that overrides the skin asset.
        let package = dir.path().join("pkg");
        std::fs::create_dir_all(package.join("assets")).unwrap();
        std::fs::write(package.join("assets/skin.bin"), b"modded-skin-bytes").unwrap();

        let id = Uuid::new_v4();
        let item = LibraryItem {
            manifest: ModManifest {
                schema_version: 1,
                id,
                name: "Skin".to_string(),
                version: "1.0.0".to_string(),
                author: "tester".to_string(),
                description: String::new(),
                tags: Vec::new(),
                preview_image: None,
                assets: vec![ModAsset {
                    source: "assets/skin.bin".to_string(),
                    target: target.to_string(),
                    wad: "Characters/Aatrox.wad.client".to_string(),
                    layer: None,
                    sha256: None,
                }],
            },
            package_path: package,
            imported_at: OffsetDateTime::UNIX_EPOCH,
        };

        let staging = dir.path().join("staging");
        let report = PatchEngine::stage(
            &PatchRequest {
                league_root: dir.path().to_path_buf(),
                dry_run: true,
                profile: profile_with(&[id]),
                library: vec![item],
            },
            &staging,
        )
        .unwrap();

        assert_eq!(report.status, PatchStatus::Applied);
        assert_eq!(report.staged.len(), 1);
        let staged = &report.staged[0];
        assert_eq!(staged.output_path.file_name().unwrap(), "Characters_Aatrox.wad.client");

        // The staged file is a real, loadable WAD v3: the override resolves to the
        // mod bytes while the untouched asset still resolves to its original bytes.
        let bytes = std::fs::read(&staged.output_path).unwrap();
        let parsed = league_wad::WadV3::parse(&bytes).unwrap();
        let overridden = parsed.find_path(target).unwrap();
        assert_eq!(
            league_wad::decode_chunk(&bytes, overridden).unwrap(),
            b"modded-skin-bytes"
        );
        let kept = parsed.find_path("data/characters/aatrox/base.bin").unwrap();
        assert_eq!(league_wad::decode_chunk(&bytes, kept).unwrap(), b"keep-me");

        // The hook config points the game's open of the real archive at the staged copy.
        let redirections: serde_json::Value =
            serde_json::from_slice(&std::fs::read(staging.join("redirections.json")).unwrap())
                .unwrap();
        let map = redirections.as_object().unwrap();
        assert_eq!(map.len(), 1);
        let (key, value) = map.iter().next().unwrap();
        // Keyed relative to `Game` (no leading `game/`) so the hook's suffix match
        // works whether the game opens an absolute or a Game-relative path.
        assert_eq!(key, "data/final/champions/aatrox.wad.client");
        assert!(value.as_str().unwrap().ends_with("Characters_Aatrox.wad.client"));
    }

    #[test]
    fn find_game_wad_prefers_data_final_and_is_deterministic() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // The same archive name exists both as a stray copy and in the canonical
        // DATA/FINAL tree; the canonical one must win regardless of walk order.
        let stray = root.join("Game/DATA/Menu.wad.client");
        let canonical = root.join("Game/DATA/FINAL/UI/Menu.wad.client");
        std::fs::create_dir_all(stray.parent().unwrap()).unwrap();
        std::fs::create_dir_all(canonical.parent().unwrap()).unwrap();
        std::fs::write(&stray, b"stray").unwrap();
        std::fs::write(&canonical, b"canonical").unwrap();

        let found = find_game_wad(root, "DATA/Menu.wad.client").unwrap();
        assert_eq!(found, canonical);
    }

    #[test]
    fn stage_redirects_raw_files_by_target_path() {
        let dir = tempfile::tempdir().unwrap();
        let package = dir.path().join("pkg");
        std::fs::create_dir_all(package.join("RAW/DATA")).unwrap();
        std::fs::write(package.join("RAW/DATA/config.bin"), b"raw-bytes").unwrap();

        let id = Uuid::new_v4();
        let item = LibraryItem {
            manifest: ModManifest {
                schema_version: 1,
                id,
                name: "Raw".to_string(),
                version: "1.0.0".to_string(),
                author: "tester".to_string(),
                description: String::new(),
                tags: Vec::new(),
                preview_image: None,
                assets: vec![ModAsset {
                    source: "RAW/DATA/config.bin".to_string(),
                    target: "DATA/config.bin".to_string(),
                    wad: "RAW".to_string(),
                    layer: Some("raw".to_string()),
                    sha256: None,
                }],
            },
            package_path: package,
            imported_at: OffsetDateTime::UNIX_EPOCH,
        };

        let staging = dir.path().join("staging");
        let report = PatchEngine::stage(
            &PatchRequest {
                league_root: dir.path().to_path_buf(),
                dry_run: true,
                profile: profile_with(&[id]),
                library: vec![item],
            },
            &staging,
        )
        .unwrap();

        assert_eq!(report.status, PatchStatus::Applied);
        assert_eq!(report.staged.len(), 1);
        assert_eq!(std::fs::read(&report.staged[0].output_path).unwrap(), b"raw-bytes");

        let redirections: serde_json::Value =
            serde_json::from_slice(&std::fs::read(staging.join("redirections.json")).unwrap())
                .unwrap();
        assert!(redirections.as_object().unwrap().contains_key("data/config.bin"));
    }

    #[test]
    fn stage_refuses_conflicting_plans() {
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join("staging");
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();

        let report = PatchEngine::stage(
            &PatchRequest {
                league_root: dir.path().to_path_buf(),
                dry_run: true,
                profile: profile_with(&[first, second]),
                library: vec![
                    library_item(first, "same.bin"),
                    library_item(second, "same.bin"),
                ],
            },
            &staging,
        )
        .unwrap();

        assert_eq!(report.status, PatchStatus::Blocked);
        assert!(report.staged.is_empty());
        assert!(!staging.exists());
    }
}
