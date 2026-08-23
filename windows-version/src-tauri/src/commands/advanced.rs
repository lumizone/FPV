//! AdvancedTab backend: open logs, export conversations, reset all data.

use tauri::State;

use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::storage;

/// Returns the path to audit.log so the frontend can open it.
#[tauri::command]
pub fn logs_path() -> AppResult<String> {
    let path = storage::audit_log_path()?;
    // The audit log is only written on MCP tool calls, so a user who's
    // never used one has no file — and `openPath()` on a missing path
    // errors ("Open Logs" did nothing but show an error). Create an
    // empty log on first open so the button always works.
    if !path.exists() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, b"");
    }
    Ok(path.to_string_lossy().to_string())
}

/// Forward a renderer-side React error to the local crash log.
/// Called from the React `ErrorBoundary` after `componentDidCatch`.
/// Always succeeds (returns nothing) — we never want a logging
/// failure to mask the original UI error.
#[tauri::command(rename_all = "snake_case")]
pub fn log_renderer_crash(message: String, stack: String, component_stack: String) {
    let body = format!(
        "renderer error: {message}\n\nstack:\n{stack}\n\ncomponent stack:\n{component_stack}"
    );
    crate::crash_log::record("renderer", &body);
}

/// Wipe ALL user data: DB + app data files.
/// The frontend must confirm before calling this.
///
/// This is an explicit full local-data reset. It removes story data,
/// generated assets, logs, model weights and file-backed credentials.
#[tauri::command]
pub async fn reset_all_data(state: State<'_, AppState>) -> AppResult<()> {
    // Stop the app-owned daemon before removing its model directory. Otherwise
    // an active inference process can recreate files after the reset returns.
    state.sidecars.shutdown_all(&state.ollama).await?;
    let db_path = storage::db_path()?;

    // Hold the DB mutex across the entire reset. Earlier this dropped
    // the lock between checkpoint and file delete, leaving a window in
    // which any concurrent Tauri command could re-acquire the
    // connection and write to a path we were about to unlink — which
    // SQLite would then silently re-create as an empty file, restoring
    // the schema we'd just deleted.
    let mut conn_guard = state.db.lock().await;
    let _ = conn_guard.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");

    // Delete all three SQLite files so nothing is left behind.
    for suffix in &["", "-wal", "-shm"] {
        let mut p = db_path.clone();
        let name = format!(
            "{}{}",
            p.file_name().unwrap_or_default().to_string_lossy(),
            suffix
        );
        p.set_file_name(name);
        if p.exists() {
            let _ = std::fs::remove_file(&p);
        }
    }

    // Re-open the connection in-place against the (now missing) path so
    // SQLite recreates a fresh DB with the current migrations applied.
    // Without this, the Connection inside the Arc still points at a
    // deleted inode and the next command would get cryptic errors.
    *conn_guard = crate::db::open_connection()?;
    drop(conn_guard);

    // Delete legacy companion-AI character data, if any survives from
    // before the FPV rebrand.
    let chars_dir = storage::app_data_dir()?.join("characters");
    if chars_dir.exists() {
        std::fs::remove_dir_all(&chars_dir)?;
    }

    // Delete legacy voice clones (removed in Phase 1, clean up any
    // old directory that may still exist on disk). Skip entirely if we
    // can't resolve the home dir — unwrap_or_default() would otherwise
    // resolve the join relative to the process's CWD, and remove_dir_all
    // on an attacker- or CWD-controlled "./.fpvdesktop/voice_clones"
    // is not a cleanup anyone asked for.
    if let Some(home) = dirs::home_dir() {
        let clone_dir = home.join(".fpvdesktop").join("voice_clones");
        if clone_dir.exists() {
            let _ = std::fs::remove_dir_all(&clone_dir);
        }
    }

    // Delete the app-owned Ollama model store (multi-GB blobs). Safe to
    // nuke now that we relocated it under our data dir via OLLAMA_MODELS
    // — it's no longer the shared ~/.ollama. The sidecar is killed on
    // app exit; a re-pull repopulates this on next use.
    if let Ok(models_dir) = storage::app_data_dir() {
        let ollama_dir = models_dir.join("ollama");
        if ollama_dir.exists() {
            let _ = std::fs::remove_dir_all(&ollama_dir);
        }
    }

    let app_data = storage::app_data_dir()?;
    for dir in ["covers", "images", "portraits"] {
        let path = app_data.join(dir);
        if path.exists() {
            std::fs::remove_dir_all(path)?;
        }
    }
    for file in ["audit.log", "crash.log", "crash.log.old"] {
        let path = app_data.join(file);
        if path.exists() {
            let _ = std::fs::remove_file(path);
        }
    }
    crate::secret_store::clear_all()?;

    Ok(())
}

/// Everything a bug report needs, in one file the user can attach.
///
/// Written because "send me your logs" was not answerable: the tracing
/// stream had no file (see `log_file`), and the state that explains a
/// voice bug — which whisper model is installed, whether the Supertonic
/// weights are there, the hardware tier that picks the decoding effort,
/// whether cloud STT is on — is spread across the database, the
/// filesystem and the build flags.
///
/// **Contains no secrets.** Not the license key, not a BYOK API key,
/// not the Telegram token, not a single line of conversation. Only
/// whether such a thing is configured. That rule is what makes this
/// safe to tell someone to attach to an email, so keep it: if you add a
/// field here, add it as a boolean unless it is genuinely inert.
#[tauri::command]
pub async fn diagnostics_write(
    state: tauri::State<'_, crate::state::AppState>,
) -> AppResult<String> {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(64 * 1024);
    let ts = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
    let _ = writeln!(out, "FPV diagnostics · {ts}");
    let _ = writeln!(
        out,
        "(no keys, no tokens, no conversation text — audio device names ARE included)\n"
    );

    let _ = writeln!(out, "--- build ---");
    let _ = writeln!(out, "version           {}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(out, "distribution      direct");
    let _ = writeln!(out, "debug assertions  {}", cfg!(debug_assertions));

    let _ = writeln!(out, "\n--- machine ---");
    let _ = writeln!(out, "chip              {}", state.hardware.chip);
    let _ = writeln!(out, "ram               {} GB", state.hardware.ram_gb);
    let _ = writeln!(out, "tier              {:?}", state.hardware.tier);
    let _ = writeln!(
        out,
        "os                {}",
        std::process::Command::new("sw_vers")
            .arg("-productVersion")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "unknown".into())
    );

    // Voice/speech diagnostics removed with voice subsystem

    // Settings that change behaviour. Values only where the value is
    // inert — a device NAME is fine, a key is not.
    let _ = writeln!(out, "\n--- settings ---");
    {
        let conn = state.db.lock().await;
        for key in [
            "language",
            "offlineMode",
            "lowPowerMode",
            "reasoning",
            "contextSize",
            "device",
            "theme",
            "voice_silence_ms",
            "voice_barge_in",
            "voice_style",
            "voice_speed",
            "voice_pitch",
            "voice_mood_expression",
            "voice_cloud_stt",
            "voice_input_device",
            "voice_output_device",
            "image_local_model",
        ] {
            let v = crate::db::meta_get(&conn, key)
                .ok()
                .flatten()
                .unwrap_or_else(|| "(unset)".into());
            let _ = writeln!(out, "{key:<18}{v}");
        }
        let _ = writeln!(
            out,
            "\nchat model        {}",
            crate::db::meta_get(&conn, "chat_model")
                .ok()
                .flatten()
                .unwrap_or_else(|| "(hardware default)".into())
        );
    }

    let _ = writeln!(out, "\n--- crash log (tail) ---");
    let crash = crate::storage::crash_log_path()
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default();
    let _ = writeln!(
        out,
        "{}",
        if crash.trim().is_empty() {
            "(empty — no crashes recorded)".to_string()
        } else {
            crash
                .chars()
                .rev()
                .take(20_000)
                .collect::<String>()
                .chars()
                .rev()
                .collect()
        }
    );

    let _ = writeln!(out, "\n--- app log (tail) ---");
    let _ = writeln!(out, "{}", crate::log_file::tail(400_000));

    let dir = crate::storage::app_data_dir()?;
    let path = dir.join(format!(
        "diagnostics-{}.txt",
        chrono::Utc::now().format("%Y%m%d-%H%M%S")
    ));
    std::fs::write(&path, out.as_bytes())?;
    tracing::info!(path = %path.display(), "diagnostics written");
    Ok(path.to_string_lossy().to_string())
}

/// The tracing log itself, for "Reveal in Finder".
#[tauri::command]
pub fn app_log_path() -> AppResult<String> {
    let path =
        crate::log_file::path().ok_or_else(|| AppError::Other("no app-data directory".into()))?;
    if !path.exists() {
        // Same missing-parent bug `logs_path` fixed above: if the log
        // directory hasn't been created yet, the write fails, the error
        // is discarded, and "Reveal in Finder" gets a path that doesn't
        // exist.
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, b"");
    }
    Ok(path.to_string_lossy().to_string())
}
