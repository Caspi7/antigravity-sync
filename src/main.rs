#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod merge;
mod settings;
mod sync;
mod updater;
mod win32;

use chrono::Local;
use settings::{AppSettings, VALID_THEMES};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use sync::{default_targets, run_sync, scan_target_stats, SyncMode, TargetStat};
use updater::{
    check_for_update, download_and_verify_update, launch_silent_installer, UpdateInfo,
    APP_VERSION, RELEASES_PAGE_URL,
};
use win32::{
    is_antigravity_running, is_windows_dark_mode, launch_antigravity, open_url, SingleInstanceGuard,
};

slint::include_modules!();

const DEFAULT_RELEASE_NOTES: &str = "\
Antigravity Sync v2.1.3 — Fast Non-Destructive Sync & Clean Process Detection\n\n\
• Eliminated multi-minute Push/Pull hangs: uses multi-threaded additive robocopy (/E /XO /MT:16) for large asset/storage folders, skips unchanged conversation DBs without re-running SQLite quick_check over Google Drive, removes the pre-sync Drive file-walk stall, and skips multi-GB cache folders in pre-Pull safety backups.\n\
• Fixed false 'Antigravity is running' status when Antigravity.exe is closed (lingering language_server.exe is still stopped cleanly during sync without blocking the status badge).";

struct AppState {
    settings: AppSettings,
    stats: Vec<TargetStat>,
    sort_col: i32,
    sort_asc: bool,
    pending_update: Option<UpdateInfo>,
    cancel_flag: Option<Arc<AtomicBool>>,
    single_instance: Option<SingleInstanceGuard>,
}

fn theme_str_to_mode(theme: &str) -> ThemeMode {
    match theme {
        "Light" => ThemeMode::Light,
        "Dark" => ThemeMode::Dark,
        "Dark Blue" => ThemeMode::DarkBlue,
        _ => ThemeMode::System,
    }
}

fn next_theme(current: &str) -> String {
    let idx = VALID_THEMES
        .iter()
        .position(|&t| t.eq_ignore_ascii_case(current))
        .unwrap_or(0);
    VALID_THEMES[(idx + 1) % VALID_THEMES.len()].to_string()
}

fn format_bytes(bytes: u64, exists: bool) -> String {
    if !exists {
        return "—".to_string();
    }
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.2} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

fn status_to_tone(status: &str) -> &'static str {
    match status {
        "Synced" => "success",
        "Local Only" => "drive",
        "Drive Only" => "imported",
        "Missing" => "warning",
        _ => "drive",
    }
}

fn build_target_rows(state: &AppState) -> Vec<TargetRow> {
    let targets = default_targets();
    let mut combined: Vec<_> = targets
        .into_iter()
        .map(|t| {
            let stat = state
                .stats
                .iter()
                .find(|s| s.name == t.name)
                .cloned()
                .unwrap_or_else(|| TargetStat {
                    name: t.name.clone(),
                    local_exists: false,
                    remote_exists: false,
                    local_bytes: 0,
                    remote_bytes: 0,
                    status: "Scanning".to_string(),
                });
            let enabled = state
                .settings
                .enabled_targets
                .iter()
                .any(|e| e.eq_ignore_ascii_case(&t.name));
            (t, stat, enabled)
        })
        .collect();

    let asc = state.sort_asc;
    combined.sort_by(|a, b| {
        let ord = match state.sort_col {
            1 => a.1.status.cmp(&b.1.status),
            2 => a.1.local_bytes.cmp(&b.1.local_bytes),
            3 => a.1.remote_bytes.cmp(&b.1.remote_bytes),
            _ => a.0.label.cmp(&b.0.label),
        };
        if asc {
            ord
        } else {
            ord.reverse()
        }
    });

    combined
        .into_iter()
        .map(|(t, stat, enabled)| TargetRow {
            name: SharedString::from(t.name),
            label: SharedString::from(t.label),
            enabled,
            status: SharedString::from(stat.status.as_str()),
            status_tone: SharedString::from(status_to_tone(&stat.status)),
            local_size: SharedString::from(format_bytes(stat.local_bytes, stat.local_exists)),
            drive_size: SharedString::from(format_bytes(stat.remote_bytes, stat.remote_exists)),
            local_path: SharedString::from(t.local_path.display().to_string()),
        })
        .collect()
}

fn refresh_rows_in_ui(ui: &MainWindow, state: &AppState) {
    let rows = build_target_rows(state);
    let model = Rc::new(VecModel::from(rows));
    ui.set_targets(ModelRc::from(model));
    ui.set_sort_column(state.sort_col);
    ui.set_sort_ascending(state.sort_asc);
}

fn append_log(ui: &MainWindow, line: &str) {
    let ts = Local::now().format("%H:%M:%S");
    let mut current = ui.get_log_text().to_string();
    if !current.is_empty() {
        current.push('\n');
    }
    current.push_str(&format!("[{ts}] {line}"));
    ui.set_log_text(SharedString::from(current));
}

fn spawn_refresh(
    ui_weak: slint::Weak<MainWindow>,
    state: Arc<Mutex<AppState>>,
    refresh_seq: Arc<AtomicU64>,
) {
    let seq = refresh_seq.fetch_add(1, Ordering::SeqCst) + 1;
    let sync_root = {
        let guard = state.lock().unwrap();
        PathBuf::from(guard.settings.sync_root.trim())
    };

    thread::spawn(move || {
        let running = is_antigravity_running();
        let stats = scan_target_stats(&sync_root);
        let _ = slint::invoke_from_event_loop(move || {
            if refresh_seq.load(Ordering::SeqCst) != seq {
                return;
            }
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_antigravity_running(running);
                let mut guard = state.lock().unwrap();
                guard.stats = stats;
                refresh_rows_in_ui(&ui, &guard);
            }
        });
    });
}

fn start_sync_task(
    mode: SyncMode,
    relaunch_override: Option<bool>,
    auto_exit_after: bool,
    ui_weak: slint::Weak<MainWindow>,
    state: Arc<Mutex<AppState>>,
    refresh_seq: Arc<AtomicU64>,
) {
    let (sync_root, enabled_targets, relaunch, cancel) = {
        let mut guard = state.lock().unwrap();
        if guard.cancel_flag.is_some() {
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        guard.cancel_flag = Some(Arc::clone(&cancel));
        (
            PathBuf::from(guard.settings.sync_root.trim()),
            guard.settings.enabled_targets.clone(),
            relaunch_override.unwrap_or(guard.settings.auto_relaunch),
            cancel,
        )
    };

    if let Some(ui) = ui_weak.upgrade() {
        ui.set_is_busy(true);
        ui.set_progress(0.0);
        ui.set_status_text(SharedString::from(match mode {
            SyncMode::Push => "Starting Push...",
            SyncMode::Pull => "Starting Pull...",
        }));
        append_log(
            &ui,
            &format!(
                "Starting {} with {}...",
                match mode {
                    SyncMode::Push => "Push",
                    SyncMode::Pull => "Pull",
                },
                sync_root.display()
            ),
        );
    }

    let ui_progress = Mutex::new(ui_weak.clone());
    thread::spawn(move || {
        let result = run_sync(
            mode,
            &sync_root,
            &enabled_targets,
            Arc::clone(&cancel),
            move |prog| {
                let ui_cb = ui_progress.lock().unwrap().clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_cb.upgrade() {
                        ui.set_progress((prog.percent / 100.0).clamp(0.0, 1.0));
                        ui.set_status_text(SharedString::from(prog.status_text));
                        if let Some(line) = prog.log_line {
                            append_log(&ui, &line);
                        }
                    }
                });
            },
        );

        let relaunch_msg = match (&result, relaunch) {
            (Ok(_), true) => match launch_antigravity() {
                Ok(()) => Some("Relaunched Antigravity.".to_string()),
                Err(e) => Some(format!("Note: {e}")),
            },
            _ => None,
        };

        let ui_done = ui_weak.clone();
        let state_done = Arc::clone(&state);
        let refresh_done = Arc::clone(&refresh_seq);
        let ok_exit = result.is_ok() && auto_exit_after;

        let _ = slint::invoke_from_event_loop(move || {
            {
                let mut guard = state_done.lock().unwrap();
                guard.cancel_flag = None;
            }
            if let Some(ui) = ui_done.upgrade() {
                ui.set_is_busy(false);
                match result {
                    Ok(summary) => {
                        ui.set_progress(1.0);
                        ui.set_status_text(SharedString::from(&summary));
                        append_log(&ui, &summary);
                        if let Some(msg) = relaunch_msg {
                            append_log(&ui, &msg);
                        }
                    }
                    Err(err) => {
                        ui.set_progress(0.0);
                        ui.set_status_text(SharedString::from(&err));
                        append_log(&ui, &format!("ERROR: {err}"));
                    }
                }
                spawn_refresh(ui_done.clone(), state_done, refresh_done);
            }
            if ok_exit {
                let _ = slint::quit_event_loop();
            }
        });
    });
}

fn spawn_update_check(
    ui_weak: slint::Weak<MainWindow>,
    state: Arc<Mutex<AppState>>,
    manual: bool,
) {
    if manual {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_checking_updates(true);
            ui.set_status_text(SharedString::from("Checking GitHub Releases for updates..."));
            append_log(&ui, "Checking GitHub Releases for updates...");
        }
    }

    thread::spawn(move || {
        let res = check_for_update();
        match res {
            Ok(Some(info)) => {
                let ver = info.version.clone();
                let notes = if info.release_notes.trim().is_empty() {
                    format!("Version v{ver} is available.\n\n{DEFAULT_RELEASE_NOTES}")
                } else {
                    format!(
                        "What's new in v{ver}:\n{}\n\n---\n\n{DEFAULT_RELEASE_NOTES}",
                        info.release_notes.trim()
                    )
                };
                {
                    let mut guard = state.lock().unwrap();
                    guard.pending_update = Some(info);
                }
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_checking_updates(false);
                        ui.set_latest_version(SharedString::from(ver.as_str()));
                        ui.set_release_notes_text(SharedString::from(notes));
                        ui.set_update_available(true);
                        if manual {
                            ui.set_status_text(SharedString::from(format!("Update v{ver} available")));
                            append_log(&ui, &format!("Update available: v{ver}"));
                            ui.set_show_release_notes(true);
                        }
                    }
                });
            }
            Ok(None) => {
                if manual {
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_weak.upgrade() {
                            ui.set_checking_updates(false);
                            ui.set_status_text(SharedString::from(format!("Up to date (v{APP_VERSION})")));
                            append_log(&ui, &format!("You are up to date (v{APP_VERSION})."));
                        }
                    });
                }
            }
            Err(e) => {
                if manual {
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_weak.upgrade() {
                            ui.set_checking_updates(false);
                            ui.set_status_text(SharedString::from("Update check failed — open Releases to check manually."));
                            append_log(&ui, &format!("Update check failed: {e}"));
                        }
                    });
                }
            }
        }
    });
}

fn main() -> Result<(), slint::PlatformError> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // Fast-path CLI launcher flag (--launch)
    if args.iter().any(|a| a.eq_ignore_ascii_case("--launch")) {
        let _ = launch_antigravity();
        return Ok(());
    }

    let Some(guard) = SingleInstanceGuard::acquire() else {
        return Ok(());
    };

    let auto_mode = if args.iter().any(|a| a.eq_ignore_ascii_case("--push")) {
        Some(SyncMode::Push)
    } else if args.iter().any(|a| a.eq_ignore_ascii_case("--pull")) {
        Some(SyncMode::Pull)
    } else {
        None
    };
    let relaunch_arg = args.iter().any(|a| a.eq_ignore_ascii_case("--relaunch"));

    let loaded_settings = AppSettings::load();
    let system_dark = is_windows_dark_mode();

    let ui = MainWindow::new()?;
    ui.set_app_version(SharedString::from(APP_VERSION));
    ui.set_sync_root(SharedString::from(loaded_settings.sync_root.as_str()));
    ui.set_auto_relaunch(loaded_settings.auto_relaunch);
    ui.set_auto_check_updates(loaded_settings.auto_check_updates);
    ui.set_theme_label(SharedString::from(loaded_settings.theme.as_str()));
    ui.set_release_notes_text(SharedString::from(DEFAULT_RELEASE_NOTES));

    let palette = ui.global::<Palette>();
    palette.set_system_dark(system_dark);
    palette.set_mode(theme_str_to_mode(&loaded_settings.theme));

    let state = Arc::new(Mutex::new(AppState {
        settings: loaded_settings,
        stats: Vec::new(),
        sort_col: 0,
        sort_asc: true,
        pending_update: None,
        cancel_flag: None,
        single_instance: Some(guard),
    }));
    let refresh_seq = Arc::new(AtomicU64::new(0));

    {
        let st = state.lock().unwrap();
        refresh_rows_in_ui(&ui, &st);
    }

    // Push
    {
        let ui_w = ui.as_weak();
        let st = Arc::clone(&state);
        let rseq = Arc::clone(&refresh_seq);
        ui.on_push_clicked(move || {
            start_sync_task(SyncMode::Push, None, false, ui_w.clone(), Arc::clone(&st), Arc::clone(&rseq));
        });
    }

    // Pull
    {
        let ui_w = ui.as_weak();
        let st = Arc::clone(&state);
        let rseq = Arc::clone(&refresh_seq);
        ui.on_pull_clicked(move || {
            start_sync_task(SyncMode::Pull, None, false, ui_w.clone(), Arc::clone(&st), Arc::clone(&rseq));
        });
    }

    // Launch Antigravity
    {
        let ui_w = ui.as_weak();
        let st = Arc::clone(&state);
        let rseq = Arc::clone(&refresh_seq);
        ui.on_launch_clicked(move || {
            if let Some(ui) = ui_w.upgrade() {
                match launch_antigravity() {
                    Ok(()) => {
                        append_log(&ui, "Launched Antigravity.");
                        ui.set_antigravity_running(true);
                    }
                    Err(e) => append_log(&ui, &format!("ERROR: {e}")),
                }
                spawn_refresh(ui_w.clone(), Arc::clone(&st), Arc::clone(&rseq));
            }
        });
    }

    // Refresh
    {
        let ui_w = ui.as_weak();
        let st = Arc::clone(&state);
        let rseq = Arc::clone(&refresh_seq);
        ui.on_refresh_clicked(move || {
            spawn_refresh(ui_w.clone(), Arc::clone(&st), Arc::clone(&rseq));
        });
    }

    // Cancel
    {
        let st = Arc::clone(&state);
        ui.on_cancel_clicked(move || {
            let guard = st.lock().unwrap();
            if let Some(flag) = &guard.cancel_flag {
                flag.store(true, Ordering::SeqCst);
            }
        });
    }

    // Browse Sync Folder
    {
        let ui_w = ui.as_weak();
        let st = Arc::clone(&state);
        let rseq = Arc::clone(&refresh_seq);
        ui.on_browse_clicked(move || {
            let current = { st.lock().unwrap().settings.sync_root.clone() };
            let mut dialog = rfd::FileDialog::new();
            let p = PathBuf::from(&current);
            if p.exists() {
                dialog = dialog.set_directory(p);
            }
            if let Some(folder) = dialog.pick_folder() {
                let folder_str = folder.display().to_string();
                {
                    let mut guard = st.lock().unwrap();
                    guard.settings.sync_root = folder_str.clone();
                    let _ = guard.settings.save();
                }
                if let Some(ui) = ui_w.upgrade() {
                    ui.set_sync_root(SharedString::from(folder_str));
                }
                spawn_refresh(ui_w.clone(), Arc::clone(&st), Arc::clone(&rseq));
            }
        });
    }

    // Settings changed (LineEdit / CheckBoxes)
    {
        let ui_w = ui.as_weak();
        let st = Arc::clone(&state);
        let rseq = Arc::clone(&refresh_seq);
        ui.on_settings_changed(move || {
            if let Some(ui) = ui_w.upgrade() {
                let root_changed = {
                    let mut guard = st.lock().unwrap();
                    let new_root = ui.get_sync_root().trim().to_string();
                    let changed = guard.settings.sync_root != new_root;
                    guard.settings.sync_root = new_root;
                    guard.settings.auto_relaunch = ui.get_auto_relaunch();
                    guard.settings.auto_check_updates = ui.get_auto_check_updates();
                    let _ = guard.settings.save();
                    changed
                };
                if root_changed {
                    spawn_refresh(ui_w.clone(), Arc::clone(&st), Arc::clone(&rseq));
                }
            }
        });
    }

    // Cycle Theme
    {
        let ui_w = ui.as_weak();
        let st = Arc::clone(&state);
        ui.on_cycle_theme(move || {
            let new_theme = {
                let mut guard = st.lock().unwrap();
                guard.settings.theme = next_theme(&guard.settings.theme);
                let _ = guard.settings.save();
                guard.settings.theme.clone()
            };
            if let Some(ui) = ui_w.upgrade() {
                ui.set_theme_label(SharedString::from(new_theme.as_str()));
                ui.global::<Palette>().set_mode(theme_str_to_mode(&new_theme));
            }
        });
    }

    // Toggle Target checkbox
    {
        let ui_w = ui.as_weak();
        let st = Arc::clone(&state);
        ui.on_toggle_target(move |row_idx| {
            if let Some(ui) = ui_w.upgrade() {
                let mut guard = st.lock().unwrap();
                let rows = build_target_rows(&guard);
                if let Some(row) = rows.get(row_idx as usize) {
                    let name = row.name.to_string();
                    if let Some(pos) = guard
                        .settings
                        .enabled_targets
                        .iter()
                        .position(|t| t.eq_ignore_ascii_case(&name))
                    {
                        guard.settings.enabled_targets.remove(pos);
                    } else {
                        guard.settings.enabled_targets.push(name);
                    }
                    let _ = guard.settings.save();
                    refresh_rows_in_ui(&ui, &guard);
                }
            }
        });
    }

    // Sort Data Grid
    {
        let ui_w = ui.as_weak();
        let st = Arc::clone(&state);
        ui.on_sort_by(move |col| {
            if let Some(ui) = ui_w.upgrade() {
                let mut guard = st.lock().unwrap();
                if guard.sort_col == col {
                    guard.sort_asc = !guard.sort_asc;
                } else {
                    guard.sort_col = col;
                    guard.sort_asc = true;
                }
                refresh_rows_in_ui(&ui, &guard);
            }
        });
    }

    // Open GitHub Releases
    ui.on_open_releases(move || {
        open_url(RELEASES_PAGE_URL);
    });

    // Manual Check for Updates
    {
        let ui_w = ui.as_weak();
        let st = Arc::clone(&state);
        ui.on_check_updates(move || {
            spawn_update_check(ui_w.clone(), Arc::clone(&st), true);
        });
    }

    // 1-Click Auto-Update Download & Install
    {
        let ui_w = ui.as_weak();
        let st = Arc::clone(&state);
        ui.on_install_update(move || {
            let info = {
                let guard = st.lock().unwrap();
                guard.pending_update.clone()
            };
            let Some(info) = info else {
                open_url(RELEASES_PAGE_URL);
                return;
            };

            if let Some(ui) = ui_w.upgrade() {
                ui.set_is_busy(true);
                ui.set_show_release_notes(false);
                append_log(&ui, &format!("Downloading update v{}...", info.version));
            }

            let ui_prog = ui_w.clone();
            let ui_done = ui_w.clone();
            let st_done = Arc::clone(&st);
            thread::spawn(move || {
                let res = download_and_verify_update(&info, move |frac, text| {
                    let u = ui_prog.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = u.upgrade() {
                            ui.set_progress(frac);
                            ui.set_status_text(SharedString::from(text));
                        }
                    });
                });

                let _ = slint::invoke_from_event_loop(move || match res {
                    Ok(installer_path) => {
                        // Release single-instance mutex cleanly before launching silent installer
                        {
                            let mut guard = st_done.lock().unwrap();
                            guard.single_instance = None;
                        }
                        match launch_silent_installer(&installer_path) {
                            Ok(()) => {
                                let _ = slint::quit_event_loop();
                            }
                            Err(e) => {
                                if let Some(ui) = ui_done.upgrade() {
                                    ui.set_is_busy(false);
                                    append_log(&ui, &format!("ERROR: {e}"));
                                }
                            }
                        }
                    }
                    Err(e) => {
                        if let Some(ui) = ui_done.upgrade() {
                            ui.set_is_busy(false);
                            ui.set_status_text(SharedString::from("Update failed — open Releases page to download manually."));
                            append_log(&ui, &format!("Update failed: {e}"));
                        }
                    }
                });
            });
        });
    }

    // Initial background scan + optional update check or auto CLI push/pull
    spawn_refresh(ui.as_weak(), Arc::clone(&state), Arc::clone(&refresh_seq));

    if let Some(mode) = auto_mode {
        start_sync_task(
            mode,
            Some(relaunch_arg || state.lock().unwrap().settings.auto_relaunch),
            true,
            ui.as_weak(),
            Arc::clone(&state),
            Arc::clone(&refresh_seq),
        );
    } else if state.lock().unwrap().settings.auto_check_updates {
        spawn_update_check(ui.as_weak(), Arc::clone(&state), false);
    }

    ui.run()
}
