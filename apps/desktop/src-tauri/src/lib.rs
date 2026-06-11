use manager_core::{
    detect_league_installations, import_package, load_state, save_state,
    AppPaths, LibraryItem, ModAsset, ModManifest, PersistedState, Profile,
};
use manager_patcher::{PatchEngine, PatchRequest, PatchStatus};
use std::process::Command;
use std::{fs, path::PathBuf, sync::Mutex};
use time::OffsetDateTime;
use uuid::Uuid;

/// The League game process that actually loads WAD archives — including in the
/// Practice Tool and custom games. The hook must be injected here, *not* into
/// `LeagueClient.exe` (the launcher), or no game asset opens are intercepted.
const GAME_PROCESS: &str = "League of Legends.exe";

/// Candidate file names for the injected hook DLL, in preference order.
const HOOK_DLL_NAMES: &[&str] = &["manager_hook_dll.dll"];

/// Candidate file names for the injector binary, in preference order.
const INJECTOR_NAMES: &[&str] = &["manager-injector.exe", "manager-injector"];

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
    let Some(paths) = &state.paths else {
        return Err("App paths are not initialized".to_string());
    };
    let item = import_package(&path_buf, &paths.packages).map_err(|error| error.to_string())?;
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

/// Result of an apply-and-inject run, surfaced to the UI.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyReport {
    /// Lowercased patch status: `applied`, `blocked`.
    pub status: String,
    pub staging_dir: String,
    pub staged_files: Vec<String>,
    pub redirection_count: usize,
    pub injector_started: bool,
    pub process_name: String,
    pub messages: Vec<String>,
}

/// Stage the profile's mods into game-loadable WADs, drop the hook DLL beside the
/// generated `redirections.json`, and start the injector watching for the game.
///
/// This is the live counterpart to [`plan_patch`]: where the dry run only reports
/// what *would* change, this writes the patched archives and arms the redirect
/// hook. The League installation itself is never modified — the hook redirects the
/// game's file opens to the staged copies. The injector waits for
/// [`GAME_PROCESS`] and injects once it appears, so it can be armed before or
/// after the game (or Practice Tool) is launched.
#[tauri::command]
fn apply_patch(
    profile_id: Uuid,
    league_root: String,
    state: tauri::State<AppState>,
) -> Result<ApplyReport, String> {
    let profile = state
        .profiles
        .lock()
        .expect("profile lock")
        .iter()
        .find(|profile| profile.id == profile_id)
        .cloned()
        .ok_or_else(|| "profile not found".to_string())?;
    let library = state.library.lock().expect("library lock").clone();

    let staging_dir = staging_dir(state.paths.as_ref())?;

    let report = PatchEngine::stage(
        &PatchRequest {
            league_root: PathBuf::from(&league_root),
            dry_run: false,
            profile,
            library,
        },
        &staging_dir,
    )
    .map_err(|error| error.to_string())?;

    let staging_display = staging_dir.to_string_lossy().into_owned();
    let staged_files: Vec<String> = report
        .staged
        .iter()
        .map(|staged| staged.output_path.to_string_lossy().into_owned())
        .collect();
    let mut messages = report.messages.clone();

    // Conflicts (or any non-applied status) stop here: nothing was written, so
    // there is nothing to inject.
    if report.status != PatchStatus::Applied {
        return Ok(ApplyReport {
            status: "blocked".to_string(),
            staging_dir: staging_display,
            staged_files,
            redirection_count: 0,
            injector_started: false,
            process_name: GAME_PROCESS.to_string(),
            messages,
        });
    }

    if report.staged.is_empty() {
        messages.push("No mods are enabled in this profile; nothing to inject.".to_string());
        return Ok(ApplyReport {
            status: "applied".to_string(),
            staging_dir: staging_display,
            staged_files,
            redirection_count: 0,
            injector_started: false,
            process_name: GAME_PROCESS.to_string(),
            messages,
        });
    }

    let redirection_count = count_redirections(&staging_dir);

    // The hook reads `redirections.json` from its own directory, so the DLL must
    // live next to the staged output.
    let dll_src = resolve_tool("LEAGUE_MOD_MANAGER_HOOK_DLL", HOOK_DLL_NAMES).ok_or_else(|| {
        "could not locate the hook DLL (manager_hook_dll.dll) next to the app; \
         build it with `cargo build -p manager-hook-dll` or set LEAGUE_MOD_MANAGER_HOOK_DLL"
            .to_string()
    })?;
    let dll_name = dll_src
        .file_name()
        .map(|name| name.to_owned())
        .ok_or_else(|| "hook DLL path has no file name".to_string())?;
    let dll_dst = staging_dir.join(&dll_name);
    fs::copy(&dll_src, &dll_dst).map_err(|error| format!("failed to copy hook DLL: {error}"))?;

    let injector = resolve_tool("LEAGUE_MOD_MANAGER_INJECTOR", INJECTOR_NAMES).ok_or_else(|| {
        "could not locate the injector (manager-injector) next to the app; \
         build it with `cargo build -p manager-injector` or set LEAGUE_MOD_MANAGER_INJECTOR"
            .to_string()
    })?;

    Command::new(&injector)
        .arg(GAME_PROCESS)
        .arg(&dll_dst)
        .spawn()
        .map_err(|error| format!("failed to start injector: {error}"))?;

    messages.push(format!(
        "Injector is watching for {GAME_PROCESS}; launch the game or Practice Tool to apply the mods."
    ));

    Ok(ApplyReport {
        status: "applied".to_string(),
        staging_dir: staging_display,
        staged_files,
        redirection_count,
        injector_started: true,
        process_name: GAME_PROCESS.to_string(),
        messages,
    })
}

/// Resolve the staging directory and ensure it starts empty, so stale overlays
/// or redirections from a previous profile never linger.
fn staging_dir(paths: Option<&AppPaths>) -> Result<PathBuf, String> {
    let base = match paths {
        Some(paths) => paths.cache.clone(),
        None => std::env::temp_dir().join("league-mod-manager"),
    };
    let dir = base.join("overlay");
    if dir.exists() {
        fs::remove_dir_all(&dir)
            .map_err(|error| format!("failed to clear staging dir {}: {error}", dir.display()))?;
    }
    fs::create_dir_all(&dir)
        .map_err(|error| format!("failed to create staging dir {}: {error}", dir.display()))?;
    Ok(dir)
}

/// Count the entries the hook will redirect, for display. Best-effort: a missing
/// or unreadable file simply reports zero.
fn count_redirections(staging_dir: &std::path::Path) -> usize {
    fs::read(staging_dir.join("redirections.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|value| value.as_object().map(|map| map.len()))
        .unwrap_or(0)
}

/// Find a bundled helper binary: an explicit env override wins, otherwise look
/// beside the running executable. In a workspace dev build every crate shares
/// one `target/` directory, so the injector and DLL sit next to the app there;
/// in a packaged build they ship as sidecars in the same folder.
fn resolve_tool(env_key: &str, names: &[&str]) -> Option<PathBuf> {
    if let Some(override_path) = std::env::var_os(env_key) {
        let path = PathBuf::from(override_path);
        if path.exists() {
            return Some(path);
        }
    }

    let exe_dir = std::env::current_exe().ok().and_then(|exe| exe.parent().map(PathBuf::from))?;
    names
        .iter()
        .map(|name| exe_dir.join(name))
        .find(|candidate| candidate.exists())
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

#[tauri::command]
fn select_directory() -> Option<String> {
    rfd::FileDialog::new()
        .pick_folder()
        .map(|path| path.to_string_lossy().into_owned())
}

#[tauri::command]
fn select_file() -> Option<String> {
    rfd::FileDialog::new()
        .add_filter("Mod Package", &["fantome", "zip"])
        .pick_file()
        .map(|path| path.to_string_lossy().into_owned())
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
            plan_patch,
            apply_patch,
            select_directory,
            select_file
        ])
        .run(tauri::generate_context!())
        .expect("error while running Tauri application");
}
