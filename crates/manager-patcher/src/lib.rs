use manager_core::{build_patch_plan, LibraryItem, PatchPlan, Profile};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;
use time::OffsetDateTime;

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

#[derive(Debug, Error)]
pub enum PatchError {
    #[error("League root does not exist: {0}")]
    MissingLeagueRoot(String),
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
}
