use crate::{manifest::ModId, profile::Profile, storage::LibraryItem};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PatchPlan {
    pub profile_id: uuid::Uuid,
    pub operations: Vec<PatchOperation>,
    pub conflicts: Vec<PatchConflict>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PatchOperation {
    pub mod_id: ModId,
    pub mod_name: String,
    pub source: String,
    pub target: String,
    pub wad: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PatchConflict {
    pub wad: String,
    pub target: String,
    pub mod_ids: Vec<ModId>,
}

pub fn build_patch_plan(profile: &Profile, library: &[LibraryItem]) -> PatchPlan {
    let enabled: HashSet<ModId> = profile.enabled_mods.iter().copied().collect();
    let ordered = profile
        .mod_order
        .iter()
        .copied()
        .filter(|mod_id| enabled.contains(mod_id))
        .collect::<Vec<_>>();

    let by_id = library
        .iter()
        .map(|item| (item.manifest.id, item))
        .collect::<HashMap<_, _>>();

    // Resolve overlaps by load order: when two enabled mods write the same asset,
    // the one later in `mod_order` wins (like cslol/LTK), and we record a warning
    // rather than blocking. Operations keep their first-seen position for a stable,
    // deterministic plan while their content is replaced by the winner.
    let mut operations: Vec<PatchOperation> = Vec::new();
    let mut slot_by_target: HashMap<(String, String), usize> = HashMap::new();
    let mut warnings = Vec::new();

    for mod_id in ordered {
        let Some(item) = by_id.get(&mod_id) else {
            warnings.push(format!("enabled mod {} is missing from library", mod_id));
            continue;
        };

        for asset in &item.manifest.assets {
            let operation = PatchOperation {
                mod_id,
                mod_name: item.manifest.name.clone(),
                source: asset.source.clone(),
                target: asset.target.clone(),
                wad: asset.wad.clone(),
            };
            let key = (asset.wad.clone(), asset.target.clone());
            match slot_by_target.get(&key) {
                Some(&index) => {
                    let previous = std::mem::replace(&mut operations[index], operation);
                    warnings.push(format!(
                        "\"{}\" overrides \"{}\" on {} in {} (later in load order wins)",
                        item.manifest.name, previous.mod_name, asset.target, asset.wad
                    ));
                }
                None => {
                    slot_by_target.insert(key, operations.len());
                    operations.push(operation);
                }
            }
        }
    }

    PatchPlan {
        profile_id: profile.id,
        operations,
        conflicts: Vec::new(),
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        manifest::{ModAsset, ModManifest},
        profile::Profile,
        storage::LibraryItem,
    };
    use std::path::PathBuf;
    use time::OffsetDateTime;
    use uuid::Uuid;

    fn item(id: Uuid, name: &str, target: &str) -> LibraryItem {
        LibraryItem {
            manifest: ModManifest {
                schema_version: 1,
                id,
                name: name.to_string(),
                version: "1".to_string(),
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

    #[test]
    fn overlapping_targets_resolve_by_load_order() {
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let mut profile = Profile::new("Default");
        profile.enable_mod(first);
        profile.enable_mod(second);

        let plan = build_patch_plan(&profile, &[
            item(first, "First", "same.bin"),
            item(second, "Second", "same.bin"),
        ]);

        // The later mod wins; the overlap is a warning, not a hard conflict.
        assert!(plan.conflicts.is_empty());
        assert_eq!(plan.operations.len(), 1);
        assert_eq!(plan.operations[0].mod_name, "Second");
        assert_eq!(plan.warnings.len(), 1);
        assert!(plan.warnings[0].contains("overrides"));
    }
}
