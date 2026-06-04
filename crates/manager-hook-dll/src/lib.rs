use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::sync::OnceLock;
use std::path::PathBuf;
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_MODE, FILE_CREATION_DISPOSITION, FILE_FLAGS_AND_ATTRIBUTES
};
use windows_sys::Win32::System::SystemServices::{DLL_PROCESS_ATTACH, DLL_PROCESS_DETACH};
use std::os::windows::ffi::OsStrExt;

/// Global configuration for file redirections.
/// We store the mapping of: Target file path (lowercase string) -> Staged replacement path.
static REDIRECTION_MAP: OnceLock<std::collections::HashMap<String, PathBuf>> = OnceLock::new();

/// Initialized when the DLL attaches. Reads a config file `redirections.json`
/// from the directory where the DLL is located.
fn init_redirections() {
    let mut map = std::collections::HashMap::new();

    // Look for redirections.json alongside the DLL itself.
    if let Ok(dll_dir) = get_dll_directory() {
        let config_path = dll_dir.join("redirections.json");
        if config_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&config_path) {
                if let Ok(parsed) = serde_json_from_str::<serde_json::Value>(&content) {
                    if let Some(obj) = parsed.as_object() {
                        for (key, val) in obj {
                            if let Some(val_str) = val.as_str() {
                                // Store the key in lowercase for case-insensitive lookup
                                map.insert(key.to_ascii_lowercase(), PathBuf::from(val_str));
                            }
                        }
                    }
                }
            }
        }
    }

    let _ = REDIRECTION_MAP.set(map);
}

/// Simple JSON parser fallback since we want to keep the DLL lightweight
fn serde_json_from_str<T: serde::de::DeserializeOwned>(s: &str) -> Result<T, ()> {
    serde_json::from_str(s).map_err(|_| ())
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

static DLL_INSTANCE: OnceLock<isize> = OnceLock::new();

fn get_dll_instance_handle() -> isize {
    *DLL_INSTANCE.get().unwrap_or(&0)
}

// Global reference to call the original CreateFileW function.
static mut ORIGINAL_CREATE_FILE_W: Option<unsafe extern "system" fn(
    *const u16,
    u32,
    FILE_SHARE_MODE,
    *const windows_sys::Win32::Security::SECURITY_ATTRIBUTES,
    FILE_CREATION_DISPOSITION,
    FILE_FLAGS_AND_ATTRIBUTES,
    HANDLE
) -> HANDLE> = None;

/// Hooked version of CreateFileW. Intercepts calls and redirects paths if they match our redirections.json config.
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
    let original = ORIGINAL_CREATE_FILE_W.expect("original function pointer must be initialized");

    if !lp_file_name.is_null() {
        // Read wide string into Rust string
        let mut len = 0;
        while *lp_file_name.offset(len) != 0 {
            len += 1;
        }
        let wide_slice = std::slice::from_raw_parts(lp_file_name, len as usize);
        let os_str = OsString::from_wide(wide_slice);
        let path_str = os_str.to_string_lossy();
        let normalized_path = path_str.replace('\\', "/").to_ascii_lowercase();

        if let Some(map) = REDIRECTION_MAP.get() {
            // Check if this path (or a sub-path / file name) is mapped for redirection
            // We search if any key in our redirection map is a suffix of the normalized path.
            // E.g., if the game opens "C:/Games/LoL/Game/DATA/Menu.wad.client"
            // and our key is "data/menu.wad.client", we redirect it!
            if let Some((_, target_path)) = map.iter().find(|(key, _)| normalized_path.ends_with(key.as_str())) {
                // Redirect! Convert the target replacement path back to wide character string
                let wide_target: Vec<u16> = target_path
                    .as_os_str()
                    .encode_wide()
                    .chain(std::iter::once(0))
                    .collect();

                // Call the original CreateFileW with our new redirected path
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

    // Call original function if no match
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

// We use standard library OsStrExt directly for encoding wide strings.

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
            init_redirections();

            // Setup the CreateFileW hook using minhook
            let original_ptr = minhook::MinHook::create_hook(
                CreateFileW as *mut _,
                Hook_CreateFileW as *mut _,
            );
            if let Ok(orig) = original_ptr {
                ORIGINAL_CREATE_FILE_W = Some(std::mem::transmute(orig));
                let _ = minhook::MinHook::enable_all_hooks();
            }
        }
        DLL_PROCESS_DETACH => {
            let _ = minhook::MinHook::disable_all_hooks();
        }
        _ => {}
    }
    1
}

