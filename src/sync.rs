use chrono::Local;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(150);

/// Describes one Antigravity data location eligible for sync.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncTarget {
    pub name: String,
    pub label: String,
    pub local_path: PathBuf,
    pub is_dir: bool,
    pub origin: String,
}

/// Live local vs. cloud state for a sync target shown in the sortable data grid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetStat {
    pub name: String,
    pub local_exists: bool,
    pub remote_exists: bool,
    pub local_bytes: u64,
    pub remote_bytes: u64,
    pub status: String,
}

/// Sync direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncMode {
    Push,
    Pull,
}

impl SyncMode {
    fn label(self) -> &'static str {
        match self {
            Self::Push => "Push",
            Self::Pull => "Pull",
        }
    }
}

/// Progress update emitted during `run_sync`.
///
/// `percent` is in the range `0.0..=100.0`.
#[derive(Clone, Debug, PartialEq)]
pub struct SyncProgress {
    pub percent: f32,
    pub status_text: String,
    pub log_line: Option<String>,
}

/// Returns the 7 default Antigravity sync targets for the current Windows user.
pub fn default_targets() -> Vec<SyncTarget> {
    let home = user_profile_dir();
    let app_data = roaming_app_data_dir(&home);
    targets_with_roots(&home, &app_data)
}

fn targets_with_roots(home: &Path, app_data: &Path) -> Vec<SyncTarget> {
    let gemini_ag = home.join(".gemini").join("antigravity");
    let gemini_cfg = home.join(".gemini").join("config");
    let ag_user = app_data.join("Antigravity").join("User");

    vec![
        SyncTarget {
            name: "conversations".to_string(),
            label: "Conversations".to_string(),
            local_path: gemini_ag.join("conversations"),
            is_dir: true,
            origin: r"%USERPROFILE%\.gemini\antigravity\conversations".to_string(),
        },
        SyncTarget {
            name: "brain".to_string(),
            label: "Brain Artifacts".to_string(),
            local_path: gemini_ag.join("brain"),
            is_dir: true,
            origin: r"%USERPROFILE%\.gemini\antigravity\brain".to_string(),
        },
        SyncTarget {
            name: "annotations".to_string(),
            label: "Annotations".to_string(),
            local_path: gemini_ag.join("annotations"),
            is_dir: true,
            origin: r"%USERPROFILE%\.gemini\antigravity\annotations".to_string(),
        },
        SyncTarget {
            name: "config".to_string(),
            label: "Config & Skills".to_string(),
            local_path: gemini_cfg,
            is_dir: true,
            origin: r"%USERPROFILE%\.gemini\config".to_string(),
        },
        SyncTarget {
            name: "globalStorage".to_string(),
            label: "Global Storage".to_string(),
            local_path: ag_user.join("globalStorage"),
            is_dir: true,
            origin: r"%APPDATA%\Antigravity\User\globalStorage".to_string(),
        },
        SyncTarget {
            name: "workspaceStorage".to_string(),
            label: "Workspace Storage".to_string(),
            local_path: ag_user.join("workspaceStorage"),
            is_dir: true,
            origin: r"%APPDATA%\Antigravity\User\workspaceStorage".to_string(),
        },
        SyncTarget {
            name: "agyhub_summaries".to_string(),
            label: "AgyHub Summaries".to_string(),
            local_path: gemini_ag.join("agyhub_summaries_proto.pb"),
            is_dir: false,
            origin: r"%USERPROFILE%\.gemini\antigravity\agyhub_summaries_proto.pb".to_string(),
        },
        SyncTarget {
            name: "conversation_summaries".to_string(),
            label: "Chat Index DB".to_string(),
            local_path: gemini_ag.join("conversation_summaries.db"),
            is_dir: false,
            origin: r"%USERPROFILE%\.gemini\antigravity\conversation_summaries.db".to_string(),
        },
        SyncTarget {
            name: "antigravity_state".to_string(),
            label: "Antigravity State".to_string(),
            local_path: gemini_ag.join("antigravity_state.pbtxt"),
            is_dir: false,
            origin: r"%USERPROFILE%\.gemini\antigravity\antigravity_state.pbtxt".to_string(),
        },
    ]
}

/// Scans all 7 default targets locally and under `sync_root` to compute byte counts and status.
pub fn scan_target_stats(sync_root: &Path) -> Vec<TargetStat> {
    scan_target_stats_for(&default_targets(), sync_root)
}

fn scan_target_stats_for(targets: &[SyncTarget], sync_root: &Path) -> Vec<TargetStat> {
    targets
        .iter()
        .map(|target| {
            let remote_path = remote_path_for(sync_root, target);
            let local_exists = path_exists_for_kind(&target.local_path, target.is_dir);
            let remote_exists = path_exists_for_kind(&remote_path, target.is_dir);

            let local_bytes = if local_exists {
                entry_size_bytes(&target.local_path, target.is_dir)
            } else {
                0
            };
            let remote_bytes = if remote_exists {
                entry_size_bytes(&remote_path, target.is_dir)
            } else {
                0
            };

            let status = match (local_exists, remote_exists) {
                (true, true) => "Synced",
                (true, false) => "Local Only",
                (false, true) => "Drive Only",
                (false, false) => "Missing",
            }
            .to_string();

            TargetStat {
                name: target.name.clone(),
                local_exists,
                remote_exists,
                local_bytes,
                remote_bytes,
                status,
            }
        })
        .collect()
}

/// Executes a Push or Pull sync over the selected `enabled_targets`.
///
/// Closes Antigravity first via `taskkill`, creates a timestamped safety backup under
/// `%USERPROFILE%\.gemini\antigravity_backup_YYYYMMDD_HHMMSS` before `Pull`, mirrors targets via
/// `robocopy.exe` with `CREATE_NO_WINDOW` and `Stdio::null()`, and polls `cancel` every 150ms.
pub fn run_sync(
    mode: SyncMode,
    sync_root: &Path,
    enabled_targets: &[String],
    cancel: Arc<AtomicBool>,
    on_progress: impl Fn(SyncProgress) + Send + Sync,
) -> Result<String, String> {
    let all_targets = default_targets();
    let backup_parent = user_profile_dir().join(".gemini");
    run_sync_impl(
        mode,
        sync_root,
        enabled_targets,
        &all_targets,
        &backup_parent,
        true,
        cancel,
        &on_progress,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_sync_impl(
    mode: SyncMode,
    sync_root: &Path,
    enabled_targets: &[String],
    all_targets: &[SyncTarget],
    backup_parent: &Path,
    kill_antigravity: bool,
    cancel: Arc<AtomicBool>,
    on_progress: &(impl Fn(SyncProgress) + Send + Sync),
) -> Result<String, String> {
    check_cancelled(&cancel)?;

    if sync_root.as_os_str().is_empty() {
        return Err("Sync folder path cannot be empty.".to_string());
    }

    let targets: Vec<&SyncTarget> = all_targets
        .iter()
        .filter(|t| enabled_targets.iter().any(|enabled| enabled == &t.name))
        .collect();

    if targets.is_empty() {
        return Err("No sync targets are enabled.".to_string());
    }

    if mode == SyncMode::Pull && !sync_root.is_dir() {
        return Err(format!(
            "Sync folder '{}' does not exist.",
            sync_root.display()
        ));
    }

    let overall_start = Instant::now();
    on_progress(SyncProgress {
        percent: 0.0,
        status_text: format!("0% · Starting {}...", mode.label()),
        log_line: Some(format!(
            "Starting {} with {} ({} targets)...",
            mode.label(),
            sync_root.display(),
            targets.len()
        )),
    });

    if kill_antigravity {
        on_progress(SyncProgress {
            percent: 2.0,
            status_text: "2% · Closing Antigravity...".to_string(),
            log_line: Some("Checking for running Antigravity processes...".to_string()),
        });
        let closed = close_antigravity(&cancel)?;
        if closed {
            on_progress(SyncProgress {
                percent: 4.0,
                status_text: "4% · Waiting for Antigravity to exit...".to_string(),
                log_line: Some("Closed Antigravity process; waiting for file locks...".to_string()),
            });
            sleep_with_cancel(Duration::from_millis(900), &cancel)?;
        }
    }

    check_cancelled(&cancel)?;

    if mode == SyncMode::Push {
        fs::create_dir_all(sync_root).map_err(|e| {
            format!(
                "Failed to create sync folder '{}': {e}",
                sync_root.display()
            )
        })?;
    }

    // Resolve any Google Drive conflict copies (e.g. `agyhub_summaries_proto (1).pb`) in sync root.
    let root_conflicts = crate::merge::cleanup_drive_conflicts_in_dir(sync_root);
    if root_conflicts > 0 {
        on_progress(SyncProgress {
            percent: 4.5,
            status_text: format!("Resolved {root_conflicts} Drive conflict duplicate(s)"),
            log_line: Some(format!(
                "Resolved {root_conflicts} Google Drive conflict duplicate(s) in {}",
                sync_root.display()
            )),
        });
    }

    if mode == SyncMode::Pull {
        on_progress(SyncProgress {
            percent: 5.0,
            status_text: "5% · Creating safety backup...".to_string(),
            log_line: Some("Creating safety backup before Pull...".to_string()),
        });
        let backup_dir = create_backup(&targets, backup_parent, &cancel)?;
        on_progress(SyncProgress {
            percent: 10.0,
            status_text: "10% · Safety backup ready".to_string(),
            log_line: Some(format!("Backup created at {}", backup_dir.display())),
        });
    }

    let start_pct: f32 = if mode == SyncMode::Pull { 10.0 } else { 5.0 };
    let end_pct: f32 = 95.0;

    // Use lightweight per-target weights so sync starts immediately without pre-walking thousands
    // of Google Drive files (live elapsed progress ticks keep the UI responsive during large targets).
    let weights: Vec<u64> = targets
        .iter()
        .map(|target| match target.name.as_str() {
            "brain" | "globalStorage" | "workspaceStorage" => 40,
            "conversations" | "config" => 15,
            _ => 5,
        })
        .collect();
    let total_weight: u64 = weights.iter().sum::<u64>().max(1);
    let mut done_weight: u64 = 0;
    let copy_start = Instant::now();

    let mut synced_count = 0usize;
    let mut skipped_count = 0usize;

    for (idx, target) in targets.iter().enumerate() {
        check_cancelled(&cancel)?;

        let remote = remote_path_for(sync_root, target);
        let (src, dst) = match mode {
            SyncMode::Push => (target.local_path.as_path(), remote.as_path()),
            SyncMode::Pull => (remote.as_path(), target.local_path.as_path()),
        };

        let pct = progress_percent(start_pct, end_pct, done_weight, total_weight);
        let eta_text = if idx == 0 || done_weight == 0 {
            "calculating ETA...".to_string()
        } else {
            let eta_secs = estimate_remaining_secs(copy_start.elapsed(), done_weight, total_weight);
            format!("~{} remaining", format_eta(eta_secs))
        };

        let exists = path_exists_for_kind(src, target.is_dir);
        if !exists {
            skipped_count += 1;
            done_weight += weights[idx];
            let post_pct = progress_percent(start_pct, end_pct, done_weight, total_weight);
            on_progress(SyncProgress {
                percent: post_pct,
                status_text: format!("{:.0}% · {}", post_pct, eta_text),
                log_line: Some(format!("Skipping {} (not found at {})", target.name, src.display())),
            });
            continue;
        }

        let target_start = Instant::now();
        let next_pct = progress_percent(start_pct, end_pct, done_weight + weights[idx], total_weight);
        let target_count = targets.len();
        let target_num = idx + 1;

        on_progress(SyncProgress {
            percent: pct,
            status_text: format!("{:.0}% · {} ({}/{}) · {}", pct, target.label, target_num, target_count, eta_text),
            log_line: Some(format!("[{}] Merging {} ({}/{})...", mode.label(), target.label, target_num, target_count)),
        });

        let detail = if target.name == "conversations" {
            let stats = crate::merge::merge_conversations_dir(src, dst, &cancel)?;
            format_merge_detail(&stats)
        } else if target.name == "conversation_summaries" {
            let stats = crate::merge::merge_conversation_summaries_db(src, dst, &cancel)?;
            if stats.rows_merged > 0 {
                format!("{} rows merged", stats.rows_merged)
            } else {
                format_merge_detail(&stats)
            }
        } else if target.is_dir {
            // Use fast multi-threaded robocopy (/E /XO /MT:16) for large asset/config directories:
            // /E + /XO is strictly non-destructive (copies new & newer files, never deletes existing).
            run_robocopy_with_tick(src, dst, true, &cancel, |elapsed| {
                let frac = (elapsed.as_secs_f32() / (elapsed.as_secs_f32() + 4.0)).clamp(0.0, 0.92);
                let live_pct = pct + (next_pct - pct) * frac;
                on_progress(SyncProgress {
                    percent: live_pct,
                    status_text: format!(
                        "{:.0}% · [{}] {} ({}/{}) — {:.1}s elapsed",
                        live_pct,
                        mode.label(),
                        target.label,
                        target_num,
                        target_count,
                        elapsed.as_secs_f32()
                    ),
                    log_line: None,
                });
            })?;
            "additive /MT:16 merge".to_string()
        } else {
            let stats = crate::merge::merge_single_file(src, dst, &cancel)?;
            format_merge_detail(&stats)
        };
        synced_count += 1;

        done_weight += weights[idx];
        let post_pct = progress_percent(start_pct, end_pct, done_weight, total_weight);
        let post_eta = if done_weight >= total_weight {
            "finishing...".to_string()
        } else {
            let eta_secs = estimate_remaining_secs(copy_start.elapsed(), done_weight, total_weight);
            format!("~{} remaining", format_eta(eta_secs))
        };

        on_progress(SyncProgress {
            percent: post_pct,
            status_text: format!("{:.0}% · {}", post_pct, post_eta),
            log_line: Some(format!(
                "[{}] {} merged ({}; {:.1}s)",
                mode.label(),
                target.label,
                detail,
                target_start.elapsed().as_secs_f32()
            )),
        });
    }

    if mode == SyncMode::Push {
        let _ = write_sync_manifest(sync_root, &targets);
    }

    let elapsed_secs = overall_start.elapsed().as_secs_f64();
    let summary = format!(
        "{} completed cleanly in {:.1}s ({} synced, {} skipped)",
        mode.label(),
        elapsed_secs,
        synced_count,
        skipped_count
    );

    on_progress(SyncProgress {
        percent: 100.0,
        status_text: format!("100% · {}", summary),
        log_line: Some(summary.clone()),
    });

    Ok(summary)
}

fn format_merge_detail(merge_stats: &crate::merge::MergeStats) -> String {
    let mut parts = vec![
        format!("+{} new", merge_stats.copied),
        format!("~{} updated", merge_stats.updated),
        format!("{} kept", merge_stats.kept_existing),
    ];
    if merge_stats.conflicts_resolved > 0 {
        parts.push(format!("{} duplicates resolved", merge_stats.conflicts_resolved));
    }
    if merge_stats.orphans_removed > 0 {
        parts.push(format!("{} orphan WALs cleaned", merge_stats.orphans_removed));
    }
    parts.join(", ")
}

fn run_taskkill(args: &[&str], cancel: &AtomicBool) -> Result<bool, String> {
    let mut cmd = Command::new("taskkill");
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    apply_no_window(&mut cmd);

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to launch taskkill: {e}"))?;
    let status = wait_child_with_cancel(&mut child, cancel)?;
    Ok(status.success())
}

fn wait_for_process_exit(image_name: &str, timeout: Duration, cancel: &AtomicBool) -> Result<bool, String> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        check_cancelled(cancel)?;
        if !crate::win32::is_process_running(image_name) {
            return Ok(true);
        }
        sleep_with_cancel(CANCEL_POLL_INTERVAL, cancel)?;
    }
    Ok(!crate::win32::is_process_running(image_name))
}

fn stop_process_gracefully(image_name: &str, grace: Duration, cancel: &AtomicBool) -> Result<bool, String> {
    if !crate::win32::is_process_running(image_name) {
        return Ok(false);
    }
    // Ask process to close gracefully first so SQLite can checkpoint WAL journals cleanly.
    let _ = run_taskkill(&["/IM", image_name], cancel)?;
    if !wait_for_process_exit(image_name, grace, cancel)? {
        let _ = run_taskkill(&["/F", "/T", "/IM", image_name], cancel)?;
        let _ = wait_for_process_exit(image_name, Duration::from_millis(1200), cancel)?;
    }
    Ok(true)
}

fn close_antigravity(cancel: &AtomicBool) -> Result<bool, String> {
    let closed_ui = stop_process_gracefully("Antigravity.exe", Duration::from_millis(1500), cancel)?;
    let closed_ls = if crate::win32::is_process_running("language_server.exe") {
        let _ = run_taskkill(&["/F", "/T", "/IM", "language_server.exe"], cancel)?;
        let _ = wait_for_process_exit("language_server.exe", Duration::from_millis(1000), cancel)?;
        true
    } else {
        false
    };
    Ok(closed_ui || closed_ls)
}

fn create_backup(
    targets: &[&SyncTarget],
    backup_parent: &Path,
    cancel: &AtomicBool,
) -> Result<PathBuf, String> {
    let stamp = Local::now().format("%Y%m%d_%H%M%S").to_string();
    let backup_dir = backup_parent.join(format!("antigravity_backup_{stamp}"));
    fs::create_dir_all(&backup_dir).map_err(|e| {
        format!(
            "Failed to create backup directory '{}': {e}",
            backup_dir.display()
        )
    })?;

    // Back up critical chat databases, state, and annotations (skip multi-GB brain/storage caches
    // since Pull is now non-destructive and never deletes local files).
    for target in targets {
        check_cancelled(cancel)?;
        if matches!(target.name.as_str(), "brain" | "globalStorage" | "workspaceStorage") {
            continue;
        }
        if !path_exists_for_kind(&target.local_path, target.is_dir) {
            continue;
        }
        let dst = remote_path_for(&backup_dir, target);
        run_robocopy(&target.local_path, &dst, target.is_dir, cancel)?;
    }

    Ok(backup_dir)
}

fn write_sync_manifest(sync_root: &Path, targets: &[&SyncTarget]) -> Result<(), String> {
    let machine = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "unknown".to_string());
    let stamp = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let names: Vec<&str> = targets.iter().map(|t| t.name.as_str()).collect();
    let manifest = serde_json::json!({
        "app_version": crate::updater::APP_VERSION,
        "last_push": stamp,
        "machine": machine,
        "targets": names,
        "excluded_caches": ["cache", "Cache", "Code Cache", "GPUCache", "CachedData", "Crashpad", "logs", ".system_generated", "browser_sandbox", "temp", "tmp", "__pycache__", "node_modules"]
    });
    let json = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    fs::write(sync_root.join("sync_manifest.json"), format!("{json}\n")).map_err(|e| e.to_string())
}

fn run_robocopy(
    src: &Path,
    dst: &Path,
    is_dir: bool,
    cancel: &AtomicBool,
) -> Result<(), String> {
    run_robocopy_with_tick(src, dst, is_dir, cancel, |_| {})
}

fn run_robocopy_with_tick(
    src: &Path,
    dst: &Path,
    is_dir: bool,
    cancel: &AtomicBool,
    on_tick: impl Fn(Duration),
) -> Result<(), String> {
    check_cancelled(cancel)?;

    let mut cmd = Command::new("robocopy.exe");

    if is_dir {
        fs::create_dir_all(dst).map_err(|e| {
            format!(
                "Failed to create destination folder '{}': {e}",
                dst.display()
            )
        })?;
            cmd.arg(clean_robocopy_dir(src))
                .arg(clean_robocopy_dir(dst))
                .args([
                    "/E", "/XO", "/FFT", "/MT:16", "/R:1", "/W:1", "/NFL", "/NDL", "/NP", "/NJH", "/NJS",
                    "/XD", "cache", "Cache", "Code Cache", "GPUCache", "CachedData", "Crashpad",
                    "logs", ".system_generated", "browser_sandbox", "temp", "tmp", "__pycache__", "node_modules",
                    "/XF", "*.log", "*.tmp", "*.bak", "* (*).*",
                ]);
    } else {
        let src_dir = src.parent().ok_or_else(|| {
            format!("Source file '{}' has no parent directory.", src.display())
        })?;
        let file_name = src.file_name().ok_or_else(|| {
            format!("Source path '{}' has no file name.", src.display())
        })?;
        let dst_dir = dst.parent().ok_or_else(|| {
            format!("Destination file '{}' has no parent directory.", dst.display())
        })?;
        fs::create_dir_all(dst_dir).map_err(|e| {
            format!(
                "Failed to create destination folder '{}': {e}",
                dst_dir.display()
            )
        })?;
        cmd.arg(clean_robocopy_dir(src_dir))
            .arg(clean_robocopy_dir(dst_dir))
            .arg(file_name)
            .args(["/IS", "/IT", "/R:1", "/W:1", "/NFL", "/NDL", "/NP", "/NJH", "/NJS"]);
    }

    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    apply_no_window(&mut cmd);

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to start robocopy.exe: {e}"))?;
    let status = wait_child_with_cancel_and_tick(&mut child, cancel, on_tick)?;

    match status.code() {
        // Robocopy exit codes 0..=7 indicate success (copied, extra, mismatched, or unchanged).
        Some(code) if (0..8).contains(&code) => Ok(()),
        Some(code) => Err(format!(
            "robocopy.exe failed with exit code {code} copying '{}' -> '{}'",
            src.display(),
            dst.display()
        )),
        None => Err("robocopy.exe terminated unexpectedly.".to_string()),
    }
}

fn wait_child_with_cancel(child: &mut Child, cancel: &AtomicBool) -> Result<ExitStatus, String> {
    wait_child_with_cancel_and_tick(child, cancel, |_| {})
}

fn wait_child_with_cancel_and_tick(
    child: &mut Child,
    cancel: &AtomicBool,
    on_tick: impl Fn(Duration),
) -> Result<ExitStatus, String> {
    let start = Instant::now();
    let mut last_tick = Instant::now();
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Sync cancelled by user.".to_string());
        }

        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {
                if last_tick.elapsed() >= Duration::from_millis(300) {
                    on_tick(start.elapsed());
                    last_tick = Instant::now();
                }
                std::thread::sleep(CANCEL_POLL_INTERVAL);
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("Failed waiting for child process: {e}"));
            }
        }
    }
}

fn sleep_with_cancel(duration: Duration, cancel: &AtomicBool) -> Result<(), String> {
    let deadline = Instant::now() + duration;
    loop {
        check_cancelled(cancel)?;
        let now = Instant::now();
        if now >= deadline {
            return Ok(());
        }
        std::thread::sleep((deadline - now).min(CANCEL_POLL_INTERVAL));
    }
}

fn check_cancelled(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        Err("Sync cancelled by user.".to_string())
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn apply_no_window(cmd: &mut Command) {
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn apply_no_window(_cmd: &mut Command) {}

/// Strips trailing separators from non-root paths so Windows CRT quoting never interprets `\"`
/// as an escaped quote in `robocopy.exe` arguments.
fn clean_robocopy_dir(path: &Path) -> OsString {
    let raw = path.as_os_str().to_string_lossy();
    let trimmed = raw.trim_end_matches(['\\', '/']);
    // Handle drive roots such as "C:" -> "C:\."
    if trimmed.len() == 2 && trimmed.as_bytes()[1] == b':' {
        return OsString::from(format!("{trimmed}\\."));
    }
    if trimmed.is_empty() {
        return path.as_os_str().to_os_string();
    }
    OsString::from(trimmed)
}

fn remote_path_for(sync_root: &Path, target: &SyncTarget) -> PathBuf {
    if target.is_dir {
        sync_root.join(&target.name)
    } else if let Some(file_name) = target.local_path.file_name() {
        sync_root.join(file_name)
    } else {
        sync_root.join(&target.name)
    }
}

fn path_exists_for_kind(path: &Path, is_dir: bool) -> bool {
    if is_dir {
        path.is_dir()
    } else {
        path.is_file()
    }
}

fn entry_size_bytes(path: &Path, is_dir: bool) -> u64 {
    if !is_dir {
        return fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    }

    let mut total: u64 = 0;
    let mut stack = vec![path.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(_) => continue,
        };
        for entry_res in entries {
            let entry = match entry_res {
                Ok(e) => e,
                Err(_) => continue,
            };
            let ft = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => continue,
            };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                stack.push(entry.path());
            } else if ft.is_file() {
                total = total.saturating_add(entry.metadata().map(|m| m.len()).unwrap_or(0));
            }
        }
    }

    total
}

fn progress_percent(start_pct: f32, end_pct: f32, done_weight: u64, total_weight: u64) -> f32 {
    let ratio = (done_weight as f32 / total_weight.max(1) as f32).clamp(0.0, 1.0);
    start_pct + ratio * (end_pct - start_pct)
}

fn estimate_remaining_secs(elapsed: Duration, done_weight: u64, total_weight: u64) -> f64 {
    if done_weight == 0 || total_weight <= done_weight {
        return 0.0;
    }
    let elapsed_secs = elapsed.as_secs_f64();
    (elapsed_secs / done_weight as f64) * (total_weight - done_weight) as f64
}

fn format_eta(seconds: f64) -> String {
    if seconds < 1.0 {
        return "<1s".to_string();
    }
    if seconds < 60.0 {
        return format!("{}s", seconds.ceil() as u64);
    }
    let total = seconds.ceil() as u64;
    format!("{}m {}s", total / 60, total % 60)
}

fn user_profile_dir() -> PathBuf {
    dirs::home_dir()
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn roaming_app_data_dir(home: &Path) -> PathBuf {
    dirs::config_dir()
        .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
        .unwrap_or_else(|| home.join("AppData").join("Roaming"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock after epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "antigravity_sync_test_{}_{}_{}",
            label,
            std::process::id(),
            nanos
        ));
        fs::create_dir_all(&dir).expect("create temp test dir");
        dir
    }

    #[test]
    fn default_targets_contains_all_nine_targets() {
        let targets = default_targets();
        assert_eq!(targets.len(), 9);
        let names: Vec<&str> = targets.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            names,
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
        assert!(targets[..6].iter().all(|t| t.is_dir));
        assert!(targets[6..].iter().all(|t| !t.is_dir));
        assert!(targets[6]
            .local_path
            .ends_with("agyhub_summaries_proto.pb"));
        assert!(targets[7]
            .local_path
            .ends_with("conversation_summaries.db"));
        assert!(targets.iter().all(|t| !t.label.is_empty() && !t.origin.is_empty()));
    }

    #[test]
    fn scan_target_stats_classifies_all_four_statuses_and_sizes() {
        let root = unique_temp_dir("stats");
        let home = root.join("home");
        let app_data = root.join("appdata");
        let sync_root = root.join("drive");
        fs::create_dir_all(&sync_root).expect("create drive dir");

        let targets = targets_with_roots(&home, &app_data);

        // 1. conversations -> Synced (local 5 bytes, remote 9 bytes)
        fs::create_dir_all(&targets[0].local_path).unwrap();
        fs::write(targets[0].local_path.join("c1.txt"), b"hello").unwrap();
        let remote_conv = remote_path_for(&sync_root, &targets[0]);
        fs::create_dir_all(&remote_conv).unwrap();
        fs::write(remote_conv.join("c1.txt"), b"hello1234").unwrap();

        // 2. brain -> Local Only (local 4 bytes)
        fs::create_dir_all(&targets[1].local_path).unwrap();
        fs::write(targets[1].local_path.join("b.md"), b"abcd").unwrap();

        // 3. annotations -> Drive Only (remote 3 bytes)
        let remote_ann = remote_path_for(&sync_root, &targets[2]);
        fs::create_dir_all(&remote_ann).unwrap();
        fs::write(remote_ann.join("a.json"), b"xyz").unwrap();

        // 4..6 -> Missing

        // 7. agyhub_summaries (file) -> Synced (local 6 bytes, remote 6 bytes)
        fs::create_dir_all(targets[6].local_path.parent().unwrap()).unwrap();
        fs::write(&targets[6].local_path, b"123456").unwrap();
        fs::write(remote_path_for(&sync_root, &targets[6]), b"123456").unwrap();

        let stats = scan_target_stats_for(&targets, &sync_root);
        assert_eq!(stats.len(), 9);

        assert_eq!(stats[0].name, "conversations");
        assert!(stats[0].local_exists && stats[0].remote_exists);
        assert_eq!(stats[0].local_bytes, 5);
        assert_eq!(stats[0].remote_bytes, 9);
        assert_eq!(stats[0].status, "Synced");

        assert_eq!(stats[1].name, "brain");
        assert!(stats[1].local_exists && !stats[1].remote_exists);
        assert_eq!(stats[1].local_bytes, 4);
        assert_eq!(stats[1].remote_bytes, 0);
        assert_eq!(stats[1].status, "Local Only");

        assert_eq!(stats[2].name, "annotations");
        assert!(!stats[2].local_exists && stats[2].remote_exists);
        assert_eq!(stats[2].local_bytes, 0);
        assert_eq!(stats[2].remote_bytes, 3);
        assert_eq!(stats[2].status, "Drive Only");

        assert_eq!(stats[3].status, "Missing");
        assert_eq!(stats[6].status, "Synced");
        assert_eq!(stats[6].local_bytes, 6);
        assert_eq!(stats[6].remote_bytes, 6);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn push_and_pull_with_backup_via_robocopy() {
        let root = unique_temp_dir("push_pull");
        let home = root.join("home");
        let app_data = root.join("appdata");
        let sync_root = root.join("drive");
        let backup_parent = home.join(".gemini");
        let targets = targets_with_roots(&home, &app_data);

        // Seed local conversations (dir) and agyhub_summaries (file)
        fs::create_dir_all(&targets[0].local_path).unwrap();
        fs::write(targets[0].local_path.join("conv1.pb"), b"local_conv").unwrap();
        fs::create_dir_all(targets[6].local_path.parent().unwrap()).unwrap();
        fs::write(&targets[6].local_path, b"summary_v1").unwrap();

        let enabled = vec!["conversations".to_string(), "agyhub_summaries".to_string()];
        let cancel = Arc::new(AtomicBool::new(false));
        let updates = Arc::new(Mutex::new(Vec::<SyncProgress>::new()));
        let updates_cb = Arc::clone(&updates);

        let push_res = run_sync_impl(
            SyncMode::Push,
            &sync_root,
            &enabled,
            &targets,
            &backup_parent,
            false,
            Arc::clone(&cancel),
            &move |p| {
                updates_cb.lock().unwrap().push(p);
            },
        );
        assert!(push_res.is_ok(), "push failed: {push_res:?}");

        assert_eq!(
            fs::read(sync_root.join("conversations").join("conv1.pb")).unwrap(),
            b"local_conv"
        );
        assert_eq!(
            fs::read(sync_root.join("agyhub_summaries_proto.pb")).unwrap(),
            b"summary_v1"
        );
        assert_eq!(updates.lock().unwrap().last().unwrap().percent, 100.0);

        // Modify remote and overwrite local on Pull; verify safety backup preserves old local.
        fs::write(sync_root.join("conversations").join("conv1.pb"), b"remote_conv_v2").unwrap();
        fs::write(sync_root.join("agyhub_summaries_proto.pb"), b"summary_v2").unwrap();

        let pull_res = run_sync_impl(
            SyncMode::Pull,
            &sync_root,
            &enabled,
            &targets,
            &backup_parent,
            false,
            cancel,
            &|_| {},
        );
        assert!(pull_res.is_ok(), "pull failed: {pull_res:?}");

        assert_eq!(
            fs::read(targets[0].local_path.join("conv1.pb")).unwrap(),
            b"remote_conv_v2"
        );
        assert_eq!(fs::read(&targets[6].local_path).unwrap(), b"summary_v2");

        let backups: Vec<PathBuf> = fs::read_dir(&backup_parent)
            .unwrap()
            .filter_map(|e| e.ok().map(|x| x.path()))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("antigravity_backup_"))
            })
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(
            fs::read(backups[0].join("conversations").join("conv1.pb")).unwrap(),
            b"local_conv"
        );
        assert_eq!(
            fs::read(backups[0].join("agyhub_summaries_proto.pb")).unwrap(),
            b"summary_v1"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn cancel_flag_aborts_sync_and_kills_child_process() {
        let cancel = Arc::new(AtomicBool::new(true));
        let root = unique_temp_dir("cancel");
        let targets = targets_with_roots(&root.join("h"), &root.join("a"));
        let enabled = vec!["conversations".to_string()];

        let res = run_sync_impl(
            SyncMode::Push,
            &root.join("drive"),
            &enabled,
            &targets,
            &root,
            false,
            Arc::clone(&cancel),
            &|_| {},
        );
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("cancelled"));

        // Also test active child process cancellation via wait_child_with_cancel.
        let cancel_async = Arc::new(AtomicBool::new(false));
        let cancel_trigger = Arc::clone(&cancel_async);
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "ping -n 10 127.0.0.1 > nul"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        apply_no_window(&mut cmd);
        let mut child = cmd.spawn().expect("spawn sleep command");

        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            cancel_trigger.store(true, Ordering::Relaxed);
        });

        let wait_res = wait_child_with_cancel(&mut child, &cancel_async);
        assert!(wait_res.is_err());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn eta_and_path_cleaning_helpers_work() {
        assert_eq!(format_eta(0.4), "<1s");
        assert_eq!(format_eta(7.2), "8s");
        assert_eq!(format_eta(65.0), "1m 5s");
        assert_eq!(
            clean_robocopy_dir(Path::new(r"G:\My Drive\AntigravitySync\")),
            OsString::from(r"G:\My Drive\AntigravitySync")
        );
        assert_eq!(
            clean_robocopy_dir(Path::new(r"G:\")),
            OsString::from(r"G:\.")
        );
    }
}
