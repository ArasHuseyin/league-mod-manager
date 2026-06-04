use crate::manifest::{detect_package_kind, load_manifest, ManifestError, ModPackageKind, ModManifest};
use crate::profile::Profile;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{fs, io, path::Path, path::PathBuf};
use thiserror::Error;
use time::OffsetDateTime;


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryItem {
    pub manifest: ModManifest,
    pub package_path: PathBuf,
    pub imported_at: OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    pub root: PathBuf,
    pub packages: PathBuf,
    pub cache: PathBuf,
    pub logs: PathBuf,
    pub database: PathBuf,
    pub state_file: PathBuf,
}

impl AppPaths {
    /// Build the standard path layout under an explicit data root.
    /// Useful for tests and for callers that want full control over the location.
    pub fn from_root(root: PathBuf) -> Self {
        Self {
            packages: root.join("packages"),
            cache: root.join("cache"),
            logs: root.join("logs"),
            database: root.join("manager.sqlite"),
            state_file: root.join("state.json"),
            root,
        }
    }

    /// Discover the per-user data directory for this application.
    pub fn discover() -> Option<Self> {
        ProjectDirs::from("dev", "LeagueModManager", "League Mod Manager")
            .map(|dirs| Self::from_root(dirs.data_dir().to_path_buf()))
    }
}

/// Persisted application state: the imported library and saved profiles.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PersistedState {
    #[serde(default)]
    pub library: Vec<LibraryItem>,
    #[serde(default)]
    pub profiles: Vec<Profile>,
}

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("failed to access state file {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to parse state file {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to encode state: {0}")]
    Encode(#[source] serde_json::Error),
}

/// Load persisted state from disk. A missing state file is treated as a fresh
/// install and returns the default (empty) state rather than an error.
pub fn load_state(paths: &AppPaths) -> Result<PersistedState, StorageError> {
    let path = &paths.state_file;
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|source| StorageError::Parse {
            path: path.clone(),
            source,
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(PersistedState::default()),
        Err(source) => Err(StorageError::Io {
            path: path.clone(),
            source,
        }),
    }
}

/// Persist state to disk atomically: write to a sibling temp file, then rename
/// over the target so a crash mid-write cannot corrupt the existing state.
pub fn save_state(paths: &AppPaths, state: &PersistedState) -> Result<(), StorageError> {
    fs::create_dir_all(&paths.root).map_err(|source| StorageError::Io {
        path: paths.root.clone(),
        source,
    })?;

    let json = serde_json::to_vec_pretty(state).map_err(StorageError::Encode)?;

    let tmp = paths.state_file.with_extension("json.tmp");
    fs::write(&tmp, &json).map_err(|source| StorageError::Io {
        path: tmp.clone(),
        source,
    })?;
    fs::rename(&tmp, &paths.state_file).map_err(|source| StorageError::Io {
        path: paths.state_file.clone(),
        source,
    })
}

/// Import a package from `src` by validating it, copying or extracting it
/// to `packages_dir/manifest_id/`, writing the normalized `manifest.json` there,
/// and returning the created `LibraryItem`.
pub fn import_package(
    src: &Path,
    packages_dir: &Path,
) -> Result<LibraryItem, ManifestError> {
    let manifest = load_manifest(src)?;
    let validation = manifest.validate();
    if !validation.ok {
        return Err(ManifestError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            validation.errors.join("; "),
        )));
    }

    let target_dir = packages_dir.join(manifest.id.to_string());
    if target_dir.exists() {
        fs::remove_dir_all(&target_dir)?;
    }
    fs::create_dir_all(&target_dir)?;

    // Extract the assets
    let kind = detect_package_kind(src)?;
    match kind {
        ModPackageKind::Directory => {
            copy_dir_all(src, &target_dir)?;
        }
        ModPackageKind::ModPkgArchive | ModPackageKind::FantomeArchive => {
            let file = fs::File::open(src)?;
            let mut archive = zip::ZipArchive::new(file)?;
            for i in 0..archive.len() {
                let mut file = archive.by_index(i)?;
                let outpath = match file.enclosed_name() {
                    Some(path) => target_dir.join(path),
                    None => continue,
                };

                if file.name().ends_with('/') {
                    fs::create_dir_all(&outpath)?;
                } else {
                    if let Some(p) = outpath.parent() {
                        if !p.exists() {
                            fs::create_dir_all(p)?;
                        }
                    }
                    let mut outfile = fs::File::create(&outpath)?;
                    std::io::copy(&mut file, &mut outfile)?;
                }
            }
        }
    }

    // Always write the normalized manifest.json into the target directory.
    // This converts legacy `.fantome` / `.zip` archives into standard directory packages.
    let manifest_path = target_dir.join("manifest.json");
    let json = serde_json::to_string_pretty(&manifest).map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::Other, format!("failed to serialize manifest: {e}"))
    })?;
    fs::write(manifest_path, json)?;

    Ok(LibraryItem {
        manifest,
        package_path: target_dir,
        imported_at: OffsetDateTime::now_utc(),
    })
}

fn copy_dir_all(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> Result<(), std::io::Error> {
    let src = src.as_ref();
    let dst = dst.as_ref();
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(entry.path(), dst.join(entry.file_name()))?;
        } else {
            fs::copy(entry.path(), dst.join(entry.file_name()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{ModAsset, ModManifest};
    use uuid::Uuid;

    fn sample_state() -> PersistedState {
        let id = Uuid::new_v4();
        let item = LibraryItem {
            manifest: ModManifest {
                schema_version: 1,
                id,
                name: "Sample".to_string(),
                version: "0.1.0".to_string(),
                author: "Tester".to_string(),
                description: "Round-trip fixture".to_string(),
                tags: vec!["test".to_string()],
                preview_image: None,
                assets: vec![ModAsset {
                    source: "assets/a.bin".to_string(),
                    target: "data/a.bin".to_string(),
                    wad: "DATA/Menu.wad.client".to_string(),
                    layer: None,
                    sha256: None,
                }],
            },
            package_path: PathBuf::from("samples/sample"),
            imported_at: OffsetDateTime::UNIX_EPOCH,
        };
        let mut profile = Profile::new("Default");
        profile.enable_mod(id);
        PersistedState {
            library: vec![item],
            profiles: vec![profile],
        }
    }

    #[test]
    fn missing_state_file_loads_default() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths::from_root(dir.path().to_path_buf());
        let loaded = load_state(&paths).unwrap();
        assert_eq!(loaded, PersistedState::default());
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths::from_root(dir.path().join("data"));
        let state = sample_state();

        save_state(&paths, &state).unwrap();
        assert!(paths.state_file.exists());

        let loaded = load_state(&paths).unwrap();
        assert_eq!(loaded, state);
    }

    #[test]
    fn save_overwrites_existing_state() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths::from_root(dir.path().to_path_buf());

        save_state(&paths, &sample_state()).unwrap();
        let empty = PersistedState::default();
        save_state(&paths, &empty).unwrap();

        assert_eq!(load_state(&paths).unwrap(), empty);
    }

    #[test]
    fn import_package_extracts_and_creates_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let packages_dir = dir.path().join("packages");
        let src_dir = dir.path().join("my-mod");
        fs::create_dir_all(&src_dir).unwrap();

        let manifest = ModManifest {
            schema_version: 1,
            id: Uuid::new_v4(),
            name: "Test Skin".to_string(),
            version: "1.0.0".to_string(),
            author: "Tester".to_string(),
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
        };

        fs::write(
            src_dir.join("manifest.json"),
            serde_json::to_string(&manifest).unwrap(),
        ).unwrap();
        fs::create_dir_all(src_dir.join("assets")).unwrap();
        fs::write(src_dir.join("assets/skin.bin"), b"skin-data").unwrap();

        let item = import_package(&src_dir, &packages_dir).unwrap();

        assert_eq!(item.manifest.name, "Test Skin");
        assert!(item.package_path.exists());
        assert!(item.package_path.join("manifest.json").exists());
        assert!(item.package_path.join("assets/skin.bin").exists());
        assert_eq!(
            fs::read(item.package_path.join("assets/skin.bin")).unwrap(),
            b"skin-data"
        );
    }
}
