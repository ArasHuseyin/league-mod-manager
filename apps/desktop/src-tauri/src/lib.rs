use manager_core::{
    assess_manifest_policy, detect_league_installations, load_manifest, load_state, save_state,
    AppPaths, LibraryItem, ModAsset, ModManifest, PersistedState, Profile,
};
use manager_patcher::{PatchEngine, PatchRequest};
use std::{path::PathBuf, sync::Mutex};
use time::OffsetDateTime;
use uuid::Uuid;

struct AppState {
    library: Mutex<Vec<LibraryItem>>,
    profiles: Mutex<Vec<Profile>>,
    paths: Option<AppPaths>,
}

impl AppState {
    /// Snapshot the current library and profiles and write them to disk.
    /// Must be called without holding either inner lock to avoid deadlocks.
    fn persist(&self) {
        let Some(paths) = &self.paths else {
            return;
        };
        let state = PersistedState {
            library: self.library.lock().expect("library lock").clone(),
            profiles: self.profiles.lock().expect("profile lock").clone(),
        };
        if let Err(error) = save_state(paths, &state) {
            eprintln!("failed to persist state: {error}");
        }
    }
}

#[tauri::command]
fn get_library(state: tauri::State<AppState>) -> Vec<LibraryItem> {
    state.library.lock().expect("library lock").clone()
}

#[tauri::command]
fn get_profiles(state: tauri::State<AppState>) -> Vec<Profile> {
    state.profiles.lock().expect("profile lock").clone()
}

#[tauri::command]
fn detect_league() -> Vec<manager_core::LeagueInstallation> {
    detect_league_installations()
}

#[tauri::command]
fn import_mod(path: String, state: tauri::State<AppState>) -> Result<LibraryItem, String> {
    let path_buf = PathBuf::from(path);
    let manifest = load_manifest(&path_buf).map_err(|error| error.to_string())?;
    let validation = manifest.validate();
    if !validation.ok {
        return Err(validation.errors.join("; "));
    }

    let _policy = assess_manifest_policy(&manifest);
    let item = LibraryItem {
        manifest,
        package_path: path_buf,
        imported_at: OffsetDateTime::now_utc(),
    };
    state.library.lock().expect("library lock").push(item.clone());
    state.persist();
    Ok(item)
}

#[tauri::command]
fn create_profile(name: String, state: tauri::State<AppState>) -> Profile {
    let profile = Profile::new(name);
    state
        .profiles
        .lock()
        .expect("profile lock")
        .push(profile.clone());
    state.persist();
    profile
}

#[tauri::command]
fn set_profile_mod_enabled(
    profile_id: Uuid,
    mod_id: Uuid,
    enabled: bool,
    state: tauri::State<AppState>,
) -> Result<Profile, String> {
    let mut profiles = state.profiles.lock().expect("profile lock");
    let profile = profiles
        .iter_mut()
        .find(|profile| profile.id == profile_id)
        .ok_or_else(|| "profile not found".to_string())?;

    if enabled {
        profile.enable_mod(mod_id);
    } else {
        profile.disable_mod(mod_id);
    }

    let updated = profile.clone();
    drop(profiles);
    state.persist();
    Ok(updated)
}

#[tauri::command]
fn plan_patch(
    profile_id: Uuid,
    league_root: String,
    state: tauri::State<AppState>,
) -> Result<manager_patcher::PatchReport, String> {
    let profiles = state.profiles.lock().expect("profile lock");
    let profile = profiles
        .iter()
        .find(|profile| profile.id == profile_id)
        .cloned()
        .ok_or_else(|| "profile not found".to_string())?;
    drop(profiles);

    PatchEngine::plan(PatchRequest {
        league_root: PathBuf::from(league_root),
        dry_run: true,
        profile,
        library: state.library.lock().expect("library lock").clone(),
    })
    .map_err(|error| error.to_string())
}

fn seed_library() -> Vec<LibraryItem> {
    let first_id = Uuid::parse_str("1f2f0a77-9648-4f7b-ae09-fd9c76995a12").unwrap();
    let second_id = Uuid::parse_str("fb657393-e6a9-466b-8719-0bfe8223b331").unwrap();
    vec![
        LibraryItem {
            manifest: ModManifest {
                schema_version: 1,
                id: first_id,
                name: "Aatrox Crimson VFX".to_string(),
                version: "0.1.0".to_string(),
                author: "Workshop".to_string(),
                description: "Sample mod package for UI development.".to_string(),
                tags: vec!["champion".to_string(), "vfx".to_string()],
                preview_image: None,
                assets: vec![ModAsset {
                    source: "assets/aatrox/vfx.bin".to_string(),
                    target: "data/characters/aatrox/skins/skin01/vfx.bin".to_string(),
                    wad: "Characters/Aatrox.wad.client".to_string(),
                    layer: Some("vfx".to_string()),
                    sha256: None,
                }],
            },
            package_path: PathBuf::from("samples/aatrox-crimson-vfx"),
            imported_at: OffsetDateTime::UNIX_EPOCH,
        },
        LibraryItem {
            manifest: ModManifest {
                schema_version: 1,
                id: second_id,
                name: "SR Minimal HUD".to_string(),
                version: "0.1.0".to_string(),
                author: "Workshop".to_string(),
                description: "Sample interface package for UI development.".to_string(),
                tags: vec!["ui".to_string(), "hud".to_string()],
                preview_image: None,
                assets: vec![ModAsset {
                    source: "assets/hud/layout.bin".to_string(),
                    target: "data/menu/hud/layout.bin".to_string(),
                    wad: "DATA/Menu.wad.client".to_string(),
                    layer: Some("interface".to_string()),
                    sha256: None,
                }],
            },
            package_path: PathBuf::from("samples/sr-minimal-hud"),
            imported_at: OffsetDateTime::UNIX_EPOCH,
        },
    ]
}

fn seed_profiles() -> Vec<Profile> {
    let mut default = Profile::new("Ranked safe");
    default.enable_mod(Uuid::parse_str("1f2f0a77-9648-4f7b-ae09-fd9c76995a12").unwrap());
    vec![default, Profile::new("Workshop testing")]
}

/// Load persisted state, or seed first-run demo data and write it to disk.
fn load_or_seed_state() -> (Option<AppPaths>, PersistedState) {
    let paths = AppPaths::discover();

    let first_run = paths
        .as_ref()
        .map(|paths| !paths.state_file.exists())
        .unwrap_or(true);

    if first_run {
        let state = PersistedState {
            library: seed_library(),
            profiles: seed_profiles(),
        };
        if let Some(paths) = &paths {
            if let Err(error) = save_state(paths, &state) {
                eprintln!("failed to seed state: {error}");
            }
        }
        return (paths, state);
    }

    let state = paths
        .as_ref()
        .map(|paths| match load_state(paths) {
            Ok(state) => state,
            Err(error) => {
                eprintln!("failed to load state, starting empty: {error}");
                PersistedState::default()
            }
        })
        .unwrap_or_default();

    (paths, state)
}

pub fn run() {
    let (paths, state) = load_or_seed_state();

    tauri::Builder::default()
        .manage(AppState {
            library: Mutex::new(state.library),
            profiles: Mutex::new(state.profiles),
            paths,
        })
        .invoke_handler(tauri::generate_handler![
            get_library,
            get_profiles,
            detect_league,
            import_mod,
            create_profile,
            set_profile_mod_enabled,
            plan_patch
        ])
        .run(tauri::generate_context!())
        .expect("error while running Tauri application");
}
