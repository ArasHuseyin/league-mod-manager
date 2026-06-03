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
