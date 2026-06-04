use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;
use std::sync::OnceLock;
use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_CREATION_DISPOSITION, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_MODE,
};
use windows_sys::Win32::System::LibraryLoader::DisableThreadLibraryCalls;
use windows_sys::Win32::System::SystemServices::{DLL_PROCESS_ATTACH, DLL_PROCESS_DETACH};

/// Global configuration for file redirections.
/// Maps a normalized target suffix (lowercase, `/` separators) -> staged replacement path.
static REDIRECTION_MAP: OnceLock<std::collections::HashMap<String, PathBuf>> = OnceLock::new();

/// HINSTANCE of this DLL, captured in `DllMain`.
static DLL_INSTANCE: OnceLock<isize> = OnceLock::new();

/// Signature of `CreateFileW`. The hook and the stored trampoline must match it exactly.
type CreateFileWFn = unsafe extern "system" fn(
    *const u16,
    u32,
    FILE_SHARE_MODE,
    *const windows_sys::Win32::Security::SECURITY_ATTRIBUTES,
    FILE_CREATION_DISPOSITION,
    FILE_FLAGS_AND_ATTRIBUTES,
    HANDLE,
) -> HANDLE;

/// Trampoline to the original `CreateFileW`. Set once during setup before hooks are enabled,
/// so it can be read concurrently from the hook without `static mut` / data races.
static ORIGINAL_CREATE_FILE_W: OnceLock<CreateFileWFn> = OnceLock::new();

/// Normalize a path for matching: forward slashes, lowercase.
fn normalize(path: &str) -> String {
    path.replace('\\', "/").to_ascii_lowercase()
}

/// Append a line to `manager_hook.log` next to the DLL. Best-effort; never panics.
/// Injected DLLs have no console, so this file is the only way to debug the hook.
fn log_line(msg: &str) {
    if let Ok(dir) = get_dll_directory() {
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("manager_hook.log"))
        {
            use std::io::Write;
            let _ = writeln!(file, "{msg}");
        }
    }
}

/// Reads `redirections.json` from the directory where the DLL is located and builds the map.
fn init_redirections() {
    let mut map = std::collections::HashMap::new();

    match get_dll_directory() {
        Ok(dll_dir) => {
            let config_path = dll_dir.join("redirections.json");
            if !config_path.exists() {
                log_line(&format!(
                    "no redirections.json found at {}",
                    config_path.display()
                ));
            } else {
                match std::fs::read_to_string(&config_path) {
                    Ok(content) => match serde_json::from_str::<serde_json::Value>(
                        // Tolerate a UTF-8 BOM — editors like Notepad add one and
                        // serde_json otherwise fails with "expected value at line 1".
                        content.strip_prefix('\u{feff}').unwrap_or(&content),
                    ) {
                        Ok(parsed) => {
                            if let Some(obj) = parsed.as_object() {
                                for (key, val) in obj {
                                    if let Some(val_str) = val.as_str() {
                                        // Normalize the key exactly like lookups so keys written
                                        // with backslashes or mixed case still match.
                                        map.insert(normalize(key), PathBuf::from(val_str));
                                    }
                                }
                            }
                        }
                        Err(e) => log_line(&format!("failed to parse redirections.json: {e}")),
                    },
                    Err(e) => log_line(&format!("failed to read redirections.json: {e}")),
                }
            }
        }
        Err(_) => log_line("failed to resolve DLL directory; no redirections loaded"),
    }

    let _ = REDIRECTION_MAP.set(map);
}

fn get_dll_directory() -> Result<PathBuf, ()> {
    let mut path_buf = [0u16; 1024];
    let len = unsafe {
        windows_sys::Win32::System::LibraryLoader::GetModuleFileNameW(
            get_dll_instance_handle() as _,
            path_buf.as_mut_ptr(),
            path_buf.len() as u32,
        )
    };
    if len == 0 {
        return Err(());
    }
    let os_str = OsString::from_wide(&path_buf[..len as usize]);
    let path = PathBuf::from(os_str);
    path.parent().map(|p| p.to_path_buf()).ok_or(())
}

fn get_dll_instance_handle() -> isize {
    *DLL_INSTANCE.get().unwrap_or(&0)
}

/// Returns true if `path` ends with `key` on a path-component boundary, so
/// `aatrox.wad.client` does not also match `xaatrox.wad.client`.
fn suffix_matches(path: &str, key: &str) -> bool {
    if !path.ends_with(key) {
        return false;
    }
    let prefix_len = path.len() - key.len();
    prefix_len == 0 || path.as_bytes()[prefix_len - 1] == b'/'
}

/// Hooked version of CreateFileW. Redirects matching paths to staged replacements.
#[allow(non_snake_case)]
unsafe extern "system" fn Hook_CreateFileW(
    lp_file_name: *const u16,
    dw_desired_access: u32,
    dw_share_mode: FILE_SHARE_MODE,
    lp_security_attributes: *const windows_sys::Win32::Security::SECURITY_ATTRIBUTES,
    dw_creation_disposition: FILE_CREATION_DISPOSITION,
    dw_flags_and_attributes: FILE_FLAGS_AND_ATTRIBUTES,
    h_template_file: HANDLE,
) -> HANDLE {
    // The trampoline is always set before hooks are enabled; if somehow missing we cannot
    // forward the call, so fail the open rather than recurse or panic across the FFI boundary.
    let Some(&original) = ORIGINAL_CREATE_FILE_W.get() else {
        return INVALID_HANDLE_VALUE;
    };

    if !lp_file_name.is_null() {
        // Read the null-terminated wide string into a Rust string.
        let mut len = 0isize;
        while *lp_file_name.offset(len) != 0 {
            len += 1;
        }
        let wide_slice = std::slice::from_raw_parts(lp_file_name, len as usize);
        let os_str = OsString::from_wide(wide_slice);
        let normalized_path = normalize(&os_str.to_string_lossy());

        if let Some(map) = REDIRECTION_MAP.get() {
            // Redirect if any configured key is a path-boundary suffix of the requested path.
            // E.g. game opens "C:/Games/LoL/Game/DATA/Menu.wad.client" and the key is
            // "data/menu.wad.client" -> we point it at the staged replacement instead.
            if let Some((key, target_path)) = map
                .iter()
                .find(|(key, _)| suffix_matches(&normalized_path, key))
            {
                log_line(&format!(
                    "redirecting {normalized_path} -> {} (key: {key})",
                    target_path.display()
                ));
                let wide_target: Vec<u16> = target_path
                    .as_os_str()
                    .encode_wide()
                    .chain(std::iter::once(0))
                    .collect();

                return original(
                    wide_target.as_ptr(),
                    dw_desired_access,
                    dw_share_mode,
                    lp_security_attributes,
                    dw_creation_disposition,
                    dw_flags_and_attributes,
                    h_template_file,
                );
            }
        }
    }

    // No match: forward to the original function unchanged.
    original(
        lp_file_name,
        dw_desired_access,
        dw_share_mode,
        lp_security_attributes,
        dw_creation_disposition,
        dw_flags_and_attributes,
        h_template_file,
    )
}

/// Installs and enables the CreateFileW hook. Runs on a dedicated thread so that no file I/O
/// or thread-suspending MinHook work happens while the DLL loader lock is held.
unsafe fn setup_hook() {
    init_redirections();
    let count = REDIRECTION_MAP.get().map(|m| m.len()).unwrap_or(0);
    log_line(&format!("hook DLL attached; {count} redirection(s) loaded"));

    match minhook::MinHook::create_hook(CreateFileW as *mut _, Hook_CreateFileW as *mut _) {
        Ok(orig) => {
            let original: CreateFileWFn = std::mem::transmute(orig);
            let _ = ORIGINAL_CREATE_FILE_W.set(original);
            match minhook::MinHook::enable_all_hooks() {
                Ok(_) => log_line("CreateFileW hook enabled"),
                Err(e) => log_line(&format!("failed to enable hooks: {e:?}")),
            }
        }
        Err(e) => log_line(&format!("failed to create CreateFileW hook: {e:?}")),
    }
}

#[no_mangle]
#[allow(non_snake_case, unused_variables)]
unsafe extern "system" fn DllMain(
    hinst_dll: isize,
    fdw_reason: u32,
    lpv_reserved: *mut std::ffi::c_void,
) -> i32 {
    match fdw_reason {
        DLL_PROCESS_ATTACH => {
            let _ = DLL_INSTANCE.set(hinst_dll);
            // We don't need per-thread attach/detach notifications.
            DisableThreadLibraryCalls(hinst_dll as _);
            // Defer hook installation off the loader lock: MinHook suspends/enumerates threads
            // and we touch the filesystem, both of which can deadlock under the lock.
            std::thread::spawn(|| unsafe { setup_hook() });
        }
        DLL_PROCESS_DETACH => {
            let _ = minhook::MinHook::disable_all_hooks();
        }
        _ => {}
    }
    1
}
