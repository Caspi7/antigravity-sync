use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};

#[link(name = "kernel32")]
extern "system" {
    fn CreateMutexW(
        lp_mutex_attributes: *const core::ffi::c_void,
        b_initial_owner: i32,
        lp_name: *const u16,
    ) -> HANDLE;
}

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const DETACHED_PROCESS: u32 = 0x0000_0008;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

pub struct SingleInstanceGuard {
    handle: HANDLE,
}

// Kernel mutex handles can be safely closed from another thread on drop.
unsafe impl Send for SingleInstanceGuard {}

impl SingleInstanceGuard {
    pub fn acquire() -> Option<Self> {
        let name: Vec<u16> = "Local\\AntigravitySync_SingleInstance\0"
            .encode_utf16()
            .collect();
        let handle = unsafe { CreateMutexW(std::ptr::null(), 1, name.as_ptr()) };
        if handle.is_null() {
            return None;
        }
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe {
                CloseHandle(handle);
            }
            return None;
        }
        Some(Self { handle })
    }
}

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe {
                CloseHandle(self.handle);
            }
            self.handle = std::ptr::null_mut();
        }
    }
}

pub fn is_windows_dark_mode() -> bool {
    let Ok(output) = Command::new("reg")
        .args([
            "query",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
            "/v",
            "AppsUseLightTheme",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    else {
        return false;
    };

    if !output.status.success() {
        return false;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if line.contains("AppsUseLightTheme") {
            if let Some(value) = line.split_whitespace().last() {
                return matches!(value, "0x0" | "0x00000000" | "0");
            }
        }
    }
    false
}

pub fn is_antigravity_running() -> bool {
    let Ok(output) = Command::new("tasklist")
        .args(["/FI", "IMAGENAME eq Antigravity.exe", "/NH"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    else {
        return false;
    };

    if !output.status.success() {
        return false;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.to_ascii_lowercase().contains("antigravity.exe")
}

pub fn antigravity_exe_path() -> Option<PathBuf> {
    let local_app_data = dirs::data_local_dir()
        .or_else(|| std::env::var_os("LOCALAPPDATA").map(PathBuf::from))?;
    let exe_path = local_app_data
        .join("Programs")
        .join("antigravity")
        .join("Antigravity.exe");
    if exe_path.is_file() {
        Some(exe_path)
    } else {
        None
    }
}

pub fn launch_antigravity() -> Result<(), String> {
    let exe = antigravity_exe_path()
        .ok_or_else(|| "Antigravity.exe was not found in %LOCALAPPDATA%\\Programs\\antigravity".to_string())?;
    Command::new(exe)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .spawn()
        .map_err(|e| format!("Failed to launch Antigravity.exe: {e}"))?;
    Ok(())
}

pub fn open_url(url: &str) {
    let _ = Command::new("cmd")
        .args(["/C", "start", "", url])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
}
