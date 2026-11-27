use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// All 9 default Antigravity sync target names, in display/execution order.
pub const DEFAULT_TARGET_NAMES: [&str; 9] = [
    "conversations",
    "brain",
    "annotations",
    "config",
    "globalStorage",
    "workspaceStorage",
    "agyhub_summaries",
    "conversation_summaries",
    "antigravity_state",
];

/// Supported UI themes.
pub const VALID_THEMES: [&str; 4] = ["System", "Light", "Dark", "Dark Blue"];

const DEFAULT_SYNC_ROOT: &str = r"G:\My Drive\AntigravitySync";
const DEFAULT_THEME: &str = "System";

/// Current settings schema version for one-time migrations (e.g. adding new default targets).
pub const SETTINGS_SCHEMA_VERSION: u32 = 2;

/// Persistent user configuration stored in `%APPDATA%\AntigravitySync\settings.json`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AppSettings {
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default = "default_sync_root", alias = "SyncRoot")]
    pub sync_root: String,
    #[serde(default = "default_theme", alias = "Theme")]
    pub theme: String,
    #[serde(default = "default_true", alias = "AutoRelaunch")]
    pub auto_relaunch: bool,
    #[serde(default = "default_true", alias = "AutoCheckUpdates")]
    pub auto_check_updates: bool,
    #[serde(default = "default_enabled_targets", alias = "EnabledTargets")]
    pub enabled_targets: Vec<String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            sync_root: default_sync_root(),
            theme: default_theme(),
            auto_relaunch: true,
            auto_check_updates: true,
            enabled_targets: default_enabled_targets(),
        }
    }
}

impl AppSettings {
    /// Loads settings from `%APPDATA%\AntigravitySync\settings.json`, falling back to defaults on
    /// missing or malformed files.
    pub fn load() -> Self {
        match settings_path() {
            Some(path) => Self::load_from_path(&path),
            None => Self::default(),
        }
    }

    /// Saves settings as pretty-printed JSON to `%APPDATA%\AntigravitySync\settings.json`.
    pub fn save(&self) -> Result<(), String> {
        let path = settings_path()
            .ok_or_else(|| "Unable to locate %APPDATA% directory for settings.".to_string())?;
        self.save_to_path(&path)
    }

    fn load_from_path(path: &Path) -> Self {
        let raw = match fs::read_to_string(path) {
            Ok(content) => content,
            Err(_) => return Self::default(),
        };

        let mut settings: Self = match serde_json::from_str(&raw) {
            Ok(parsed) => parsed,
            Err(_) => return Self::default(),
        };

        settings.theme = normalize_theme(&settings.theme);

        // One-time migration for upgrades from v2.0.0/v2.0.1 (which only had 7 targets):
        // auto-enable Chat Index DB & Antigravity State so pulled chats show up in the sidebar.
        if settings.schema_version < SETTINGS_SCHEMA_VERSION {
            for required in ["conversation_summaries", "antigravity_state"] {
                if !settings
                    .enabled_targets
                    .iter()
                    .any(|t| t.eq_ignore_ascii_case(required))
                {
                    settings.enabled_targets.push(required.to_string());
                }
            }
            settings.schema_version = SETTINGS_SCHEMA_VERSION;
        }

        settings
    }

    fn save_to_path(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create settings directory '{}': {e}", parent.display()))?;
        }

        let mut normalized = self.clone();
        normalized.theme = normalize_theme(&normalized.theme);

        let json = serde_json::to_string_pretty(&normalized)
            .map_err(|e| format!("Failed to serialize settings: {e}"))?;

        let tmp_path = path.with_extension("json.tmp");
        fs::write(&tmp_path, format!("{json}\n"))
            .map_err(|e| format!("Failed to write temporary settings file '{}': {e}", tmp_path.display()))?;

        if let Err(rename_err) = fs::rename(&tmp_path, path) {
            // Fall back to direct write if rename across an existing locked file fails, cleaning up tmp.
            let write_res = fs::write(path, format!("{json}\n"));
            let _ = fs::remove_file(&tmp_path);
            write_res.map_err(|write_err| {
                format!(
                    "Failed to save settings to '{}' (rename: {rename_err}; direct write: {write_err})",
                    path.display()
                )
            })?;
        }

        Ok(())
    }
}

fn settings_path() -> Option<PathBuf> {
    dirs::config_dir()
        .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
        .map(|dir| dir.join("AntigravitySync").join("settings.json"))
}

fn default_sync_root() -> String {
    DEFAULT_SYNC_ROOT.to_string()
}

fn default_theme() -> String {
    DEFAULT_THEME.to_string()
}

fn default_true() -> bool {
    true
}

fn default_enabled_targets() -> Vec<String> {
    DEFAULT_TARGET_NAMES.iter().map(|s| (*s).to_string()).collect()
}

fn normalize_theme(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.eq_ignore_ascii_case("darkblue") {
        return "Dark Blue".to_string();
    }
    for valid in VALID_THEMES {
        if trimmed.eq_ignore_ascii_case(valid) {
            return valid.to_string();
        }
    }
    DEFAULT_THEME.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock after epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "antigravity_sync_settings_test_{}_{}_{}",
            label,
            std::process::id(),
            nanos
        ));
        fs::create_dir_all(&dir).expect("create temp test dir");
        dir
    }

    #[test]
    fn default_values_match_spec() {
        let s = AppSettings::default();
        assert_eq!(s.sync_root, r"G:\My Drive\AntigravitySync");
        assert_eq!(s.theme, "System");
        assert!(s.auto_relaunch);
        assert!(s.auto_check_updates);
        assert_eq!(s.enabled_targets.len(), 9);
        assert_eq!(
            s.enabled_targets,
            vec![
                "conversations",
                "brain",
                "annotations",
                "config",
                "globalStorage",
                "workspaceStorage",
                "agyhub_summaries",
                "conversation_summaries",
                "antigravity_state",
            ]
        );
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = unique_temp_dir("roundtrip");
        let path = dir.join("AntigravitySync").join("settings.json");

        let custom = AppSettings {
            schema_version: SETTINGS_SCHEMA_VERSION,
            sync_root: r"D:\Cloud\AntigravitySync".to_string(),
            theme: "Dark Blue".to_string(),
            auto_relaunch: false,
            auto_check_updates: false,
            enabled_targets: vec!["conversations".to_string(), "brain".to_string()],
        };

        custom.save_to_path(&path).expect("save should succeed");
        assert!(path.is_file());

        let loaded = AppSettings::load_from_path(&path);
        assert_eq!(loaded, custom);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_missing_or_corrupt_returns_defaults() {
        let dir = unique_temp_dir("corrupt");
        let missing = dir.join("does_not_exist.json");
        assert_eq!(AppSettings::load_from_path(&missing), AppSettings::default());

        let corrupt = dir.join("corrupt.json");
        fs::write(&corrupt, "{not valid json!!!").expect("write corrupt json");
        assert_eq!(AppSettings::load_from_path(&corrupt), AppSettings::default());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_normalizes_theme_and_supports_legacy_keys() {
        let dir = unique_temp_dir("legacy");
        let legacy_path = dir.join("legacy.json");
        let legacy_json = r#"{
  "SyncRoot": "E:\\Drive\\Antigravity",
  "Theme": "dark blue",
  "AutoRelaunch": false,
  "AutoCheckUpdates": true
}"#;
        fs::write(&legacy_path, legacy_json).expect("write legacy json");

        let loaded = AppSettings::load_from_path(&legacy_path);
        assert_eq!(loaded.sync_root, r"E:\Drive\Antigravity");
        assert_eq!(loaded.theme, "Dark Blue");
        assert!(!loaded.auto_relaunch);
        assert!(loaded.auto_check_updates);
        assert_eq!(loaded.enabled_targets.len(), 9);

        let invalid_theme_path = dir.join("invalid_theme.json");
        fs::write(&invalid_theme_path, r#"{"theme": "NeonGreen"}"#).expect("write invalid theme");
        let loaded_invalid = AppSettings::load_from_path(&invalid_theme_path);
        assert_eq!(loaded_invalid.theme, "System");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn settings_path_points_to_appdata_antigravity_sync() {
        let path = settings_path().expect("dirs::config_dir should exist on Windows");
        assert_eq!(
            path.file_name().and_then(|n| n.to_str()),
            Some("settings.json")
        );
        assert_eq!(
            path.parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str()),
            Some("AntigravitySync")
        );
    }

    #[test]
    fn legacy_seven_target_settings_auto_enables_chat_index_db() {
        let dir = unique_temp_dir("migrate");
        let path = dir.join("settings.json");
        let old_json = r#"{
  "sync_root": "G:\\My Drive\\AntigravitySync",
  "theme": "Dark",
  "auto_relaunch": true,
  "auto_check_updates": true,
  "enabled_targets": ["conversations", "brain", "annotations", "config", "globalStorage", "workspaceStorage", "agyhub_summaries"]
}"#;
        fs::write(&path, old_json).expect("write legacy 7-target settings");

        let loaded = AppSettings::load_from_path(&path);
        assert_eq!(loaded.schema_version, SETTINGS_SCHEMA_VERSION);
        assert!(loaded.enabled_targets.iter().any(|t| t == "conversation_summaries"));
        assert!(loaded.enabled_targets.iter().any(|t| t == "antigravity_state"));

        let _ = fs::remove_dir_all(&dir);
    }
}
