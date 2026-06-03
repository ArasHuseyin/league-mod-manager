use crate::manifest::ModManifest;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
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
}

impl AppPaths {
    pub fn discover() -> Option<Self> {
        ProjectDirs::from("dev", "LeagueModManager", "League Mod Manager").map(|dirs| {
            let root = dirs.data_dir().to_path_buf();
            Self {
                packages: root.join("packages"),
                cache: root.join("cache"),
                logs: root.join("logs"),
                database: root.join("manager.sqlite"),
                root,
            }
        })
    }
}
