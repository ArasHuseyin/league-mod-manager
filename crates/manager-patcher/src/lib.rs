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
use wad::{Compression, WadBuilder};

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

    /// Build overlay WAD archives for a profile's active mods and write them to
    /// `staging_dir`. This never touches the League installation: each target
    /// WAD becomes a separate overlay file containing only the modded assets.
    /// Plans with conflicts are refused and nothing is written.
    pub fn stage(request: &PatchRequest, staging_dir: &Path) -> Result<StageReport, PatchError> {
        let plan = build_patch_plan(&request.profile, &request.library);

        if !plan.conflicts.is_empty() {
            return Ok(StageReport {
                status: PatchStatus::Blocked,
                staged: Vec::new(),
                plan,
                messages: vec!["Patch plan has conflicts; no overlays were written.".to_string()],
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

        let mut staged = Vec::new();
        for (wad, operations) in groups {
            let mut builder = WadBuilder::new();
            for operation in &operations {
                let item = by_id.get(&operation.mod_id).ok_or_else(|| {
                    PatchError::Stage(format!("mod {} is missing from the library", operation.mod_id))
                })?;
                let bytes = read_package_asset(&item.package_path, &operation.source)
                    .map_err(|error| PatchError::Stage(format!("{}: {error}", operation.source)))?;
                builder.add(&operation.target, bytes, Compression::Gzip);
            }

            let output_path = staging_dir.join(overlay_file_name(&wad));
            let mut file = fs::File::create(&output_path)?;
            builder.write(&mut file)?;
            staged.push(StagedWad {
                wad,
                output_path,
                entry_count: operations.len(),
            });
        }

        let messages = vec![format!(
            "Wrote {} overlay WAD(s) to staging. The live League installation was not modified.",
            staged.len()
        )];

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

    #[test]
    fn stage_writes_an_overlay_wad_per_target() {
        let dir = tempfile::tempdir().unwrap();
        let package = dir.path().join("pkg");
        std::fs::create_dir_all(package.join("assets")).unwrap();
        std::fs::write(package.join("assets/skin.bin"), b"skin-bytes").unwrap();

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
                    target: "data/characters/aatrox/skin01.bin".to_string(),
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
        assert_eq!(staged.entry_count, 1);
        assert!(staged.output_path.exists());
        assert_eq!(staged.output_path.file_name().unwrap(), "Characters_Aatrox.wad.client");

        let bytes = std::fs::read(&staged.output_path).unwrap();
        let entries = wad::read_wad(&bytes).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].path_hash,
            wad::path_hash("data/characters/aatrox/skin01.bin")
        );
        assert_eq!(entries[0].data, b"skin-bytes");
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
