# Antigravity Sync (Rust + Slint)

Native Windows 11 Fluent desktop utility, launcher, and 1-click installer to sync your Antigravity conversations, brain artifacts, annotations, config/skills, and storage across machines via Google Drive, OneDrive, or Dropbox.

## Features (v2.0.0)
- **Rust + Slint Native GUI (`PE Subsystem 2 / WINDOWS_GUI`)** — zero console or PowerShell window flashes.
- **Fluent Desktop UI** — 4 persisted themes (**System**, **Light**, **Dark**, **Dark Blue**), bottom-anchored command bar tabs, sortable Target Data Grid with semantic status pills, live activity log, and a card-based `F1` Help Guide.
- **Built-in Launcher & 1-Click Sync** — Push, Pull, and Launch Antigravity directly from the command bar or via Desktop shortcuts (`--push`, `--pull`, `--relaunch`, `--launch`).
- **Deadlock-Free Multi-Threaded Sync** — uses `robocopy /MT:8` with null stdio (zero stdout pipe deadlocks), live `%` + ETA progress, responsive Cancel, and automatic timestamped safety backups before every Pull.
- **1-Click GitHub Auto-Updater** — checks GitHub Releases on startup, streams the installer with progress, verifies `SHA256SUMS.txt`, cleanly releases the single-instance mutex, and installs silently (`/S`).

## Build
```powershell
powershell -ExecutionPolicy Bypass -File .\build.ps1
```
