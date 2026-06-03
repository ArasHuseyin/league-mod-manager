use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, io::Read, path::Path};
use thiserror::Error;
use uuid::Uuid;

pub type ModId = Uuid;
pub type ModVersion = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModManifest {
    pub schema_version: u16,
    pub id: ModId,
    pub name: String,
    pub version: ModVersion,
    pub author: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub preview_image: Option<String>,
    #[serde(default)]
    pub assets: Vec<ModAsset>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModAsset {
    pub source: String,
    pub target: String,
    pub wad: String,
    #[serde(default)]
    pub layer: Option<String>,
    #[serde(default)]
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ModPackageKind {
    Directory,
    ModPkgArchive,
    FantomeArchive,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ValidationReport {
    pub ok: bool,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("manifest path does not exist: {0}")]
    MissingPath(String),
    #[error("archive does not contain a supported manifest file")]
    MissingArchiveManifest,
    #[error("failed to read archive: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("manifest.json was not found in package directory")]
    MissingManifest,
    #[error("failed to read manifest: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse manifest JSON: {0}")]
    Json(#[from] serde_json::Error),
}

impl ModManifest {
    pub fn validate(&self) -> ValidationReport {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        if self.schema_version != 1 {
            warnings.push(format!(
                "schemaVersion {} is not explicitly supported; expected 1",
                self.schema_version
            ));
        }

        if self.name.trim().is_empty() {
            errors.push("name is required".to_string());
        }

        if self.version.trim().is_empty() {
            errors.push("version is required".to_string());
        }

        if self.author.trim().is_empty() {
            warnings.push("author is empty".to_string());
        }

        if self.assets.is_empty() {
            errors.push("at least one asset is required".to_string());
        }

        for asset in &self.assets {
            if asset.source.trim().is_empty() {
                errors.push("asset.source is required".to_string());
            }
            if asset.target.trim().is_empty() {
                errors.push(format!("asset {} is missing target", asset.source));
            }
            if asset.wad.trim().is_empty() {
                errors.push(format!("asset {} is missing wad", asset.source));
            }
        }

        ValidationReport {
            ok: errors.is_empty(),
            warnings,
            errors,
        }
    }
}

pub fn detect_package_kind(path: &Path) -> Result<ModPackageKind, ManifestError> {
    if !path.exists() {
        return Err(ManifestError::MissingPath(path.display().to_string()));
    }

    if path.is_dir() {
        return Ok(ModPackageKind::Directory);
    }

    match path.extension().and_then(|extension| extension.to_str()) {
        Some("modpkg") => Ok(ModPackageKind::ModPkgArchive),
        Some("fantome") => Ok(ModPackageKind::FantomeArchive),
        _ => Ok(ModPackageKind::Directory),
    }
}

pub fn load_manifest(path: impl AsRef<Path>) -> Result<ModManifest, ManifestError> {
    let path = path.as_ref();
    let kind = detect_package_kind(path)?;
    match kind {
        ModPackageKind::Directory => {
            let manifest_path = if path.is_dir() {
                path.join("manifest.json")
            } else {
                path.to_path_buf()
            };
            if !manifest_path.exists() {
                return Err(ManifestError::MissingManifest);
            }
            let manifest = fs::read_to_string(manifest_path)?;
            Ok(serde_json::from_str(&manifest)?)
        }
        ModPackageKind::ModPkgArchive | ModPackageKind::FantomeArchive => load_archive_manifest(path),
    }
}

fn load_archive_manifest(path: &Path) -> Result<ModManifest, ManifestError> {
    let file = fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    let manifest_names = [
        "manifest.json",
        "modpkg.json",
        "META/manifest.json",
        "META/info.json",
        "meta/manifest.json",
        "meta/info.json",
    ];

    for name in manifest_names {
        if let Ok(mut entry) = archive.by_name(name) {
            let mut contents = String::new();
            entry.read_to_string(&mut contents)?;
            return Ok(serde_json::from_str(&contents)?);
        }
    }

    Err(ManifestError::MissingArchiveManifest)
}

pub fn hash_file(path: impl AsRef<Path>) -> Result<String, std::io::Error> {
    let bytes = fs::read(path)?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_manifest() -> ModManifest {
        ModManifest {
            schema_version: 1,
            id: Uuid::new_v4(),
            name: "Test Skin".to_string(),
            version: "1.0.0".to_string(),
            author: "tester".to_string(),
            description: String::new(),
            tags: vec!["champion".to_string()],
            preview_image: None,
            assets: vec![ModAsset {
                source: "assets/skin.bin".to_string(),
                target: "data/characters/aatrox/skins/skin01.bin".to_string(),
                wad: "Characters/Aatrox.wad.client".to_string(),
                layer: None,
                sha256: None,
            }],
        }
    }

    #[test]
    fn validates_manifest_with_assets() {
        let report = valid_manifest().validate();
        assert!(report.ok);
        assert!(report.errors.is_empty());
    }

    #[test]
    fn rejects_manifest_without_assets() {
        let mut manifest = valid_manifest();
        manifest.assets.clear();
        let report = manifest.validate();
        assert!(!report.ok);
        assert_eq!(report.errors, vec!["at least one asset is required"]);
    }

    #[test]
    fn loads_manifest_from_modpkg_archive() {
        let dir = tempfile::tempdir().expect("tempdir");
        let archive_path = dir.path().join("sample.modpkg");
        let file = fs::File::create(&archive_path).expect("archive file");
        let mut archive = zip::ZipWriter::new(file);
        let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
        let manifest = serde_json::to_string(&valid_manifest()).expect("manifest json");

        archive
            .start_file("manifest.json", options)
            .expect("start manifest");
        use std::io::Write;
        archive.write_all(manifest.as_bytes()).expect("write manifest");
        archive.finish().expect("finish archive");

        let loaded = load_manifest(&archive_path).expect("load archive manifest");
        assert_eq!(loaded.name, "Test Skin");
    }
}
