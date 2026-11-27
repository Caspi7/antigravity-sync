use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
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

pub fn is_process_running(image_name: &str) -> bool {
    let filter = format!("IMAGENAME eq {image_name}");
    let Ok(output) = Command::new("tasklist")
        .args(["/FI", &filter, "/NH"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    else {
        return false;
    };

    if !output.status.success() {
        return false;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .to_ascii_lowercase()
        .contains(&image_name.to_ascii_lowercase())
}

pub fn is_antigravity_running() -> bool {
    is_process_running("Antigravity.exe")
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

const PLUGIN_JSON: &str = include_str!("../plugin/plugin.json");
const PLUGIN_PACKAGE_JSON: &str = include_str!("../plugin/package.json");
const PLUGIN_SERVER_MJS: &str = include_str!("../plugin/server.mjs");

fn find_node_exe(home: &Path) -> Option<PathBuf> {
    // 1. Antigravity's bundled node.exe
    let node_root = home.join(".gemini").join("antigravity").join("bin").join("node");
    if let Ok(entries) = std::fs::read_dir(&node_root) {
        for entry in entries.flatten() {
            let candidate = entry.path().join("node.exe");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    // 2. Standard Windows / user Node.js install locations
    let mut candidates = Vec::new();
    for env_key in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(base) = std::env::var_os(env_key) {
            candidates.push(PathBuf::from(base).join("nodejs").join("node.exe"));
        }
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(PathBuf::from(local).join("Programs").join("nodejs").join("node.exe"));
    }
    if let Some(nvm_sym) = std::env::var_os("NVM_SYMLINK") {
        candidates.push(PathBuf::from(nvm_sym).join("node.exe"));
    }
    candidates.push(home.join("scoop").join("shims").join("node.exe"));

    if let Some(found) = candidates.into_iter().find(|p| p.is_file()) {
        return Some(found);
    }

    // 3. Fallback: search PATH via where.exe
    let mut cmd = Command::new("where.exe");
    cmd.arg("node")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW);
    if let Ok(out) = cmd.output() {
        if out.status.success() {
            let stdout = String::from_utf8_lossy(&out.stdout);
            for line in stdout.lines() {
                let p = PathBuf::from(line.trim());
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }

    None
}

fn find_npm_cli(node_exe: &Path) -> Option<PathBuf> {
    if let Some(node_dir) = node_exe.parent() {
        let direct = node_dir.join("node_modules").join("npm").join("bin").join("npm-cli.js");
        if direct.is_file() {
            return Some(direct);
        }
        let parent_nm = node_dir
            .parent()
            .map(|p| p.join("node_modules").join("npm").join("bin").join("npm-cli.js"));
        if let Some(p) = parent_nm.filter(|p| p.is_file()) {
            return Some(p);
        }
    }
    None
}

fn find_npm_cmd(node_exe: &Path) -> Option<PathBuf> {
    if let Some(node_dir) = node_exe.parent() {
        let sibling = node_dir.join("npm.cmd");
        if sibling.is_file() {
            return Some(sibling);
        }
    }
    let mut cmd = Command::new("where.exe");
    cmd.arg("npm.cmd")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW);
    if let Ok(out) = cmd.output() {
        if out.status.success() {
            let stdout = String::from_utf8_lossy(&out.stdout);
            for line in stdout.lines() {
                let p = PathBuf::from(line.trim());
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    None
}

pub fn is_plugin_installed() -> bool {
    let Some(home) = dirs::home_dir().or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from)) else {
        return false;
    };
    let plugin_dir = home.join(".gemini").join("config").join("plugins").join("antigravity-sync");
    let mcp_path = plugin_dir.join("mcp_config.json");
    if !(plugin_dir.join("plugin.json").is_file()
        && plugin_dir.join("server.mjs").is_file()
        && mcp_path.is_file()
        && plugin_dir.join("node_modules").join("@modelcontextprotocol").join("sdk").is_dir())
    {
        return false;
    }

    // Verify that the node command in mcp_config.json actually exists on this machine
    let Ok(raw) = std::fs::read_to_string(&mcp_path) else {
        return false;
    };
    let Ok(cfg) = serde_json::from_str::<serde_json::Value>(raw.trim_start_matches('\u{feff}')) else {
        return false;
    };
    let Some(cmd) = cfg
        .get("mcpServers")
        .and_then(|s| s.get("antigravity-sync"))
        .and_then(|s| s.get("command"))
        .and_then(|c| c.as_str())
    else {
        return false;
    };

    if cmd.contains('\\') || cmd.contains('/') {
        Path::new(cmd).is_file()
    } else {
        find_node_exe(&home).is_some()
    }
}

pub fn install_or_repair_plugin() -> Result<String, String> {
    let home = dirs::home_dir()
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .ok_or_else(|| "Could not locate %USERPROFILE%.".to_string())?;
    let config_dir = home.join(".gemini").join("config");
    let plugin_dir = config_dir.join("plugins").join("antigravity-sync");
    std::fs::create_dir_all(&plugin_dir)
        .map_err(|e| format!("Failed to create plugin folder: {e}"))?;

    std::fs::write(plugin_dir.join("plugin.json"), PLUGIN_JSON)
        .map_err(|e| format!("Failed to write plugin.json: {e}"))?;
    std::fs::write(plugin_dir.join("package.json"), PLUGIN_PACKAGE_JSON)
        .map_err(|e| format!("Failed to write package.json: {e}"))?;
    std::fs::write(plugin_dir.join("server.mjs"), PLUGIN_SERVER_MJS)
        .map_err(|e| format!("Failed to write server.mjs: {e}"))?;

    let node_exe = find_node_exe(&home).ok_or_else(|| {
        "Node.js (node.exe) was not found in .gemini\\antigravity\\bin\\node, Program Files, or PATH. Install Node.js LTS or launch Antigravity once first.".to_string()
    })?;

    let mcp_json = serde_json::json!({
        "mcpServers": {
            "antigravity-sync": {
                "command": node_exe.display().to_string(),
                "args": [plugin_dir.join("server.mjs").display().to_string()]
            }
        }
    });
    std::fs::write(
        plugin_dir.join("mcp_config.json"),
        serde_json::to_string_pretty(&mcp_json).unwrap() + "\n",
    )
    .map_err(|e| format!("Failed to write mcp_config.json: {e}"))?;

    let sdk_dir = plugin_dir.join("node_modules").join("@modelcontextprotocol").join("sdk");
    if !sdk_dir.is_dir() {
        let mut cmd = if let Some(npm_cli) = find_npm_cli(&node_exe) {
            let mut c = Command::new(&node_exe);
            c.arg(npm_cli);
            c
        } else if let Some(npm_cmd) = find_npm_cmd(&node_exe) {
            let mut c = Command::new("cmd.exe");
            c.arg("/C").arg(npm_cmd);
            c
        } else {
            return Err(format!(
                "Found node.exe at '{}' but could not locate npm-cli.js or npm.cmd to install @modelcontextprotocol/sdk.",
                node_exe.display()
            ));
        };

        cmd.args(["install", "--omit=dev", "--no-audit", "--no-fund"])
            .current_dir(&plugin_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .creation_flags(CREATE_NO_WINDOW);

        if let Some(node_dir) = node_exe.parent() {
            let cur_path = std::env::var_os("PATH").unwrap_or_default();
            let mut new_path = std::ffi::OsString::from(node_dir.as_os_str());
            new_path.push(";");
            new_path.push(cur_path);
            cmd.env("PATH", new_path);
        }

        let out = cmd
            .output()
            .map_err(|e| format!("Failed to run npm install: {e}"))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(format!("npm install failed: {}", err.trim()));
        }
    }

    // Ensure antigravity-sync is enabled in ~/.gemini/config/config.json
    let cfg_path = config_dir.join("config.json");
    let mut cfg: serde_json::Value = std::fs::read_to_string(&cfg_path)
        .ok()
        .and_then(|raw| serde_json::from_str(raw.trim_start_matches('\u{feff}')).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if let Some(obj) = cfg.as_object_mut() {
        let plugins = obj
            .entry("plugins")
            .or_insert_with(|| serde_json::json!({}));
        if let Some(p_obj) = plugins.as_object_mut() {
            p_obj.insert("antigravity-sync".to_string(), serde_json::json!({ "enabled": true }));
        }
        let _ = std::fs::write(
            &cfg_path,
            serde_json::to_string_pretty(&cfg).unwrap() + "\n",
        );
    }

    Ok(format!(
        "Antigravity Sync plugin ready (using {}). Restart Antigravity to load it.",
        node_exe.display()
    ))
}
