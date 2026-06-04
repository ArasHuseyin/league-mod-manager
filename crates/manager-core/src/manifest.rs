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
        ModPackageKind::ModPkgArchive => load_archive_manifest(path),
        ModPackageKind::FantomeArchive => load_fantome_manifest(path),
    }
}

/// Load a native `.modpkg` archive, which embeds our own `manifest.json`.
fn load_archive_manifest(path: &Path) -> Result<ModManifest, ManifestError> {
    let file = fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    let manifest_names = ["manifest.json", "modpkg.json"];

    for name in manifest_names {
        if let Ok(mut entry) = archive.by_name(name) {
            let mut contents = String::new();
            entry.read_to_string(&mut contents)?;
            return Ok(serde_json::from_str(&contents)?);
        }
    }

    Err(ManifestError::MissingArchiveManifest)
}

/// Legacy Fantome/cslol metadata, stored in `META/info.json` with PascalCase keys.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct FantomeInfo {
    name: String,
    #[serde(default)]
    author: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    description: String,
}

/// Load a legacy `.fantome` archive and normalize it into our internal model.
///
/// A `.fantome` does not carry our manifest; it stores `META/info.json` metadata
/// plus content under `WAD/` (game archive overrides) and `RAW/` (raw files).
fn load_fantome_manifest(path: &Path) -> Result<ModManifest, ManifestError> {
    let file = fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    let info = read_fantome_info(&mut archive)?;
    let names: Vec<String> = archive.file_names().map(|name| name.to_string()).collect();
    Ok(normalize_fantome(info, &names))
}

fn read_fantome_info(
    archive: &mut zip::ZipArchive<fs::File>,
) -> Result<FantomeInfo, ManifestError> {
    for name in ["META/info.json", "meta/info.json", "info.json"] {
        if let Ok(mut entry) = archive.by_name(name) {
            let mut contents = String::new();
            entry.read_to_string(&mut contents)?;
            return Ok(serde_json::from_str(&contents)?);
        }
    }

    Err(ManifestError::MissingArchiveManifest)
}

fn normalize_fantome(info: FantomeInfo, names: &[String]) -> ModManifest {
    let assets = fantome_assets_from_names(names);
    let preview_image = names
        .iter()
        .find(|name| {
            let lower = name.to_ascii_lowercase();
            lower == "image.png" || lower == "meta/image.png"
        })
        .cloned();
    let version = if info.version.trim().is_empty() {
        "0.0.0".to_string()
    } else {
        info.version
    };

    ModManifest {
        schema_version: 1,
        id: Uuid::new_v4(),
        name: info.name,
        version,
        author: info.author,
        description: info.description,
        tags: vec!["fantome".to_string()],
        preview_image,
        assets,
    }
}

/// Map Fantome archive entry names into normalized assets.
///
/// `WAD/<wad>/<path>` targets `<path>` inside `<wad>`; a bare `WAD/<wad>` file is a
/// whole-WAD replacement; `RAW/<path>` entries use the `RAW` sentinel wad. Directory
/// entries and metadata (`META/`, preview images) are ignored.
fn fantome_assets_from_names(names: &[String]) -> Vec<ModAsset> {
    let mut assets = Vec::new();

    for raw_name in names {
        if raw_name.ends_with('/') {
            continue; // directory entry
        }
        let name = raw_name.replace('\\', "/");

        if let Some(rest) = name.strip_prefix("WAD/") {
            if rest.is_empty() {
                continue;
            }
            let (wad, target) = match rest.split_once('/') {
                Some((wad, target)) if !target.is_empty() => (wad.to_string(), target.to_string()),
                _ => (rest.to_string(), rest.to_string()),
            };
            assets.push(ModAsset {
                source: name,
                target,
                wad,
                layer: Some("wad".to_string()),
                sha256: None,
            });
        } else if let Some(rest) = name.strip_prefix("RAW/") {
            if rest.is_empty() {
                continue;
            }
            assets.push(ModAsset {
                source: name.clone(),
                target: rest.to_string(),
                wad: "RAW".to_string(),
                layer: Some("raw".to_string()),
                sha256: None,
            });
        }
    }

    assets
}

/// Read the bytes of a single asset from its package, regardless of layout.
///
/// For directory packages `source` is resolved relative to the package root;
/// for `.modpkg`/`.fantome` archives it is the name of an entry inside the zip.
pub fn read_package_asset(
    package_path: impl AsRef<Path>,
    source: &str,
) -> Result<Vec<u8>, ManifestError> {
    let package_path = package_path.as_ref();
    match detect_package_kind(package_path)? {
        ModPackageKind::Directory => {
            let asset_path = if package_path.is_dir() {
                package_path.join(source)
            } else {
                package_path
                    .parent()
                    .map(|parent| parent.join(source))
                    .unwrap_or_else(|| Path::new(source).to_path_buf())
            };
            Ok(fs::read(asset_path)?)
        }
        ModPackageKind::ModPkgArchive | ModPackageKind::FantomeArchive => {
            let file = fs::File::open(package_path)?;
            let mut archive = zip::ZipArchive::new(file)?;
            let mut entry = archive.by_name(source)?;
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            Ok(bytes)
        }
    }
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

    #[test]
    fn normalizes_fantome_entry_names_into_assets() {
        let names = vec![
            "META/info.json".to_string(),
            "WAD/".to_string(),
            "WAD/Aatrox.wad.client/data/characters/aatrox/skin01.bin".to_string(),
            "WAD/Map11.wad.client".to_string(),
            "RAW/DATA/menu/hud.bin".to_string(),
            "image.png".to_string(),
        ];

        let assets = fantome_assets_from_names(&names);
        assert_eq!(assets.len(), 3);

        let nested = &assets[0];
        assert_eq!(nested.wad, "Aatrox.wad.client");
        assert_eq!(nested.target, "data/characters/aatrox/skin01.bin");

        let whole_wad = &assets[1];
        assert_eq!(whole_wad.wad, "Map11.wad.client");
        assert_eq!(whole_wad.target, "Map11.wad.client");

        let raw = &assets[2];
        assert_eq!(raw.wad, "RAW");
        assert_eq!(raw.target, "DATA/menu/hud.bin");
    }

    #[test]
    fn loads_manifest_from_fantome_archive() {
        use std::io::Write;

        let dir = tempfile::tempdir().expect("tempdir");
        let archive_path = dir.path().join("crimson-aatrox.fantome");
        let file = fs::File::create(&archive_path).expect("archive file");
        let mut archive = zip::ZipWriter::new(file);
        let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();

        archive.start_file("META/info.json", options).expect("start info");
        let info = r#"{"Name":"Crimson Aatrox","Author":"someone","Version":"1.2.0","Description":"Legacy skin","Home":"https://example.test"}"#;
        archive.write_all(info.as_bytes()).expect("write info");

        archive
            .start_file("WAD/Aatrox.wad.client/data/characters/aatrox/skin01.bin", options)
            .expect("start wad asset");
        archive.write_all(b"binary").expect("write wad asset");

        archive.start_file("image.png", options).expect("start preview");
        archive.write_all(b"png").expect("write preview");
        archive.finish().expect("finish archive");

        let loaded = load_manifest(&archive_path).expect("load fantome manifest");
        assert_eq!(loaded.name, "Crimson Aatrox");
        assert_eq!(loaded.author, "someone");
        assert_eq!(loaded.version, "1.2.0");
        assert!(loaded.tags.contains(&"fantome".to_string()));
        assert_eq!(loaded.preview_image.as_deref(), Some("image.png"));
        assert_eq!(loaded.assets.len(), 1);
        assert_eq!(loaded.assets[0].wad, "Aatrox.wad.client");
        assert!(loaded.validate().ok);
    }

    #[test]
    fn reads_asset_bytes_from_directory_package() {
        let dir = tempfile::tempdir().expect("tempdir");
        let package = dir.path().join("pkg");
        fs::create_dir_all(package.join("assets")).expect("create assets dir");
        fs::write(package.join("assets/skin.bin"), b"raw-bytes").expect("write asset");

        let bytes = read_package_asset(&package, "assets/skin.bin").expect("read asset");
        assert_eq!(bytes, b"raw-bytes");
    }

    #[test]
    fn reads_asset_bytes_from_archive_package() {
        use std::io::Write;

        let dir = tempfile::tempdir().expect("tempdir");
        let archive_path = dir.path().join("pkg.modpkg");
        let file = fs::File::create(&archive_path).expect("archive file");
        let mut archive = zip::ZipWriter::new(file);
        let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
        archive
            .start_file("assets/skin.bin", options)
            .expect("start asset");
        archive.write_all(b"zip-bytes").expect("write asset");
        archive.finish().expect("finish archive");

        let bytes = read_package_asset(&archive_path, "assets/skin.bin").expect("read asset");
        assert_eq!(bytes, b"zip-bytes");
    }
}
