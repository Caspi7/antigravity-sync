# Antigravity Sync

One-click Windows desktop app and installer to sync your Antigravity conversations, brain artifacts, annotations, config, and storage across machines via Google Drive, OneDrive, or Dropbox.

## Features
- **Native Windows GUI (`PE Subsystem 2`)** — zero console or PowerShell window flashes.
- **1-click Push & Pull** — automatically closes Antigravity, creates a safety backup before Pull, mirrors data with `robocopy`, and optionally relaunches Antigravity.
- **Multi-theme Windows 11 Fluent UI** — supports System, Light, Dark, and Dark Blue themes (persisted).
- **Built-in 1-click Auto-Updater** — checks GitHub Releases on startup, verifies SHA-256 checksums, and installs silently.
- **Desktop Shortcuts** — installer creates **Antigravity Sync**, **Push Antigravity**, and **Pull Antigravity** shortcuts.

## Build
```powershell
powershell -ExecutionPolicy Bypass -File .\build.ps1
```
