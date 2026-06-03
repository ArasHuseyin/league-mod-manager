use crate::manifest::ModManifest;
use crate::profile::Profile;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{fs, io, path::PathBuf};
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
}
