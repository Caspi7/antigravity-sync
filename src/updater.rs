use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const APP_VERSION: &str = "2.2.3";
pub const REPO_OWNER: &str = "Caspi7";
pub const REPO_NAME: &str = "antigravity-sync";
pub const RELEASES_PAGE_URL: &str = "https://github.com/Caspi7/antigravity-sync/releases";

const DETACHED_PROCESS: u32 = 0x0000_0008;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateInfo {
    pub version: String,
    pub installer_url: String,
    pub sha256_url: Option<String>,
    pub release_notes: String,
}

fn parse_version_parts(raw: &str) -> Option<Vec<u64>> {
    let trimmed = raw.trim().trim_start_matches(['v', 'V']);
    let core = trimmed.split(['-', '+']).next()?.trim();
    if core.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    for segment in core.split('.') {
        if segment.is_empty() {
            return None;
        }
        parts.push(segment.parse::<u64>().ok()?);
    }
    Some(parts)
}

pub fn is_newer_version(latest: &str, current: &str) -> bool {
    let (Some(latest_parts), Some(current_parts)) =
        (parse_version_parts(latest), parse_version_parts(current))
    else {
        return false;
    };

    let max_len = latest_parts.len().max(current_parts.len());
    for i in 0..max_len {
        let l = *latest_parts.get(i).unwrap_or(&0);
        let c = *current_parts.get(i).unwrap_or(&0);
        if l != c {
            return l > c;
        }
    }
    false
}

pub fn check_for_update() -> Result<Option<UpdateInfo>, String> {
    let url = format!("https://api.github.com/repos/{REPO_OWNER}/{REPO_NAME}/releases/latest");
    let user_agent = format!("AntigravitySync/{APP_VERSION}");

    let response = ureq::get(&url)
        .set("User-Agent", &user_agent)
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("Failed to check for updates: {e}"))?;

    let body = response
        .into_string()
        .map_err(|e| format!("Failed to read update response: {e}"))?;

    let json: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("Failed to parse release JSON: {e}"))?;

    let tag_name = json
        .get("tag_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Release JSON missing tag_name".to_string())?;

    let version = tag_name.trim().trim_start_matches(['v', 'V']).to_string();
    if !is_newer_version(&version, APP_VERSION) {
        return Ok(None);
    }

    let release_notes = json
        .get("body")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let assets = json
        .get("assets")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "Release JSON missing assets".to_string())?;

    let mut installer_url: Option<String> = None;
    let mut sha256_url: Option<String> = None;

    for asset in assets {
        let Some(name) = asset.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(download_url) = asset.get("browser_download_url").and_then(|v| v.as_str()) else {
            continue;
        };

        let lower = name.to_ascii_lowercase();
        if installer_url.is_none() && lower.contains("setup") && lower.ends_with(".exe") {
            installer_url = Some(download_url.to_string());
        } else if sha256_url.is_none() && name.eq_ignore_ascii_case("SHA256SUMS.txt") {
            sha256_url = Some(download_url.to_string());
        }
    }

    let Some(installer_url) = installer_url else {
        return Ok(None);
    };

    Ok(Some(UpdateInfo {
        version,
        installer_url,
        sha256_url,
        release_notes,
    }))
}

pub fn compute_sha256(path: &Path) -> Result<String, String> {
    let file =
        File::open(path).map_err(|e| format!("Failed to open file for SHA-256: {e}"))?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65_536];

    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("Failed to read file for SHA-256: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }

    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(hex, "{byte:02x}");
    }
    Ok(hex)
}

pub fn download_and_verify_update(
    info: &UpdateInfo,
    on_progress: impl Fn(f32, String),
) -> Result<PathBuf, String> {
    let safe_version: String = info
        .version
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let temp_path = std::env::temp_dir().join(format!("AntigravitySync-Setup-v{safe_version}.exe"));

    let user_agent = format!("AntigravitySync/{APP_VERSION}");
    let response = ureq::get(&info.installer_url)
        .set("User-Agent", &user_agent)
        .call()
        .map_err(|e| format!("Failed to download installer: {e}"))?;

    let total_bytes = response
        .header("Content-Length")
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&n| n > 0);

    on_progress(0.0, "0% — Downloading update...".to_string());

    {
        let file = File::create(&temp_path)
            .map_err(|e| format!("Failed to create temp installer file: {e}"))?;
        let mut writer = BufWriter::new(file);
        let mut reader = response.into_reader();
        let mut buf = [0u8; 65_536];
        let mut downloaded: u64 = 0;
        let mut last_pct: i32 = -1;

        loop {
            let n = reader
                .read(&mut buf)
                .map_err(|e| format!("Failed while downloading installer: {e}"))?;
            if n == 0 {
                break;
            }
            writer
                .write_all(&buf[..n])
                .map_err(|e| format!("Failed to write installer to disk: {e}"))?;
            downloaded += n as u64;

            if let Some(total) = total_bytes {
                let frac = (downloaded as f32 / total as f32).clamp(0.0, 1.0);
                let pct = (frac * 100.0).round() as i32;
                if pct != last_pct {
                    last_pct = pct;
                    on_progress(frac, format!("{pct}% — Downloading update..."));
                }
            } else {
                let mb = downloaded as f64 / (1024.0 * 1024.0);
                on_progress(0.0, format!("Downloading update... ({mb:.1} MB)"));
            }
        }

        writer
            .flush()
            .map_err(|e| format!("Failed to flush installer file: {e}"))?;
    }

    if let Some(sha_url) = &info.sha256_url {
        on_progress(0.95, "95% — Verifying SHA-256 checksum...".to_string());

        let sums_resp = ureq::get(sha_url)
            .set("User-Agent", &user_agent)
            .call()
            .map_err(|e| {
                let _ = fs::remove_file(&temp_path);
                format!("Failed to download SHA256SUMS.txt: {e}")
            })?;

        let sums_text = sums_resp.into_string().map_err(|e| {
            let _ = fs::remove_file(&temp_path);
            format!("Failed to read SHA256SUMS.txt: {e}")
        })?;

        let actual_hash = compute_sha256(&temp_path).map_err(|e| {
            let _ = fs::remove_file(&temp_path);
            e
        })?;

        let verified = sums_text
            .trim_start_matches('\u{feff}')
            .lines()
            .any(|line| {
                line.split_whitespace()
                    .next()
                    .is_some_and(|token| token.eq_ignore_ascii_case(&actual_hash))
            });

        if !verified {
            let _ = fs::remove_file(&temp_path);
            return Err("SHA-256 verification failed! Aborting update.".to_string());
        }
    }

    on_progress(1.0, "100% — Update ready to install.".to_string());
    Ok(temp_path)
}

pub fn launch_silent_installer(installer_path: &Path) -> Result<(), String> {
    Command::new(installer_path)
        .args(["/SP-", "/SILENT", "/NORESTART", "/UPDATE"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .spawn()
        .map_err(|e| format!("Failed to launch installer: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_newer_version() {
        assert!(is_newer_version("2.0.1", "2.0.0"));
        assert!(is_newer_version("v2.1.0", "2.0.0"));
        assert!(is_newer_version("v3.0.0", "v2.9.9"));
        assert!(is_newer_version("2.10.0", "2.2.0"));
        assert!(is_newer_version("2.1", "2.0.0"));
        assert!(is_newer_version("2.0.0.1", "2.0.0"));

        assert!(!is_newer_version("2.0.0", "2.0.0"));
        assert!(!is_newer_version("v2.0.0", "2.0.0"));
        assert!(!is_newer_version("1.9.9", "2.0.0"));
        assert!(!is_newer_version("v2.0.0-beta.1", "2.0.0"));
        assert!(!is_newer_version("", "2.0.0"));
        assert!(!is_newer_version("invalid", "2.0.0"));
    }

    #[test]
    fn test_compute_sha256_known_vectors() {
        let dir = std::env::temp_dir().join(format!(
            "antigravity_sync_sha256_test_{}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();

        let empty_path = dir.join("empty.bin");
        fs::write(&empty_path, b"").unwrap();
        assert_eq!(
            compute_sha256(&empty_path).unwrap(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );

        let abc_path = dir.join("abc.txt");
        fs::write(&abc_path, b"abc").unwrap();
        assert_eq!(
            compute_sha256(&abc_path).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );

        let missing_path = dir.join("does_not_exist.bin");
        assert!(compute_sha256(&missing_path).is_err());

        let _ = fs::remove_dir_all(&dir);
    }
}
