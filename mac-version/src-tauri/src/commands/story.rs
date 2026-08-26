use tauri::State;

use crate::error::AppResult;
use crate::state::AppState;
use crate::story::db::{
    self, CodexEntry, Message, NewWorld, PinnedCanon, SessionSummary, StoryState, VisualBible,
    World,
};

fn validate_text(field: &str, value: &str, max: usize) -> AppResult<()> {
    let len = value.chars().count();
    if len == 0 || len > max {
        return Err(crate::error::AppError::Config(format!(
            "{field} must contain 1-{max} characters"
        )));
    }
    Ok(())
}

fn validate_world(w: &NewWorld) -> AppResult<()> {
    validate_text("world name", &w.name, 200)?;
    validate_text("world genre", &w.genre, 80)?;
    validate_text("world system prompt", &w.system_prompt, 30_000)?;
    if let Some(description) = &w.description {
        if description.chars().count() > 10_000 {
            return Err(crate::error::AppError::Config(
                "world description is too long".into(),
            ));
        }
    }
    Ok(())
}

fn validate_privacy_mode(mode: &str) -> AppResult<()> {
    if matches!(mode, "local_only" | "cloud_allowed") {
        Ok(())
    } else {
        Err(crate::error::AppError::Config(
            "privacy mode must be local_only or cloud_allowed".into(),
        ))
    }
}

fn validate_visual_bible(visual_bible: &VisualBible) -> AppResult<()> {
    for (field, value, max) in [
        ("visual style", &visual_bible.style, 2_000),
        ("visual palette", &visual_bible.palette, 1_000),
        ("character anchors", &visual_bible.character_anchors, 8_000),
        ("location anchors", &visual_bible.location_anchors, 8_000),
        ("negative prompt", &visual_bible.negative_prompt, 4_000),
    ] {
        if value.chars().count() > max {
            return Err(crate::error::AppError::Config(format!(
                "{field} is too long"
            )));
        }
    }
    Ok(())
}

fn validate_codex_entry(entry: &db::CodexSaveEntry) -> AppResult<()> {
    if let Some(id) = &entry.id {
        crate::storage::fs_safe_id(id)?;
    }
    validate_text("codex title", &entry.title, 200)?;
    validate_text("codex content", &entry.content, 20_000)?;
    if entry.triggers.chars().count() > 4_000 {
        return Err(crate::error::AppError::Config(
            "codex triggers are too long".into(),
        ));
    }
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn world_create(state: State<'_, AppState>, w: NewWorld) -> AppResult<String> {
    validate_world(&w)?;
    let conn = state.db.lock().await;
    db::create_world(&conn, &w).map_err(Into::into)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn world_list(state: State<'_, AppState>) -> AppResult<Vec<World>> {
    let conn = state.db.lock().await;
    db::list_worlds(&conn).map_err(Into::into)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn world_get(state: State<'_, AppState>, id: String) -> AppResult<World> {
    let conn = state.db.lock().await;
    db::get_world(&conn, &id).map_err(Into::into)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn world_delete(state: State<'_, AppState>, id: String) -> AppResult<()> {
    crate::storage::fs_safe_id(&id)?;
    let (sessions, cover) = {
        let conn = state.db.lock().await;
        let sessions = db::list_sessions_for_world(&conn, &id)?
            .into_iter()
            .map(|session| session.id)
            .collect::<Vec<_>>();
        let cover = db::get_world(&conn, &id)?.cover_image_path;
        db::delete_world(&conn, &id)?;
        (sessions, cover)
    };
    cleanup_session_assets(&sessions).await;
    if let Some(cover) = cover {
        cleanup_owned_cover(&cover).await;
    }
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn world_save(
    state: State<'_, AppState>,
    id: Option<String>,
    w: NewWorld,
    visual_bible: VisualBible,
    codex_entries: Vec<db::CodexSaveEntry>,
    removed_codex_ids: Vec<String>,
    privacy_mode: Option<String>,
) -> AppResult<String> {
    if let Some(existing_id) = &id {
        crate::storage::fs_safe_id(existing_id)?;
    }
    if removed_codex_ids.len() > 100 || codex_entries.len() > 100 {
        return Err(crate::error::AppError::Config(
            "a story can contain at most 100 Codex entries".into(),
        ));
    }
    for codex_id in &removed_codex_ids {
        crate::storage::fs_safe_id(codex_id)?;
    }
    validate_world(&w)?;
    let privacy_mode = privacy_mode.unwrap_or_else(|| "local_only".into());
    validate_privacy_mode(&privacy_mode)?;
    validate_visual_bible(&visual_bible)?;
    for entry in &codex_entries {
        validate_codex_entry(entry)?;
    }

    let conn = state.db.lock().await;
    db::save_world(
        &conn,
        id.as_deref(),
        &w,
        &visual_bible,
        &codex_entries,
        &removed_codex_ids,
        &privacy_mode,
    )
    .map_err(Into::into)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn session_create(state: State<'_, AppState>, world_id: String) -> AppResult<String> {
    crate::storage::fs_safe_id(&world_id)?;
    let conn = state.db.lock().await;
    db::create_session(&conn, &world_id).map_err(Into::into)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn session_delete(state: State<'_, AppState>, session_id: String) -> AppResult<()> {
    crate::storage::fs_safe_id(&session_id)?;
    {
        let conn = state.db.lock().await;
        db::delete_session(&conn, &session_id)?;
    }
    cleanup_session_assets(&[session_id]).await;
    Ok(())
}

async fn cleanup_session_assets(session_ids: &[String]) {
    if let Ok(base) = crate::storage::app_data_dir() {
        for session_id in session_ids {
            let _ = tokio::fs::remove_dir_all(base.join("images").join("scenes").join(session_id))
                .await;
        }
    }
}

fn is_owned_cover_path(path: &std::path::Path, covers_dir: &std::path::Path) -> bool {
    let Ok(covers_dir) = covers_dir.canonicalize() else {
        return false;
    };
    let Ok(path) = path.canonicalize() else {
        return false;
    };
    path.starts_with(covers_dir)
}

pub(crate) async fn cleanup_owned_cover(path: &str) {
    let Ok(base) = crate::storage::app_data_dir() else {
        return;
    };
    let candidate = std::path::Path::new(path);
    let covers_dir = base.join("covers");
    if is_owned_cover_path(candidate, &covers_dir) {
        let _ = tokio::fs::remove_file(candidate).await;
    }
}

#[tauri::command(rename_all = "snake_case")]
pub async fn session_branch(
    state: State<'_, AppState>,
    session_id: String,
    label: String,
) -> AppResult<String> {
    crate::storage::fs_safe_id(&session_id)?;
    validate_text("branch name", label.trim(), 120)?;
    let conn = state.db.lock().await;
    let new_id = db::branch_session(&conn, &session_id, label.trim())
        .map_err(crate::error::AppError::from)?;
    drop(conn);
    if let Err(error) = copy_session_assets(&session_id, &new_id).await {
        let conn = state.db.lock().await;
        let _ = db::delete_session(&conn, &new_id);
        drop(conn);
        cleanup_session_assets(&[new_id]).await;
        return Err(error);
    }
    Ok(new_id)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn session_checkpoint(
    state: State<'_, AppState>,
    session_id: String,
    label: String,
) -> AppResult<String> {
    crate::storage::fs_safe_id(&session_id)?;
    validate_text("checkpoint name", label.trim(), 120)?;
    let conn = state.db.lock().await;
    let new_id = db::checkpoint_session(&conn, &session_id, label.trim())
        .map_err(crate::error::AppError::from)?;
    drop(conn);
    if let Err(error) = copy_session_assets(&session_id, &new_id).await {
        let conn = state.db.lock().await;
        let _ = db::delete_session(&conn, &new_id);
        drop(conn);
        cleanup_session_assets(&[new_id]).await;
        return Err(error);
    }
    Ok(new_id)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn session_fork_from_message(
    state: State<'_, AppState>,
    session_id: String,
    message_id: String,
    label: String,
) -> AppResult<String> {
    crate::storage::fs_safe_id(&session_id)?;
    crate::storage::fs_safe_id(&message_id)?;
    validate_text("path name", label.trim(), 120)?;
    let conn = state.db.lock().await;
    let source_messages = db::list_messages(&conn, &session_id)?;
    let cutoff = source_messages
        .iter()
        .position(|message| message.id == message_id)
        .ok_or_else(|| {
            crate::error::AppError::Config("message does not belong to session".into())
        })?;
    let new_id = db::fork_session_at_message(&conn, &session_id, &message_id, label.trim())?;
    let target_messages = db::list_messages(&conn, &new_id)?;
    drop(conn);
    if let Err(error) = copy_fork_assets(
        &session_id,
        &new_id,
        &source_messages[..=cutoff],
        &target_messages,
    )
    .await
    {
        let conn = state.db.lock().await;
        let _ = db::delete_session(&conn, &new_id);
        drop(conn);
        cleanup_session_assets(&[new_id]).await;
        return Err(error);
    }
    Ok(new_id)
}

async fn copy_session_assets(source_session: &str, target_session: &str) -> AppResult<()> {
    let base = crate::storage::app_data_dir()?;
    let source = base.join("images").join("scenes").join(source_session);
    if !source.exists() {
        return Ok(());
    }
    let target = base.join("images").join("scenes").join(target_session);
    tokio::fs::create_dir_all(&target).await?;
    let mut entries = tokio::fs::read_dir(source).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.is_file() {
            if let Some(name) = path.file_name() {
                tokio::fs::copy(&path, target.join(name)).await?;
            }
        }
    }
    Ok(())
}

async fn copy_fork_assets(
    source_session: &str,
    target_session: &str,
    source_messages: &[Message],
    target_messages: &[Message],
) -> AppResult<()> {
    let base = crate::storage::app_data_dir()?;
    let source = base.join("images").join("scenes").join(source_session);
    if !source.exists() {
        return Ok(());
    }
    let target = base.join("images").join("scenes").join(target_session);
    tokio::fs::create_dir_all(&target).await?;
    for (source_message, target_message) in source_messages.iter().zip(target_messages) {
        for extension in ["png", "json"] {
            let from = source.join(format!("{}.{}", source_message.id, extension));
            if from.exists() {
                tokio::fs::copy(
                    &from,
                    target.join(format!("{}.{}", target_message.id, extension)),
                )
                .await?;
            }
        }
    }
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn session_list_messages(
    state: State<'_, AppState>,
    session_id: String,
) -> AppResult<Vec<Message>> {
    let conn = state.db.lock().await;
    db::list_messages(&conn, &session_id).map_err(Into::into)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn session_get_summary(
    state: State<'_, AppState>,
    session_id: String,
) -> AppResult<String> {
    let conn = state.db.lock().await;
    db::get_session_summary(&conn, &session_id).map_err(Into::into)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn session_list_for_world(
    state: State<'_, AppState>,
    world_id: String,
) -> AppResult<Vec<SessionSummary>> {
    let conn = state.db.lock().await;
    db::list_sessions_for_world(&conn, &world_id).map_err(Into::into)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn session_append_message(
    state: State<'_, AppState>,
    session_id: String,
    role: String,
    content: String,
) -> AppResult<String> {
    crate::storage::fs_safe_id(&session_id)?;
    if !matches!(role.as_str(), "user" | "narrator") {
        return Err(crate::error::AppError::Config(
            "invalid message role".into(),
        ));
    }
    validate_text("message", &content, 50_000)?;
    let conn = state.db.lock().await;
    db::append_message(&conn, &session_id, &role, &content).map_err(Into::into)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn message_update(
    state: State<'_, AppState>,
    id: String,
    content: String,
) -> AppResult<()> {
    let content = content.trim();
    if content.is_empty() {
        return Err(crate::error::AppError::Config(
            "message cannot be empty".into(),
        ));
    }
    if content.len() > 50_000 {
        return Err(crate::error::AppError::Config("message is too long".into()));
    }
    let conn = state.db.lock().await;
    db::update_message(&conn, &id, content).map_err(Into::into)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn message_replace_content(
    state: State<'_, AppState>,
    session_id: String,
    message_id: String,
    content: String,
) -> AppResult<String> {
    crate::storage::fs_safe_id(&session_id)?;
    crate::storage::fs_safe_id(&message_id)?;
    let content = content.trim();
    validate_text("message", content, 50_000)?;
    let conn = state.db.lock().await;
    db::replace_message_content(&conn, &session_id, &message_id, content)?;
    Ok(message_id)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn narration_prepare_regeneration(
    state: State<'_, AppState>,
    session_id: String,
    message_id: String,
) -> AppResult<()> {
    crate::storage::fs_safe_id(&session_id)?;
    crate::storage::fs_safe_id(&message_id)?;
    let conn = state.db.lock().await;
    db::clear_regeneration_derived_data(&conn, &session_id, &message_id).map_err(Into::into)
}

/// Remove the last turn of a session: deletes the newest message and its
/// scene image, restores the story state from before that turn (V29
/// history), and drops the corresponding memory record.
#[tauri::command(rename_all = "snake_case")]
pub async fn session_undo(state: State<'_, AppState>, session_id: String) -> AppResult<()> {
    crate::storage::fs_safe_id(&session_id)?;
    let conn = state.db.lock().await;
    let deleted = db::undo_last_turn(&conn, &session_id)?;
    drop(conn);
    if let Some(message_id) = deleted {
        // Best-effort cleanup of the turn's scene image (if any).
        let dir = crate::storage::images_dir()?
            .join("scenes")
            .join(&session_id);
        for ext in ["png", "json"] {
            let _ = std::fs::remove_file(dir.join(format!("{message_id}.{ext}")));
        }
    }
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn story_state_get(
    state: State<'_, AppState>,
    session_id: String,
) -> AppResult<StoryState> {
    let conn = state.db.lock().await;
    db::get_story_state(&conn, &session_id).map_err(Into::into)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn story_state_set(
    state: State<'_, AppState>,
    session_id: String,
    story_state: StoryState,
) -> AppResult<()> {
    let conn = state.db.lock().await;
    db::set_story_state(&conn, &session_id, &story_state).map_err(Into::into)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn world_visual_bible_get(
    state: State<'_, AppState>,
    world_id: String,
) -> AppResult<VisualBible> {
    crate::storage::fs_safe_id(&world_id)?;
    let conn = state.db.lock().await;
    db::get_visual_bible(&conn, &world_id).map_err(Into::into)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn world_visual_bible_set(
    state: State<'_, AppState>,
    world_id: String,
    visual_bible: VisualBible,
) -> AppResult<()> {
    crate::storage::fs_safe_id(&world_id)?;
    validate_visual_bible(&visual_bible)?;
    let conn = state.db.lock().await;
    db::set_visual_bible(&conn, &world_id, &visual_bible).map_err(Into::into)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn story_pinned_canon_list(
    state: State<'_, AppState>,
    session_id: String,
) -> AppResult<Vec<PinnedCanon>> {
    let conn = state.db.lock().await;
    db::list_pinned_canon(&conn, &session_id).map_err(Into::into)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn story_pinned_canon_add(
    state: State<'_, AppState>,
    session_id: String,
    content: String,
) -> AppResult<String> {
    crate::storage::fs_safe_id(&session_id)?;
    validate_text("pinned canon", content.trim(), 2_000)?;
    let conn = state.db.lock().await;
    db::add_pinned_canon(&conn, &session_id, content.trim()).map_err(Into::into)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn story_pinned_canon_delete(state: State<'_, AppState>, id: String) -> AppResult<()> {
    crate::storage::fs_safe_id(&id)?;
    let conn = state.db.lock().await;
    db::delete_pinned_canon(&conn, &id).map_err(Into::into)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn session_update_summary(
    state: State<'_, AppState>,
    session_id: String,
    summary: String,
) -> AppResult<()> {
    let conn = state.db.lock().await;
    db::update_summary(&conn, &session_id, &summary).map_err(Into::into)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn codex_upsert(
    state: State<'_, AppState>,
    id: Option<String>,
    world_id: String,
    title: String,
    content: String,
    triggers: String,
) -> AppResult<String> {
    crate::storage::fs_safe_id(&world_id)?;
    validate_codex_entry(&db::CodexSaveEntry {
        id: id.clone(),
        title: title.clone(),
        content: content.clone(),
        triggers: triggers.clone(),
    })?;
    let conn = state.db.lock().await;
    db::upsert_codex_entry(&conn, id.as_deref(), &world_id, &title, &content, &triggers)
        .map_err(Into::into)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn codex_list_for_world(
    state: State<'_, AppState>,
    world_id: String,
) -> AppResult<Vec<CodexEntry>> {
    let conn = state.db.lock().await;
    db::list_codex_for_world(&conn, &world_id).map_err(Into::into)
}

/// Import a portable world template. It deliberately excludes covers, local
/// paths, sessions, generated images, provider settings, and credentials.
#[tauri::command(rename_all = "snake_case")]
pub async fn world_import_json(state: State<'_, AppState>, json: String) -> AppResult<String> {
    if json.len() > 1_000_000 {
        return Err(crate::error::AppError::Config(
            "story import is too large".into(),
        ));
    }
    #[derive(serde::Deserialize)]
    struct ImportWorld {
        name: String,
        genre: String,
        description: Option<String>,
        system_prompt: Option<String>,
        accent_color: Option<String>,
        is_nsfw: Option<bool>,
        #[serde(default)]
        privacy_mode: Option<String>,
    }
    #[derive(serde::Deserialize)]
    struct ImportCodex {
        title: String,
        content: String,
        triggers: String,
    }
    #[derive(serde::Deserialize)]
    struct Template {
        world: ImportWorld,
        #[serde(default)]
        codex: Vec<ImportCodex>,
        #[serde(default)]
        visual_bible: VisualBible,
    }

    let value: serde_json::Value = serde_json::from_str(&json)
        .map_err(|e| crate::error::AppError::Other(format!("Invalid story JSON: {e}")))?;
    let template = if value.get("world").is_some() {
        serde_json::from_value::<Template>(value)
            .map_err(|e| crate::error::AppError::Other(format!("Invalid story template: {e}")))?
    } else {
        Template {
            world: serde_json::from_value(value)
                .map_err(|e| crate::error::AppError::Other(format!("Invalid story JSON: {e}")))?,
            codex: Vec::new(),
            visual_bible: VisualBible::default(),
        }
    };
    let w = template.world;
    let privacy_mode = w.privacy_mode.unwrap_or_else(|| "local_only".into());
    validate_privacy_mode(&privacy_mode)?;

    let new_world = db::NewWorld {
        name: w.name,
        genre: w.genre,
        description: w.description,
        system_prompt: w.system_prompt.unwrap_or_default(),
        accent_color: w.accent_color,
        is_nsfw: w.is_nsfw.unwrap_or(false),
        // Imported absolute paths belong to another machine and must never
        // be trusted as local renderer paths. The user can upload a cover
        // after import.
        cover_image_path: None,
        source: "user".to_string(),
    };
    validate_world(&new_world)?;

    let conn = state.db.lock().await;
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let result = (|| -> AppResult<String> {
        let world_id = db::create_world(&conn, &new_world)?;
        conn.execute(
            "UPDATE worlds SET privacy_mode = ?2 WHERE id = ?1",
            rusqlite::params![world_id, privacy_mode],
        )?;
        db::set_visual_bible(&conn, &world_id, &template.visual_bible)?;
        if template.codex.len() > 100 {
            return Err(crate::error::AppError::Config(
                "story import has more than 100 codex entries".into(),
            ));
        }
        for entry in template.codex.into_iter() {
            validate_text("codex title", &entry.title, 200)?;
            validate_text("codex content", &entry.content, 20_000)?;
            if entry.triggers.chars().count() > 4_000 {
                return Err(crate::error::AppError::Config(
                    "codex triggers are too long".into(),
                ));
            }
            db::upsert_codex_entry(
                &conn,
                None,
                &world_id,
                &entry.title,
                &entry.content,
                &entry.triggers,
            )?;
        }
        Ok(world_id)
    })();
    match result {
        Ok(id) => {
            conn.execute_batch("COMMIT")?;
            Ok(id)
        }
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

/// Upload a custom cover image (base64 PNG/JPEG) for a story and persist it
/// as the world's cover. Saves to `<app_data>/covers/cover_<world_id>.<ext>`
/// and updates `cover_image_path` in the DB.
#[tauri::command(rename_all = "snake_case")]
pub async fn world_upload_cover(
    state: State<'_, AppState>,
    world_id: String,
    image_b64: String,
) -> AppResult<String> {
    crate::storage::fs_safe_id(&world_id)?;
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(image_b64)
        .map_err(|e| crate::error::AppError::Other(format!("invalid base64: {e}")))?;
    if bytes.len() > 25 * 1024 * 1024 {
        return Err(crate::error::AppError::Config(
            "cover image is too large — 25 MB max".into(),
        ));
    }

    // Heuristic MIME from magic bytes.
    let ext = if bytes.starts_with(b"\x89PNG") {
        "png"
    } else if bytes.starts_with(b"\xff\xd8") {
        "jpg"
    } else if bytes.starts_with(b"RIFF") && bytes.len() > 12 && &bytes[8..12] == b"WEBP" {
        "webp"
    } else {
        return Err(crate::error::AppError::Other(
            "unsupported image format — use PNG, JPEG or WebP".into(),
        ));
    };

    let app_data = crate::storage::app_data_dir()?;
    let covers_dir = app_data.join("covers");
    std::fs::create_dir_all(&covers_dir)?;
    let path = covers_dir.join(format!("cover_{world_id}.{ext}"));
    std::fs::write(&path, &bytes)?;
    let path_str = path.to_string_lossy().to_string();

    let conn = state.db.lock().await;
    db::update_cover_image_path(&conn, &world_id, &path_str)?;
    Ok(path_str)
}

#[cfg(test)]
mod tests {
    use super::is_owned_cover_path;

    #[test]
    fn owned_cover_cleanup_rejects_paths_outside_canonical_covers_directory() {
        let root = tempfile::tempdir().unwrap();
        let covers = root.path().join("covers");
        std::fs::create_dir(&covers).unwrap();
        let cover = covers.join("cover_world.png");
        let outside = root.path().join("secret.txt");
        std::fs::write(&cover, []).unwrap();
        std::fs::write(&outside, []).unwrap();

        assert!(is_owned_cover_path(&cover, &covers));
        assert!(!is_owned_cover_path(&outside, &covers));
        assert!(!is_owned_cover_path(&covers.join("missing.png"), &covers));
    }
}
