//! Tauri commands for generating world cover images.
//!
//! These commands orchestrate the selected image pipeline to produce a
//! cover PNG for a story world and persist the file path in the DB.

use base64::Engine;
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::image::{self, ImageProvider, ImageRequest, ImageStyle};
use crate::state::AppState;
use crate::story::db;

/// Generate a cover image for a story world using the selected image backend.
///
/// 1. Loads the world from the database.
/// 2. Builds a descriptive prompt from the world's name, genre, and description.
/// 3. Generates the image via the bundled stable-diffusion.cpp sidecar.
/// 4. Saves the result as a PNG in `<app_data>/covers/`.
/// 5. Persists the file path in the world's `cover_image_path` column.
/// 6. Returns the absolute path to the generated PNG.
#[tauri::command(rename_all = "snake_case")]
pub async fn world_generate_cover(
    _app: tauri::AppHandle,
    state: State<'_, AppState>,
    world_id: String,
) -> AppResult<String> {
    crate::storage::fs_safe_id(&world_id)?;
    // 1. Load world from DB
    let conn = state.db.lock().await;
    let world = db::get_world(&conn, &world_id)?;
    let visual_bible = db::get_visual_bible(&conn, &world_id)?;
    drop(conn); // release lock before the long-running image gen

    // 2. Build a short descriptive prompt
    let prompt = format!(
        "{} cover art, {}, {}\nVisual style: {}\nColor palette: {}\nCharacter continuity: {}\nLocation continuity: {}\nAvoid: {}",
        world.name,
        world.genre,
        world.description.unwrap_or_default(),
        visual_bible.style,
        visual_bible.palette,
        visual_bible.character_anchors,
        visual_bible.location_anchors,
        visual_bible.negative_prompt,
    );

    // 3. Resolve the selected provider and local configuration while the DB is held.
    let (provider, offline, configured_model, cloud_model) = {
        let conn = state.db.lock().await;
        let provider = crate::db::meta_get(&conn, "image_provider")?
            .as_deref()
            .and_then(ImageProvider::from_str)
            .unwrap_or_default();
        (
            provider,
            crate::db::meta_get(&conn, "offlineMode")?
                .map(|value| value == "true")
                .unwrap_or(false),
            crate::commands::image::read_local_model_id(&conn),
            crate::commands::image::read_cloud_image_model(&conn, provider),
        )
    };
    if provider.is_cloud() {
        crate::commands::image::cloud_image_allowed(offline, Some(&world.privacy_mode))?;
    }
    let local_model = configured_model
        .or_else(|| Some(crate::image::sdcpp::LocalModel::default().id().to_string()));
    let model_kind = local_model
        .as_deref()
        .and_then(crate::image::sdcpp::LocalModel::from_id)
        .unwrap_or_default();
    let style = if model_kind.is_sdxl() {
        match model_kind {
            crate::image::sdcpp::LocalModel::Animagine40 => ImageStyle::Anime,
            _ => ImageStyle::Realistic,
        }
    } else {
        ImageStyle::Cinematic
    };
    if provider == ImageProvider::Local {
        crate::commands::image::preflight_local_generation(&state, model_kind).await?;
    }
    let req = ImageRequest {
        prompt,
        style,
        reference_image_b64: None,
        local_steps: None,
        local_model,
        kontext_refs_b64: vec![],
        extra_negative: None,
        seed: None,
    };
    let result = image::generate(provider, req, cloud_model.as_deref()).await?;

    // 4. Decode the base64-encoded PNG payload
    let img_bytes = base64::engine::general_purpose::STANDARD
        .decode(&result.image_b64)
        .map_err(|e| AppError::Other(format!("base64 decode: {e}")))?;
    image::validate_png_payload(&img_bytes, 25 * 1024 * 1024)?;

    // 5. Save to covers directory inside app data
    let app_data = crate::storage::app_data_dir()?;
    let covers_dir = app_data.join("covers");
    tokio::fs::create_dir_all(&covers_dir).await?;
    let filename = format!("cover_{world_id}.png");
    let path = covers_dir.join(&filename);
    let partial = covers_dir.join(format!("{filename}.part"));
    tokio::fs::write(&partial, &img_bytes).await?;
    tokio::fs::rename(&partial, &path).await?;

    let path_str = path.to_string_lossy().to_string();

    // 6. Persist the cover path in the database
    {
        let conn = state.db.lock().await;
        db::update_cover_image_path(&conn, &world_id, &path_str)?;
    }
    // Guard released first: deleting the superseded cover is filesystem
    // work that needs no database, and this is the one global mutex every
    // narration turn contends for.
    if let Some(old_path) = world.cover_image_path {
        if old_path != path_str {
            crate::commands::story::cleanup_owned_cover(&old_path).await;
        }
    }

    Ok(path_str)
}
