use rusqlite::{Connection, OpenFlags};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

const MTIME_TOLERANCE: Duration = Duration::from_secs(2);

#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct MergeStats {
    pub copied: usize,
    pub updated: usize,
    pub kept_existing: usize,
    pub conflicts_resolved: usize,
    pub orphans_removed: usize,
    pub rows_merged: usize,
}

fn check_cancelled(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        Err("Sync cancelled by user.".to_string())
    } else {
        Ok(())
    }
}

fn file_mtime(path: &Path) -> SystemTime {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

fn file_size(path: &Path) -> u64 {
    fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

fn is_excluded_file(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".log") || lower.ends_with(".tmp") || lower.ends_with(".bak")
}

/// Detects Google Drive conflict copies like `foo (1).pb`, `uuid (2).db`, or `name (1)`.
/// Returns the canonical base filename if `file_name` matches a Drive conflict pattern.
pub fn canonical_name_if_conflict(file_name: &str) -> Option<String> {
    // Split stem and extension (only last dot after any ')')
    let (stem, ext) = match file_name.rfind('.') {
        Some(dot_idx) => (&file_name[..dot_idx], &file_name[dot_idx..]),
        None => (file_name, ""),
    };

    let open = stem.rfind(" (")?;
    if !stem.ends_with(')') {
        return None;
    }
    let digits = &stem[open + 2..stem.len() - 1];
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let base_stem = &stem[..open];
    if base_stem.is_empty() {
        return None;
    }
    Some(format!("{base_stem}{ext}"))
}

/// Runs `PRAGMA wal_checkpoint(TRUNCATE)` on a SQLite database so all WAL pages are flushed
/// into the main `.db` file and sidecar `-wal`/`-shm` files are cleanly removed.
pub fn checkpoint_sqlite_file(db_path: &Path) -> bool {
    if !db_path.is_file() {
        return false;
    }
    let Ok(conn) = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return false;
    };
    let _ = conn.busy_timeout(Duration::from_millis(2000));
    let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    true
}

/// Returns `true` if `db_path` opens cleanly and passes `PRAGMA quick_check`.
pub fn sqlite_quick_check(db_path: &Path) -> bool {
    if !db_path.is_file() || file_size(db_path) == 0 {
        return false;
    }
    let Ok(conn) = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return false;
    };
    let _ = conn.busy_timeout(Duration::from_millis(1500));
    conn.query_row("PRAGMA quick_check;", [], |row| row.get::<_, String>(0))
        .map(|res| res.eq_ignore_ascii_case("ok"))
        .unwrap_or(false)
}

/// Returns `true` if `candidate` and `existing` have the same size and mtimes within `MTIME_TOLERANCE`.
fn is_unchanged(candidate: &Path, existing: &Path) -> bool {
    if !existing.is_file() {
        return false;
    }
    if file_size(candidate) != file_size(existing) {
        return false;
    }
    let cand_time = file_mtime(candidate);
    let exist_time = file_mtime(existing);
    let diff = cand_time
        .duration_since(exist_time)
        .or_else(|_| exist_time.duration_since(cand_time));
    matches!(diff, Ok(d) if d <= MTIME_TOLERANCE)
}

/// Returns `true` if `candidate` should replace `existing` (newest-wins with 2s Drive tolerance;
/// within 2s, prefers larger file if sizes differ, otherwise strictly newer mtime).
fn is_newer_or_better(candidate: &Path, existing: &Path) -> bool {
    let cand_time = file_mtime(candidate);
    let exist_time = file_mtime(existing);

    if let Ok(diff) = cand_time.duration_since(exist_time) {
        if diff > MTIME_TOLERANCE {
            return true;
        }
    }
    if let Ok(diff) = exist_time.duration_since(cand_time) {
        if diff > MTIME_TOLERANCE {
            return false;
        }
    }
    let cand_size = file_size(candidate);
    let exist_size = file_size(existing);
    if cand_size != exist_size {
        return cand_size > exist_size;
    }
    cand_time > exist_time
}

fn copy_file_atomic(src: &Path, dst: &Path) -> Result<(), String> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create directory '{}': {e}", parent.display()))?;
    }
    let tmp = dst.with_extension("sync_tmp");
    fs::copy(src, &tmp).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!(
            "Failed to copy '{}' -> '{}': {e}",
            src.display(),
            dst.display()
        )
    })?;
    if let Err(rename_err) = fs::rename(&tmp, dst) {
        // Fallback if destination exists on Windows
        let _ = fs::remove_file(dst);
        if let Err(retry_err) = fs::rename(&tmp, dst) {
            let _ = fs::remove_file(&tmp);
            return Err(format!(
                "Failed to finalize '{}' (rename: {rename_err}; retry: {retry_err})",
                dst.display()
            ));
        }
    }
    Ok(())
}

fn remove_sqlite_sidecars(db_path: &Path) {
    let Some(name) = db_path.file_name().and_then(|n| n.to_str()) else {
        return;
    };
    let _ = fs::remove_file(db_path.with_file_name(format!("{name}-wal")));
    let _ = fs::remove_file(db_path.with_file_name(format!("{name}-shm")));
}

/// Resolves Google Drive conflict duplicates (`foo (1).ext`) in a single directory (non-recursive).
/// Keeps the newer/valid file under the canonical filename and deletes the conflict copy.
pub fn cleanup_drive_conflicts_in_dir(dir: &Path) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };

    let mut resolved = 0usize;
    let mut conflicts: Vec<(PathBuf, PathBuf)> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if let Some(canonical) = canonical_name_if_conflict(name) {
            conflicts.push((path, dir.join(canonical)));
        }
    }

    for (conflict_path, base_path) in conflicts {
        let is_db = base_path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("db"));

        if is_db {
            let _ = checkpoint_sqlite_file(&conflict_path);
            let _ = checkpoint_sqlite_file(&base_path);
        }

        let conflict_valid = !is_db || sqlite_quick_check(&conflict_path);
        let base_exists = base_path.is_file();
        let base_valid = base_exists && (!is_db || sqlite_quick_check(&base_path));

        if !base_exists {
            if conflict_valid && copy_file_atomic(&conflict_path, &base_path).is_ok() {
                let _ = fs::remove_file(&conflict_path);
                resolved += 1;
            }
        } else if conflict_valid && (!base_valid || is_newer_or_better(&conflict_path, &base_path))
        {
            if copy_file_atomic(&conflict_path, &base_path).is_ok() {
                if is_db {
                    remove_sqlite_sidecars(&base_path);
                }
                let _ = fs::remove_file(&conflict_path);
                resolved += 1;
            }
        } else {
            // Base is already newer/valid (or conflict is corrupt): drop duplicate copy
            if fs::remove_file(&conflict_path).is_ok() {
                resolved += 1;
            }
        }
    }

    resolved
}

/// Removes orphaned `*-wal` / `*-shm` files whose parent `.db` file does not exist.
pub fn cleanup_orphaned_wal_files(dir: &Path) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };

    let mut removed = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let parent_db_name = if let Some(base) = name.strip_suffix("-wal") {
            Some(base)
        } else {
            name.strip_suffix("-shm")
        };

        if let Some(base_name) = parent_db_name {
            if !dir.join(base_name).is_file() && fs::remove_file(&path).is_ok() {
                removed += 1;
            }
        }
    }
    removed
}

/// Non-destructive, checkpoint-first, integrity-checked merge for `conversations/`.
/// Fast-skips unchanged `.db` files (same size & mtime within 2s) so Google Drive folders
/// never stall on unnecessary SQLite quick_checks; never deletes chats that only exist on `dst`.
pub fn merge_conversations_dir(
    src: &Path,
    dst: &Path,
    cancel: &AtomicBool,
) -> Result<MergeStats, String> {
    check_cancelled(cancel)?;
    fs::create_dir_all(dst)
        .map_err(|e| format!("Failed to create '{}': {e}", dst.display()))?;

    let mut stats = MergeStats::default();
    stats.conflicts_resolved += cleanup_drive_conflicts_in_dir(src);
    stats.conflicts_resolved += cleanup_drive_conflicts_in_dir(dst);
    stats.orphans_removed += cleanup_orphaned_wal_files(src);
    stats.orphans_removed += cleanup_orphaned_wal_files(dst);

    let entries = fs::read_dir(src)
        .map_err(|e| format!("Failed to read '{}': {e}", src.display()))?;

    for entry in entries.flatten() {
        check_cancelled(cancel)?;
        let src_path = entry.path();
        if !src_path.is_file() {
            continue;
        }
        let Some(name) = src_path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let lower = name.to_ascii_lowercase();
        if lower.ends_with("-wal") || lower.ends_with("-shm") || is_excluded_file(name) || canonical_name_if_conflict(name).is_some() {
            continue;
        }

        let dst_path = dst.join(name);
        if lower.ends_with(".db") {
            let wal_exists = src_path.with_file_name(format!("{name}-wal")).is_file();
            // Fast path: if destination exists with same size & mtime (and no pending source WAL), skip SQLite open.
            if !wal_exists && is_unchanged(&src_path, &dst_path) {
                stats.kept_existing += 1;
                continue;
            }

            // If destination exists and is strictly newer than source (and has no WAL to flush), keep destination.
            if dst_path.is_file() && !wal_exists && !is_newer_or_better(&src_path, &dst_path) {
                stats.kept_existing += 1;
                continue;
            }

            // Flush any WAL journal on source before checking & copying
            if wal_exists {
                let _ = checkpoint_sqlite_file(&src_path);
            }
            if !sqlite_quick_check(&src_path) {
                continue;
            }

            if !dst_path.is_file() {
                remove_sqlite_sidecars(&dst_path);
                copy_file_atomic(&src_path, &dst_path)?;
                stats.copied += 1;
            } else {
                let _ = checkpoint_sqlite_file(&dst_path);
                let dst_valid = sqlite_quick_check(&dst_path);
                if !dst_valid || is_newer_or_better(&src_path, &dst_path) {
                    remove_sqlite_sidecars(&dst_path);
                    copy_file_atomic(&src_path, &dst_path)?;
                    stats.updated += 1;
                } else {
                    stats.kept_existing += 1;
                }
            }
        } else if !dst_path.is_file() {
            copy_file_atomic(&src_path, &dst_path)?;
            stats.copied += 1;
        } else if is_newer_or_better(&src_path, &dst_path) {
            copy_file_atomic(&src_path, &dst_entry_or(&dst_path))?;
            stats.updated += 1;
        } else {
            stats.kept_existing += 1;
        }
    }

    stats.orphans_removed += cleanup_orphaned_wal_files(dst);
    Ok(stats)
}

fn dst_entry_or(p: &Path) -> PathBuf {
    p.to_path_buf()
}

/// True row-level SQLite merge for `conversation_summaries.db`.
/// Merges `conversation_summaries` by `conversation_id` (keeping the row with the newer
/// `last_modified_time` or higher `step_count`) and `conversation_parents` by `conversation_id`,
/// so chats from both PC and Laptop always appear together without overwriting each other.
pub fn merge_conversation_summaries_db(
    src: &Path,
    dst: &Path,
    cancel: &AtomicBool,
) -> Result<MergeStats, String> {
    check_cancelled(cancel)?;
    let mut stats = MergeStats::default();

    let _ = checkpoint_sqlite_file(src);
    if !sqlite_quick_check(src) {
        return Err(format!(
            "Source Chat Index DB '{}' failed integrity check.",
            src.display()
        ));
    }

    if !dst.is_file() || !sqlite_quick_check(dst) {
        remove_sqlite_sidecars(dst);
        copy_file_atomic(src, dst)?;
        stats.copied = 1;
        return Ok(stats);
    }

    let _ = checkpoint_sqlite_file(dst);

    let conn = Connection::open_with_flags(
        dst,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("Failed to open '{}': {e}", dst.display()))?;
    let _ = conn.busy_timeout(Duration::from_millis(3000));

    let src_str = src.to_string_lossy().replace('\'', "''");
    conn.execute_batch(&format!("ATTACH DATABASE '{src_str}' AS remote_sync;"))
        .map_err(|e| format!("Failed to attach '{}': {e}", src.display()))?;

    // Discover shared columns in `conversation_summaries` so schema upgrades remain compatible.
    let main_cols = table_columns(&conn, "main", "conversation_summaries");
    let remote_cols = table_columns(&conn, "remote_sync", "conversation_summaries");
    let remote_set: HashSet<&str> = remote_cols.iter().map(|s| s.as_str()).collect();
    let shared_cols: Vec<String> = main_cols
        .into_iter()
        .filter(|c| remote_set.contains(c.as_str()))
        .collect();

    if shared_cols.iter().any(|c| c == "conversation_id") {
        let col_list = shared_cols
            .iter()
            .map(|c| format!("`{c}`"))
            .collect::<Vec<_>>()
            .join(", ");
        let r_col_list = shared_cols
            .iter()
            .map(|c| format!("r.`{c}`"))
            .collect::<Vec<_>>()
            .join(", ");

        let has_mtime = shared_cols.iter().any(|c| c == "last_modified_time");
        let has_steps = shared_cols.iter().any(|c| c == "step_count");

        let sql = if has_mtime && has_steps {
            format!(
                "INSERT OR REPLACE INTO main.conversation_summaries ({col_list}) \
                 SELECT {r_col_list} FROM remote_sync.conversation_summaries r \
                 LEFT JOIN main.conversation_summaries m ON m.conversation_id = r.conversation_id \
                 WHERE m.conversation_id IS NULL \
                    OR r.last_modified_time > m.last_modified_time \
                    OR (r.last_modified_time = m.last_modified_time AND r.step_count > m.step_count);"
            )
        } else {
            format!(
                "INSERT OR IGNORE INTO main.conversation_summaries ({col_list}) \
                 SELECT {r_col_list} FROM remote_sync.conversation_summaries r;"
            )
        };

        let merged = conn
            .execute(&sql, [])
            .map_err(|e| format!("Failed merging conversation_summaries rows: {e}"))?;
        stats.rows_merged += merged;
    }

    // Merge conversation_parents (subagent links) if present in both DBs
    let main_parents = table_columns(&conn, "main", "conversation_parents");
    let remote_parents = table_columns(&conn, "remote_sync", "conversation_parents");
    if !main_parents.is_empty() && !remote_parents.is_empty() {
        let _ = conn.execute(
            "INSERT OR IGNORE INTO main.conversation_parents (conversation_id, parent_id) \
             SELECT conversation_id, parent_id FROM remote_sync.conversation_parents;",
            [],
        );
    }

    let _ = conn.execute_batch("DETACH DATABASE remote_sync;");
    let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    drop(conn);

    remove_sqlite_sidecars(dst);
    stats.updated = 1;
    Ok(stats)
}

fn table_columns(conn: &Connection, schema: &str, table: &str) -> Vec<String> {
    let sql = format!("PRAGMA {schema}.table_info(`{table}`);");
    let Ok(mut stmt) = conn.prepare(&sql) else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(1)) else {
        return Vec::new();
    };
    rows.flatten().collect()
}

/// Safe newest-wins copy for non-SQLite single files (e.g. `agyhub_summaries_proto.pb`, `antigravity_state.pbtxt`).
pub fn merge_single_file(
    src: &Path,
    dst: &Path,
    cancel: &AtomicBool,
) -> Result<MergeStats, String> {
    check_cancelled(cancel)?;
    let mut stats = MergeStats::default();
    if !dst.is_file() {
        copy_file_atomic(src, dst)?;
        stats.copied = 1;
    } else if is_newer_or_better(src, dst) {
        copy_file_atomic(src, dst)?;
        stats.updated = 1;
    } else {
        stats.kept_existing = 1;
    }
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::UNIX_EPOCH;

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "antigravity_sync_merge_{label}_{}_{}",
            std::process::id(),
            nanos
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn create_test_summary_db(path: &Path, rows: &[(&str, &str, i64, &str)]) {
        let conn = Connection::open(path).expect("open test db");
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE conversation_summaries (
                 conversation_id TEXT PRIMARY KEY,
                 title TEXT NOT NULL DEFAULT '',
                 step_count INTEGER NOT NULL DEFAULT 0,
                 last_modified_time TEXT NOT NULL
             );
             CREATE TABLE conversation_parents (
                 conversation_id TEXT PRIMARY KEY,
                 parent_id TEXT NOT NULL
             ) WITHOUT ROWID;",
        )
        .expect("create schema");
        for (id, title, steps, mtime) in rows {
            conn.execute(
                "INSERT INTO conversation_summaries (conversation_id, title, step_count, last_modified_time) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![id, title, steps, mtime],
            )
            .expect("insert row");
        }
    }

    #[test]
    fn canonical_name_detects_google_drive_conflicts() {
        assert_eq!(
            canonical_name_if_conflict("agyhub_summaries_proto (1).pb"),
            Some("agyhub_summaries_proto.pb".to_string())
        );
        assert_eq!(
            canonical_name_if_conflict("abc-123 (2).db"),
            Some("abc-123.db".to_string())
        );
        assert_eq!(canonical_name_if_conflict("normal.db"), None);
        assert_eq!(canonical_name_if_conflict("file (draft).txt"), None);
    }

    #[test]
    fn merge_conversations_keeps_both_machines_chats_and_cleans_orphans() {
        let root = unique_temp_dir("conv");
        let src = root.join("src");
        let dst = root.join("dst");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&dst).unwrap();

        create_test_summary_db(&src.join("laptop-chat.db"), &[("1", "Laptop", 5, "2026-11-27 14:00:00")]);
        create_test_summary_db(&dst.join("pc-chat.db"), &[("2", "PC", 8, "2026-11-27 14:05:00")]);
        // Orphan WAL on dst without matching .db
        fs::write(dst.join("deleted-chat.db-wal"), b"orphan").unwrap();

        let cancel = AtomicBool::new(false);
        let stats = merge_conversations_dir(&src, &dst, &cancel).expect("merge ok");

        assert!(dst.join("laptop-chat.db").is_file());
        assert!(dst.join("pc-chat.db").is_file(), "PC chat must never be deleted!");
        assert!(!dst.join("deleted-chat.db-wal").exists(), "Orphan WAL must be removed");
        assert_eq!(stats.copied, 1);
        assert_eq!(stats.orphans_removed, 1);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn merge_conversation_summaries_preserves_all_chats_and_picks_newer_row() {
        let root = unique_temp_dir("summaries");
        let src = root.join("remote.db");
        let dst = root.join("local.db");

        create_test_summary_db(
            &dst,
            &[
                ("pc-only", "PC Chat", 10, "2026-11-27 13:00:00"),
                ("shared", "Shared Old", 3, "2026-11-27 12:00:00"),
            ],
        );
        create_test_summary_db(
            &src,
            &[
                ("laptop-only", "Laptop Chat", 4, "2026-11-27 14:00:00"),
                ("shared", "Shared Newer", 9, "2026-11-27 14:30:00"),
            ],
        );

        let cancel = AtomicBool::new(false);
        let stats = merge_conversation_summaries_db(&src, &dst, &cancel).expect("row merge ok");
        assert_eq!(stats.rows_merged, 2);

        let conn = Connection::open(&dst).unwrap();
        let count: i64 = conn
            .query_row("SELECT count(*) FROM conversation_summaries", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 3, "All 3 chats (PC + Laptop + Shared) must exist");

        let shared_title: String = conn
            .query_row(
                "SELECT title FROM conversation_summaries WHERE conversation_id = 'shared'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(shared_title, "Shared Newer");

        let _ = fs::remove_dir_all(&root);
    }
}
