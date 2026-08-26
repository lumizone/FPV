use serde::Serialize;
use std::path::Path;
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct SavedSceneImage {
    pub message_id: String,
    pub path: String,
}

fn count_scene_images(dir: &Path, replacing_message_id: &str) -> usize {
    std::fs::read_dir(dir)
        .ok()
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| {
                    entry.path().extension().and_then(|ext| ext.to_str()) == Some("png")
                        && entry.file_name()
                            != std::ffi::OsString::from(format!("{replacing_message_id}.png"))
                })
                .count()
        })
        .unwrap_or(0)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn scene_image_save(
    state: State<'_, AppState>,
    session_id: String,
    message_id: String,
    image_b64: String,
    metadata: Option<String>,
) -> AppResult<String> {
    crate::storage::fs_safe_id(&session_id)?;
    crate::storage::fs_safe_id(&message_id)?;
    {
        let conn = state.db.lock().await;
        let belongs: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM messages WHERE id = ?1 AND session_id = ?2)",
            rusqlite::params![message_id, session_id],
            |row| row.get(0),
        )?;
        if !belongs {
            return Err(AppError::Config(
                "message does not belong to session".into(),
            ));
        }
    }
    if let Ok(base) = crate::storage::app_data_dir() {
        let dir = base.join("images").join("scenes").join(&session_id);
        let existing = count_scene_images(&dir, &message_id);
        if existing >= 100 {
            return Err(AppError::Config(
                "scene image limit reached for this session (100)".into(),
            ));
        }
    }
    if image_b64.len() > 34 * 1024 * 1024 {
        return Err(AppError::Config("scene image payload is too large".into()));
    }
    let encoded = image_b64
        .strip_prefix("data:image/png;base64,")
        .unwrap_or(&image_b64);
    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)
        .map_err(|_| AppError::Config("invalid scene image data".into()))?;
    if bytes.is_empty() || bytes.len() > 25 * 1024 * 1024 {
        return Err(AppError::Config(
            "scene image must be between 1 byte and 25 MB".into(),
        ));
    }
    crate::image::validate_png_payload(&bytes, 25 * 1024 * 1024)?;
    // Validate metadata BEFORE touching the disk — a bad metadata payload
    // must not leave a written image behind after an error return.
    if let Some(metadata) = metadata.as_ref() {
        if metadata.len() > 64 * 1024 {
            return Err(AppError::Config("scene metadata is too large".into()));
        }
        serde_json::from_str::<serde_json::Value>(metadata)
            .map_err(|_| AppError::Config("scene metadata must be valid JSON".into()))?;
    }
    let dir = crate::storage::images_dir()?
        .join("scenes")
        .join(&session_id);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{message_id}.png"));
    let partial = dir.join(format!("{message_id}.png.part"));
    std::fs::write(&partial, bytes)?;
    std::fs::rename(&partial, &path)?;
    if let Some(metadata) = metadata {
        let metadata_path = dir.join(format!("{message_id}.json"));
        let metadata_partial = dir.join(format!("{message_id}.json.part"));
        std::fs::write(&metadata_partial, metadata)?;
        std::fs::rename(metadata_partial, metadata_path)?;
    }
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn scene_image_list(
    state: State<'_, AppState>,
    session_id: String,
) -> AppResult<Vec<SavedSceneImage>> {
    crate::storage::fs_safe_id(&session_id)?;
    let dir = crate::storage::images_dir()?
        .join("scenes")
        .join(&session_id);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let messages = {
        let conn = state.db.lock().await;
        crate::story::db::list_messages(&conn, &session_id).unwrap_or_default()
    };
    let mut images = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("png") {
            continue;
        }
        if let Some(raw_id) = path.file_stem().and_then(|name| name.to_str()) {
            let message_id = raw_id
                .parse::<usize>()
                .ok()
                .and_then(|index| messages.get(index).map(|message| message.id.clone()))
                .unwrap_or_else(|| raw_id.to_string());
            if message_id != raw_id {
                let migrated = path.with_file_name(format!("{message_id}.png"));
                if !migrated.exists() {
                    std::fs::rename(&path, &migrated)?;
                }
                let legacy_metadata = path.with_extension("json");
                let migrated_metadata = migrated.with_extension("json");
                if legacy_metadata.exists() && !migrated_metadata.exists() {
                    std::fs::rename(legacy_metadata, migrated_metadata)?;
                }
            }
            if crate::storage::fs_safe_id(&message_id).is_ok() {
                let resolved_path = path.with_file_name(format!("{message_id}.png"));
                images.push(SavedSceneImage {
                    message_id,
                    path: resolved_path.to_string_lossy().into_owned(),
                });
            }
        }
    }
    images.sort_by(|left, right| left.message_id.cmp(&right.message_id));
    Ok(images)
}

#[cfg(test)]
mod tests {
    use super::count_scene_images;

    #[test]
    fn scene_image_limit_allows_replacing_an_existing_image() {
        let dir = tempfile::tempdir().unwrap();
        for index in 0..100 {
            std::fs::write(dir.path().join(format!("message-{index}.png")), []).unwrap();
        }

        assert_eq!(count_scene_images(dir.path(), "message-42"), 99);
        assert_eq!(count_scene_images(dir.path(), "new-message"), 100);
    }
}
