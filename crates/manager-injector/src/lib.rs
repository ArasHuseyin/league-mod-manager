use anyhow::{anyhow, Result};
use std::ffi::CString;
use std::path::Path;
use std::ptr;
use std::thread;
use std::time::Duration;
use windows_sys::Win32::Foundation::{CloseHandle, FALSE, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32First, Process32Next, PROCESSENTRY32, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleA;
use windows_sys::Win32::System::Memory::{
    VirtualAllocEx, VirtualFreeEx, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE,
};
use windows_sys::Win32::System::Diagnostics::Debug::WriteProcessMemory;
use windows_sys::Win32::System::Threading::{
    CreateRemoteThread, GetExitCodeThread, OpenProcess, WaitForSingleObject, INFINITE,
    PROCESS_ALL_ACCESS,
};

/// Finds the process ID of a process by its executable name (case-insensitive).
pub fn find_process_by_name(name: &str) -> Result<Option<u32>> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE as HANDLE {
        return Err(anyhow!("failed to create toolhelp snapshot"));
    }

    let mut entry: PROCESSENTRY32 = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32>() as u32;

    if unsafe { Process32First(snapshot, &mut entry) } == FALSE {
        unsafe { CloseHandle(snapshot) };
        return Ok(None);
    }

    let target = name.to_ascii_lowercase();
    loop {
        // Convert c_char array to a Rust string
        let exe_name = entry
            .szExeFile
            .iter()
            .take_while(|&&c| c != 0)
            .map(|&c| c as u8 as char)
            .collect::<String>();

        if exe_name.to_ascii_lowercase() == target {
            unsafe { CloseHandle(snapshot) };
            return Ok(Some(entry.th32ProcessID));
        }

        if unsafe { Process32Next(snapshot, &mut entry) } == FALSE {
            break;
        }
    }

    unsafe { CloseHandle(snapshot) };
    Ok(None)
}

/// Inject a DLL into the target process by ID using LoadLibraryW / LoadLibraryA pattern.
pub fn inject_dll(pid: u32, dll_path: &Path) -> Result<()> {
    let full_path = dll_path
        .canonicalize()
        .map_err(|e| anyhow!("failed to resolve canonical path of DLL: {e}"))?;

    let path_str = full_path.to_string_lossy();
    let c_path_str = CString::new(path_str.as_bytes())?;
    let path_bytes = c_path_str.as_bytes_with_nul();

    let process_handle = unsafe { OpenProcess(PROCESS_ALL_ACCESS, FALSE, pid) };
    if process_handle.is_null() {
        return Err(anyhow!("failed to open target process with all access"));
    }

    // Allocate memory inside target process for the DLL path string
    let remote_mem = unsafe {
        VirtualAllocEx(
            process_handle,
            ptr::null(),
            path_bytes.len(),
            MEM_COMMIT | MEM_RESERVE,
            PAGE_READWRITE,
        )
    };
    if remote_mem.is_null() {
        unsafe { CloseHandle(process_handle) };
        return Err(anyhow!("failed to allocate memory in target process"));
    }

    // Write the path bytes into target process memory
    let mut bytes_written = 0;
    let write_res = unsafe {
        WriteProcessMemory(
            process_handle,
            remote_mem,
            path_bytes.as_ptr() as *const _,
            path_bytes.len(),
            &mut bytes_written,
        )
    };

    if write_res == FALSE || bytes_written != path_bytes.len() {
        unsafe {
            VirtualFreeEx(process_handle, remote_mem, 0, MEM_RELEASE);
            CloseHandle(process_handle);
        }
        return Err(anyhow!("failed to write DLL path into target process memory"));
    }

    // Find the module handle of kernel32.dll and the address of LoadLibraryA
    let kernel32_name = CString::new("kernel32.dll")?;
    let kernel32_handle = unsafe { GetModuleHandleA(kernel32_name.as_ptr() as *const _) };
    if kernel32_handle.is_null() {
        unsafe {
            VirtualFreeEx(process_handle, remote_mem, 0, MEM_RELEASE);
            CloseHandle(process_handle);
        }
        return Err(anyhow!("failed to locate kernel32.dll module"));
    }

    let load_library_name = CString::new("LoadLibraryA")?;
    let load_library_addr = unsafe {
        windows_sys::Win32::System::LibraryLoader::GetProcAddress(
            kernel32_handle,
            load_library_name.as_ptr() as *const _,
        )
    };
    let Some(load_library_fn) = load_library_addr else {
        unsafe {
            VirtualFreeEx(process_handle, remote_mem, 0, MEM_RELEASE);
            CloseHandle(process_handle);
        }
        return Err(anyhow!("failed to resolve address of LoadLibraryA"));
    };

    // Create a remote thread that calls LoadLibraryA(remote_mem)
    let remote_thread = unsafe {
        CreateRemoteThread(
            process_handle,
            ptr::null(),
            0,
            Some(std::mem::transmute(load_library_fn)),
            remote_mem,
            0,
            ptr::null_mut(),
        )
    };

    if remote_thread.is_null() {
        unsafe {
            VirtualFreeEx(process_handle, remote_mem, 0, MEM_RELEASE);
            CloseHandle(process_handle);
        }
        return Err(anyhow!("failed to spawn remote thread in target process"));
    }

    // Wait for the remote LoadLibrary call to finish
    unsafe {
        WaitForSingleObject(remote_thread, INFINITE);
    };

    let mut exit_code = 0;
    let exit_res = unsafe { GetExitCodeThread(remote_thread, &mut exit_code) };

    unsafe {
        CloseHandle(remote_thread);
        VirtualFreeEx(process_handle, remote_mem, 0, MEM_RELEASE);
        CloseHandle(process_handle);
    }

    if exit_res == FALSE || exit_code == 0 {
        return Err(anyhow!("LoadLibraryA call in target process returned failure (0)"));
    }

    Ok(())
}

/// Helper that watches for a process name and blocks until it is found and successfully injected.
pub fn monitor_and_inject(process_name: &str, dll_path: &Path, interval: Duration) -> Result<()> {
    println!("Waiting for process: {process_name}...");
    loop {
        match find_process_by_name(process_name)? {
            Some(pid) => {
                println!("Found process {process_name} (PID: {pid}). Injecting DLL...");
                match inject_dll(pid, dll_path) {
                    Ok(_) => {
                        println!("DLL injected successfully!");
                        return Ok(());
                    }
                    Err(err) => {
                        return Err(anyhow!("Failed to inject DLL: {err}"));
                    }
                }
            }
            None => {
                thread::sleep(interval);
            }
        }
    }
}
