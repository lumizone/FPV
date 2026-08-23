use std::path::PathBuf;

use crate::error::{AppError, AppResult};

/// Validate that `id` is safe to embed in a filesystem path. Accepts
/// ASCII alphanumerics plus `-`, `_`, `.` and rejects anything with a
/// path separator, NUL, or `..` so a crafted id — e.g. one imported
/// from a malicious export JSON — can't traverse out of the app-data
/// directory. Returns the id unchanged when safe.
pub fn fs_safe_id(id: &str) -> AppResult<&str> {
    let safe = !id.is_empty()
        && id != "."
        && id != ".."
        && !id.contains("..")
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if safe {
        Ok(id)
    } else {
        Err(AppError::Config(format!("unsafe path id: {id:?}")))
    }
}

pub fn app_data_dir() -> AppResult<PathBuf> {
    let base = dirs::data_dir()
        .ok_or_else(|| AppError::Config("could not resolve user data dir".into()))?;
    let dir = base.join("com.lumizone.fpvdesktop");
    if !dir.exists() {
        std::fs::create_dir_all(&dir)?;
    }
    Ok(dir)
}

pub fn db_path() -> AppResult<PathBuf> {
    Ok(app_data_dir()?.join("app.db"))
}

/// Where chat images live — one PNG per message (the character's
/// selfies and the photos the user attaches), keyed by a UUID.
pub fn images_dir() -> AppResult<PathBuf> {
    let p = app_data_dir()?.join("images");
    if !p.exists() {
        std::fs::create_dir_all(&p)?;
    }
    Ok(p)
}

/// Private scratch space for short-lived renderer inputs and outputs. It is
/// app-owned rather than the shared system temp directory because references
/// can contain user portraits or other sensitive material.
pub fn renderer_temp_dir() -> AppResult<PathBuf> {
    let p = app_data_dir()?.join("renderer-temp");
    if !p.exists() {
        std::fs::create_dir_all(&p)?;
    }
    Ok(p)
}

/// App-owned Ollama model store. Before this, the bundled Ollama used
/// its default `~/.ollama/models`, leaving multi-GB blobs in the user's
/// home after the app was trashed (and risking collision with a
/// system-wide Ollama). We now point `OLLAMA_MODELS` here so models live
/// inside the app's data dir and are removed together with it.
pub fn ollama_models_dir() -> AppResult<PathBuf> {
    let p = app_data_dir()?.join("ollama").join("models");
    if !p.exists() {
        std::fs::create_dir_all(&p)?;
    }
    Ok(p)
}

pub fn audit_log_path() -> AppResult<PathBuf> {
    Ok(app_data_dir()?.join("audit.log"))
}

/// Append-only crash log. Both Rust panics and renderer-side React
/// errors land here so a user reporting "the app died" can paste a
/// single file. Rotation is handled lazily in `crash_log::record` —
/// at 1 MB we rename to `crash.log.old` and start fresh.
pub fn crash_log_path() -> AppResult<PathBuf> {
    Ok(app_data_dir()?.join("crash.log"))
}

#[cfg(test)]
mod fs_safe_id_tests {
    use super::fs_safe_id;

    #[test]
    fn accepts_generated_id_shapes() {
        assert!(fs_safe_id("550e8400-e29b-41d4-a716-446655440000").is_ok());
        assert!(fs_safe_id("char-1720000000000").is_ok());
        assert!(fs_safe_id("default").is_ok());
    }

    #[test]
    fn rejects_path_traversal_and_separators() {
        for bad in [
            "",
            ".",
            "..",
            "../../evil",
            "a/b",
            "a\\b",
            "..%2f",
            "id\0null",
            "foo/../bar",
        ] {
            assert!(fs_safe_id(bad).is_err(), "{bad:?} must be rejected");
        }
    }
}
