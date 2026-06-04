pub mod league;
pub mod manifest;
pub mod patch_plan;
pub mod policy;
pub mod profile;
pub mod storage;

pub use league::{detect_league_installations, LeagueInstallation};
pub use manifest::{
    load_manifest, read_package_asset, ModAsset, ModId, ModManifest, ModPackageKind, ModVersion,
    ValidationReport,
};
pub use patch_plan::{build_patch_plan, PatchConflict, PatchOperation, PatchPlan};
pub use policy::{assess_manifest_policy, PolicyAssessment, PolicyRisk};
pub use profile::{Profile, ProfileId};
pub use storage::{load_state, save_state, import_package, AppPaths, LibraryItem, PersistedState, StorageError};

