use crate::manifest::ModManifest;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PolicyRisk {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PolicyAssessment {
    pub risk: PolicyRisk,
    pub warnings: Vec<String>,
}

pub fn assess_manifest_policy(manifest: &ModManifest) -> PolicyAssessment {
    let mut warnings = Vec::new();
    let mut risk = PolicyRisk::Low;
    let searchable = format!(
        "{} {} {}",
        manifest.name,
        manifest.description,
        manifest.tags.join(" ")
    )
    .to_lowercase();

    if ["paid", "prestige", "mythic", "ultimate"]
        .iter()
        .any(|needle| searchable.contains(needle))
    {
        risk = PolicyRisk::High;
        warnings.push(
            "This mod may replicate paid or premium cosmetic content. Review Riot policy before use."
                .to_string(),
        );
    }

    if ["zoom", "maphack", "hitbox", "visibility", "competitive"]
        .iter()
        .any(|needle| searchable.contains(needle))
    {
        risk = PolicyRisk::High;
        warnings.push(
            "This mod may affect competitive visibility or gameplay information.".to_string(),
        );
    }

    if warnings.is_empty() && manifest.assets.len() > 1000 {
        risk = PolicyRisk::Medium;
        warnings.push("Large asset count; patching may be slow or fragile.".to_string());
    }

    PolicyAssessment { risk, warnings }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{ModAsset, ModManifest};
    use uuid::Uuid;

    #[test]
    fn warns_about_paid_cosmetic_terms() {
        let manifest = ModManifest {
            schema_version: 1,
            id: Uuid::new_v4(),
            name: "Prestige Example".to_string(),
            version: "1.0.0".to_string(),
            author: "tester".to_string(),
            description: String::new(),
            tags: Vec::new(),
            preview_image: None,
            assets: vec![ModAsset {
                source: "a".to_string(),
                target: "b".to_string(),
                wad: "c".to_string(),
                layer: None,
                sha256: None,
            }],
        };

        let assessment = assess_manifest_policy(&manifest);
        assert_eq!(assessment.risk, PolicyRisk::High);
        assert!(!assessment.warnings.is_empty());
    }
}
