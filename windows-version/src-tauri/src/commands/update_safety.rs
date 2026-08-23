//! Safety rails around the auto-updater so a botched install can
//! never lose story data.
//!
//! Three commands:
//!
//!   1. `pre_update_check` — runs before `downloadAndInstall`. Opens
//!      app.db read-only, runs PRAGMA integrity_check, verifies the
//!      master-key derivation inputs are present in app_meta. If any
//!      check fails the UI refuses to install and points the user at
//!      the crash log.
//!
//!   2. `backup_app_data` — copies the entire user-data directory
//!      (`~/Library/Application Support/com.lumizone.fpvdesktop/`)
//!      to a user-chosen folder. Includes app.db (+ -wal + -shm) and
//!      all story data. Optional but recommended before a major update.
//!
//!   3. `post_update_finalize` — fires on first boot of a new version
//!      (the runtime detects `current_version != app_meta["last_known_version"]`).
//!      Verifies database integrity and logs the result. Also bumps
//!      `last_known_version` so subsequent boots skip the check.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::storage;

const META_LAST_VERSION: &str = "last_known_version";
const META_LAST_UPDATE_AT: &str = "last_update_finalized_at";
const META_POST_UPDATE_RETRIES: &str = "post_update_finalize_retries";

#[derive(Debug, Serialize)]
pub struct PreUpdateCheckResult {
    pub ok: bool,
    /// Empty when ok; populated with a human-readable failure
    /// reason otherwise (UI surfaces verbatim).
    pub reason: String,
    /// App data directory size in bytes — surfaced to the UI so the
    /// user has a sense of "how much would I lose if this went
    /// wrong" before deciding whether to take the backup option.
    pub data_size_bytes: u64,
}

/// Block the auto-updater install path until app data is provably
/// healthy. Read-only — never mutates anything, safe to call as
/// often as the UI wants.
#[tauri::command]
pub async fn pre_update_check(state: State<'_, AppState>) -> AppResult<PreUpdateCheckResult> {
    let db_path = storage::db_path()?;
    let app_dir = storage::app_data_dir()?;

    // (1) DB integrity. Open read-only to make sure even if a writer
    //     held the WAL we wouldn't block — APFS lets multiple readers
    //     in even when SQLite has the WAL open. URI path needs percent
    //     encoding so users with spaces in $HOME (e.g.
    //     `/Users/Two Words/…`) don't get rejected by SQLite with an
    //     "unable to open" error blocking the update.
    let path_str = db_path.display().to_string();
    let path_escaped = path_str
        .replace('%', "%25")
        .replace(' ', "%20")
        .replace('?', "%3F")
        .replace('#', "%23");
    let uri = format!("file:{}?mode=ro", path_escaped);
    let conn = match Connection::open_with_flags(
        &uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    ) {
        Ok(c) => c,
        Err(e) => {
            return Ok(PreUpdateCheckResult {
                ok: false,
                reason: format!("Cannot open app database: {e}"),
                data_size_bytes: dir_size(&app_dir),
            });
        }
    };
    let integrity: String = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
        .unwrap_or_else(|e| format!("query failed: {e}"));
    drop(conn);
    if integrity != "ok" {
        return Ok(PreUpdateCheckResult {
            ok: false,
            reason: format!(
                "Database integrity check failed: {integrity}. \
                 Update blocked. Take a backup first or contact support."
            ),
            data_size_bytes: dir_size(&app_dir),
        });
    }

    // (2) Security metadata present. These are initialized on first
    //     launch and are needed for app identity verification.
    let conn = state.db.lock().await;
    let salt_present = crate::db::meta_get(&conn, "crypto_salt_hex")?
        .filter(|s| !s.is_empty())
        .is_some();
    let hw_present = crate::db::meta_get(&conn, "hardware_uuid")?
        .filter(|s| !s.is_empty())
        .is_some();
    drop(conn);

    if !salt_present || !hw_present {
        return Ok(PreUpdateCheckResult {
            ok: false,
            reason: "App is missing security metadata (crypto_salt or hardware_uuid). \
                     Cannot guarantee data integrity after update."
                .into(),
            data_size_bytes: dir_size(&app_dir),
        });
    }

    Ok(PreUpdateCheckResult {
        ok: true,
        reason: String::new(),
        data_size_bytes: dir_size(&app_dir),
    })
}

#[derive(Debug, Serialize)]
pub struct BackupResult {
    pub backup_path: String,
    pub bytes: u64,
    pub file_count: u32,
}

/// Copy the whole user-data directory to a destination folder.
/// `dest_dir` must already exist (frontend file picker creates it).
/// We mint a child folder named `fpvdesktop-backup-<unix>` so a
/// dest dir can hold multiple snapshots without overwrite.
///
/// Unlike `project_export` (which curates specific tables and asset
/// folders), this is a raw copy of everything under the app-data dir —
/// the whole SQLite database, model weights, images, logs. BYOK
/// credentials normally never touch disk here (they live in the OS
/// keychain / DPAPI-encrypted `secrets.json`), but the one exception —
/// `secret_store`'s legacy plaintext `secrets.json` fallback, kept only
/// for pre-migration installs — is excluded explicitly so a backup
/// folder (which the user may place anywhere, including a cloud-synced
/// directory) can never carry an unencrypted copy of an API key.
#[tauri::command(rename_all = "snake_case")]
pub async fn backup_app_data(
    state: tauri::State<'_, crate::state::AppState>,
    dest_dir: String,
) -> AppResult<BackupResult> {
    let dest_root = PathBuf::from(&dest_dir);
    if !dest_root.is_dir() {
        return Err(AppError::Config(format!(
            "Backup destination is not a directory: {dest_dir}"
        )));
    }
    // Resolve the path against the filesystem before joining the
    // child folder. Without this, a frontend-supplied dest_dir like
    // "/tmp/../../etc" would survive the is_dir() check (the target
    // exists) and the recursive copy would write inside the resolved
    // location. Canonicalize fails-closed if the path can't be
    // resolved (broken symlink, vanished dir) so an attacker can't
    // race the check.
    let dest_root = dest_root
        .canonicalize()
        .map_err(|e| AppError::Config(format!("cannot resolve backup destination: {e}")))?;
    let src = storage::app_data_dir()?;
    // Defence in depth: reject destinations inside our own app-data
    // dir — backing up into the source produces an infinite copy
    // loop and (worse) lets a malicious frontend trick the user into
    // overwriting their data files.
    let src_canon = src.canonicalize().unwrap_or_else(|_| src.clone());
    if dest_root.starts_with(&src_canon) || src_canon.starts_with(&dest_root) {
        return Err(AppError::Config(
            "Backup destination overlaps FPV's data directory — pick another folder".into(),
        ));
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let backup_prefix = dest_root.join(format!("fpvdesktop-backup-{now}"));

    // Hold the DB mutex across checkpoint and copy. This prevents an app
    // writer from creating a new WAL between the checkpoint and the app.db
    // copy, which would otherwise make the backup internally inconsistent.
    let conn = state.db.lock().await;
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .map_err(|e| AppError::Other(format!("cannot create a consistent SQLite backup: {e}")))?;

    // Seconds are not unique when two backups start together. Reserve the
    // directory atomically and add a suffix on collision.
    let backup_dir = (0u32..)
        .map(|suffix| {
            if suffix == 0 {
                backup_prefix.clone()
            } else {
                dest_root.join(format!("fpvdesktop-backup-{now}-{suffix}"))
            }
        })
        .find(|candidate| std::fs::create_dir(candidate).is_ok())
        .ok_or_else(|| AppError::Other("could not create unique backup directory".into()))?;

    // Never copy plaintext or quarantined secret files. Omit SQLite WAL
    // sidecars: the checkpoint makes app.db self-contained, and the DB mutex
    // prevents a concurrent writer while this copy runs.
    let skip = vec![
        src.join("secrets.json"),
        src.join("secrets.json.corrupt"),
        src.join("app.db-wal"),
        src.join("app.db-shm"),
    ];
    let (bytes, file_count) = copy_dir_recursive(&src, &backup_dir, &skip)?;
    drop(conn);

    Ok(BackupResult {
        backup_path: backup_dir.to_string_lossy().into_owned(),
        bytes,
        file_count,
    })
}

#[derive(Debug, Serialize)]
pub struct PostUpdateResult {
    /// `true` only on the first boot after a version bump. Subsequent
    /// boots return false (the marker has already been written).
    pub was_post_update_boot: bool,
    /// Empty when nothing's wrong, populated with a warning the UI
    /// should surface as a toast otherwise.
    pub warning: String,
}

/// Detect "first boot after an update" and verify what we can. Should
/// be called by the frontend bootstrap (App.tsx) after the app has
/// loaded, to verify the update was applied cleanly.
#[tauri::command]
pub async fn post_update_finalize(state: State<'_, AppState>) -> AppResult<PostUpdateResult> {
    let current = env!("CARGO_PKG_VERSION").to_string();
    let conn = state.db.lock().await;
    let last_known = crate::db::meta_get(&conn, META_LAST_VERSION)?.unwrap_or_default();
    drop(conn);

    if last_known == current {
        return Ok(PostUpdateResult {
            was_post_update_boot: false,
            warning: String::new(),
        });
    }

    // Log the post-update audit row + crash-log summary BEFORE we mark
    // the version "seen" — that way a crash mid-verification gets one
    // more retry on the next boot. The previous implementation marked
    // the version first, so any panic between "mark" and "verify"
    // permanently hid the warning. Cap the retries to 2 with a
    // separate `META_POST_UPDATE_RETRIES` counter so an issue that
    // genuinely went missing doesn't loop the warning forever.
    let retries: i64 = {
        let conn = state.db.lock().await;
        crate::db::meta_get(&conn, META_POST_UPDATE_RETRIES)?
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    };

    // First-boot summary into the crash log so users reporting issues
    // have an audit trail of "I updated v0.1.0 → v0.1.1 at <date>".
    let summary = if last_known.is_empty() {
        format!("first launch at v{current}")
    } else {
        format!("post-update boot: v{last_known} → v{current}")
    };
    crate::crash_log::record("update", &summary);

    // DB integrity check. Run it before marking the version "seen" so a
    // corrupt database after an update surfaces a warning instead of
    // silently finalizing. Cap retries at 2: a genuinely corrupt DB
    // shouldn't re-run this check (and re-log the warning) forever.
    const MAX_RETRIES: i64 = 2;
    let integrity: String = {
        let conn = state.db.lock().await;
        conn.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap_or_else(|e| format!("query failed: {e}"))
    };

    let warning = if integrity != "ok" {
        let msg = format!(
            "Post-update database integrity check failed: {integrity}. \
             Consider restoring from a backup."
        );
        tracing::warn!(%integrity, retries, "post_update_finalize integrity check failed");
        crate::crash_log::record("update", &msg);
        msg
    } else {
        String::new()
    };

    let now = chrono::Utc::now().timestamp();
    if integrity != "ok" && retries < MAX_RETRIES {
        // Don't finalize yet — leave META_LAST_VERSION unset so the next
        // boot retries the check, but bump the counter so it eventually
        // gives up rather than looping forever.
        let conn = state.db.lock().await;
        crate::db::meta_set(&conn, META_POST_UPDATE_RETRIES, &(retries + 1).to_string())?;
    } else {
        // Either the check passed, or we've exhausted retries on a
        // persistently failing check — mark the version "seen" either way
        // so the app doesn't get stuck re-checking forever.
        let conn = state.db.lock().await;
        crate::db::meta_set(&conn, META_LAST_VERSION, &current)?;
        crate::db::meta_set(&conn, META_LAST_UPDATE_AT, &now.to_string())?;
        crate::db::meta_set(&conn, META_POST_UPDATE_RETRIES, "0")?;
    }

    Ok(PostUpdateResult {
        was_post_update_boot: true,
        warning,
    })
}

// ── helpers ───────────────────────────────────────────────────────

fn dir_size(p: &std::path::Path) -> u64 {
    let mut total = 0u64;
    let walk = |path: &std::path::Path, total: &mut u64| {
        if let Ok(meta) = std::fs::metadata(path) {
            if meta.is_file() {
                *total += meta.len();
            }
        }
    };
    fn recurse(p: &std::path::Path, acc: &mut u64) {
        let Ok(entries) = std::fs::read_dir(p) else {
            return;
        };
        for e in entries.flatten() {
            let path = e.path();
            if let Ok(ft) = e.file_type() {
                if ft.is_dir() {
                    recurse(&path, acc);
                } else if let Ok(meta) = e.metadata() {
                    *acc += meta.len();
                }
            }
        }
    }
    walk(p, &mut total);
    recurse(p, &mut total);
    total
}

fn copy_dir_recursive(
    src: &std::path::Path,
    dst: &std::path::Path,
    skip: &[PathBuf],
) -> AppResult<(u64, u32)> {
    let mut bytes = 0u64;
    let mut count = 0u32;
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        if skip.contains(&from) {
            continue;
        }
        let to = dst.join(entry.file_name());
        let ft = entry.file_type()?;
        // Skip symlinks entirely. Without this check, a symlink pointing
        // back into the app-data dir (or one of its ancestors) would
        // make `read_dir` recurse infinitely until the stack blew, and
        // a symlink to anywhere outside would silently exfiltrate
        // unrelated files into the backup.
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            let (b, c) = copy_dir_recursive(&from, &to, skip)?;
            bytes += b;
            count += c;
        } else {
            std::fs::copy(&from, &to)?;
            bytes += entry.metadata()?.len();
            count += 1;
        }
    }
    Ok((bytes, count))
}

#[cfg(test)]
mod tests {
    use super::copy_dir_recursive;

    #[test]
    fn copy_dir_preserves_files_and_counts() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join("a.txt"), b"hello").unwrap();
        std::fs::create_dir_all(src.path().join("sub")).unwrap();
        std::fs::write(src.path().join("sub/b.txt"), b"world!").unwrap();

        let (bytes, count) = copy_dir_recursive(src.path(), dst.path(), &[]).unwrap();
        assert_eq!(count, 2);
        assert_eq!(bytes, 11);
        assert!(dst.path().join("a.txt").exists());
        assert!(dst.path().join("sub/b.txt").exists());
    }
}
