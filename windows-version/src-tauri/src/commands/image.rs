//! Tauri commands for FPV image generation (local sd.cpp only).

use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};

use crate::db;
use crate::error::AppResult;
use crate::image::{self, ImageProvider, ImageRequest, ImageResult, ImageStyle};
use crate::state::AppState;

pub(crate) async fn preflight_local_generation(
    state: &AppState,
    model: image::sdcpp::LocalModel,
) -> AppResult<()> {
    if image::local::check().1 < model.min_ram_gib() {
        return Err(crate::error::AppError::Config(format!(
            "{} requires at least {} GiB RAM",
            model.id(),
            model.min_ram_gib()
        )));
    }
    // Apple Silicon shares memory between local LLMs and Metal rendering.
    // Release the narrator before a multi-GB image render to avoid swapping.
    state.sidecars.unload_owned(&state.ollama).await;
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct ImageCheckResult {
    pub mflux_installed: bool,
    pub ram_gb: u64,
    pub local_ready: bool,
    pub weights_cached: Option<bool>,
}

/// Check whether the local image backend (bundled sd.cpp) is ready.
#[tauri::command]
pub async fn image_local_check(state: State<'_, AppState>) -> AppResult<ImageCheckResult> {
    let (mflux_installed, ram_gb) = image::local::check();
    let (weights_cached, min_ram_gib) = if mflux_installed {
        let model = {
            let conn = state.db.lock().await;
            read_local_model(&conn)
        };
        (
            image::local::weights_status_of_async(model).await,
            model.min_ram_gib(),
        )
    } else {
        (None, image::local_min_ram_gb())
    };
    Ok(ImageCheckResult {
        mflux_installed,
        ram_gb,
        local_ready: mflux_installed && ram_gb >= min_ram_gib && weights_cached != Some(false),
        weights_cached,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct PrewarmProgress {
    pub phase: String,
    pub percent: Option<u8>,
    pub message: Option<String>,
}

#[tauri::command]
pub async fn image_local_prewarm(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> AppResult<()> {
    use tauri::Emitter;
    let emit = |phase: &str, percent: Option<u8>, message: Option<String>| {
        let _ = app.emit(
            "image:prewarm",
            PrewarmProgress {
                phase: phase.to_string(),
                percent,
                message,
            },
        );
    };
    let model = {
        let conn = state.db.lock().await;
        read_local_model(&conn)
    };
    if image::local::weights_status_of_async(model).await == Some(true) {
        emit("done", Some(100), None);
        return Ok(());
    }
    emit("downloading", Some(0), None);
    let result = image::local::prewarm(model, |phase, percent| {
        let phase = match phase {
            image::local::PrewarmPhase::Downloading => "downloading",
        };
        emit(phase, percent, None);
    })
    .await;
    match result {
        Ok(()) => {
            emit("done", Some(100), None);
            Ok(())
        }
        Err(e) => {
            emit("error", None, Some(e.to_string()));
            Err(e)
        }
    }
}

fn selected_provider(conn: &rusqlite::Connection) -> ImageProvider {
    db::meta_get(conn, "image_provider")
        .ok()
        .flatten()
        .and_then(|provider| ImageProvider::from_str(&provider))
        .unwrap_or_default()
}

pub(crate) fn cloud_image_allowed(offline: bool, privacy_mode: Option<&str>) -> AppResult<()> {
    if offline {
        return Err(crate::error::AppError::Config(
            "offline mode is enabled; select the local image provider first".into(),
        ));
    }
    if privacy_mode != Some("cloud_allowed") {
        return Err(crate::error::AppError::Config(
            "this story is Local only; enable cloud access for this story or select the local image provider".into(),
        ));
    }
    Ok(())
}

/// Generate an image using the selected provider.
#[tauri::command]
pub async fn image_generate(
    app: tauri::AppHandle,
    input: GenerateImageInput,
    state: State<'_, AppState>,
) -> AppResult<ImageResult> {
    if input.prompt.len() > 32_000 {
        return Err(crate::error::AppError::Config(
            "image prompt exceeds 32 KB".into(),
        ));
    }
    let style = match input.style.as_deref() {
        Some("photo") => ImageStyle::Photo,
        Some("realistic") => ImageStyle::Realistic,
        Some("watercolor") => ImageStyle::Watercolor,
        Some("ink") => ImageStyle::Ink,
        Some("cinematic") => ImageStyle::Cinematic,
        Some("dark-fantasy") => ImageStyle::DarkFantasy,
        Some("manga") => ImageStyle::Manga,
        _ => ImageStyle::Anime,
    };
    let (provider, offline, privacy_mode, local_steps, local_model, cloud_model) = {
        let conn = state.db.lock().await;
        let provider = selected_provider(&conn);
        let offline = db::meta_get(&conn, "offlineMode")?
            .map(|value| value == "true")
            .unwrap_or(false);
        let privacy_mode = input
            .world_id
            .as_deref()
            .map(|world_id| {
                crate::story::db::get_world(&conn, world_id).map(|world| world.privacy_mode)
            })
            .transpose()?;
        let model = read_local_model(&conn);
        let configured_steps = read_local_steps(&conn);
        let quality_steps = match input.quality.as_deref() {
            Some("draft") => model.step_choices().first().copied(),
            Some("high") => {
                if model.is_large() {
                    model.step_choices().last().copied()
                } else {
                    model
                        .step_choices()
                        .get(1)
                        .copied()
                        .or_else(|| model.step_choices().first().copied())
                }
            }
            _ => configured_steps,
        };
        (
            provider,
            offline,
            privacy_mode,
            quality_steps,
            read_local_model_id(&conn),
            read_cloud_image_model(&conn, provider),
        )
    };
    let model_kind = image::sdcpp::LocalModel::from_id(local_model.as_deref().unwrap_or_default())
        .unwrap_or_default();
    let provider_health_test = input.prompt == "test" && input.world_id.is_none();
    if provider == ImageProvider::Local {
        preflight_local_generation(&state, model_kind).await?;
    } else if provider_health_test {
        // No world_id means there's no story to apply the per-story privacy
        // gate to, but Offline Mode is a global switch and must still hold.
        if offline {
            return Err(crate::error::AppError::Config(
                "offline mode is enabled; select the local image provider first".into(),
            ));
        }
    } else {
        cloud_image_allowed(offline, privacy_mode.as_deref())?;
    }
    let req = ImageRequest {
        prompt: input.prompt,
        style,
        reference_image_b64: None,
        local_steps,
        local_model,
        kontext_refs_b64: Vec::new(),
        extra_negative: None,
        seed: input.seed,
    };
    if provider.is_cloud() {
        return image::generate(provider, req, cloud_model.as_deref()).await;
    }
    let mut progress = image::sdcpp::subscribe_render_progress();
    let generation = image::generate(provider, req, cloud_model.as_deref());
    tokio::pin!(generation);
    loop {
        tokio::select! {
            result = &mut generation => return result,
            changed = progress.changed() => {
                if changed.is_err() { continue; }
                let _ = app.emit("image:render_progress", *progress.borrow());
            }
        }
    }
}

#[tauri::command]
pub async fn image_generation_cancel() -> AppResult<()> {
    image::sdcpp::cancel_generation();
    Ok(())
}

#[tauri::command]
pub async fn image_download_cancel() -> AppResult<()> {
    image::sdcpp::cancel_download();
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct GenerateImageInput {
    pub prompt: String,
    #[serde(default)]
    pub style: Option<String>,
    #[serde(default)]
    pub quality: Option<String>,
    #[serde(default)]
    pub seed: Option<i64>,
    #[serde(default)]
    pub world_id: Option<String>,
}

// --- Local model management ---

pub fn read_local_steps(conn: &rusqlite::Connection) -> Option<u32> {
    let model = read_local_model(conn);
    db::meta_get(conn, &format!("image_local_steps_{}", model.id()))
        .ok()
        .flatten()
        .and_then(|s| s.parse::<u32>().ok())
        .map(|v| model.sanitize_steps(v))
}

pub fn read_local_model_id(conn: &rusqlite::Connection) -> Option<String> {
    db::meta_get(conn, "image_local_model")
        .ok()
        .flatten()
        .filter(|s| image::sdcpp::LocalModel::from_id(s).is_some())
}

pub fn read_local_model(conn: &rusqlite::Connection) -> image::sdcpp::LocalModel {
    read_local_model_id(conn)
        .and_then(|s| image::sdcpp::LocalModel::from_id(&s))
        .unwrap_or_default()
}

#[derive(Debug, serde::Serialize)]
pub struct LocalModelChoice {
    pub id: String,
    pub download_gb: u64,
    pub ready: bool,
    pub large: bool,
    pub steps: [u32; 4],
    pub default_steps: u32,
    pub min_ram_gib: u64,
    pub ram_ok: bool,
}

#[tauri::command]
pub async fn image_local_models(state: State<'_, AppState>) -> AppResult<Vec<LocalModelChoice>> {
    let _ = state;
    let ram_gb = image::local::check().1;
    let mut choices = Vec::with_capacity(image::sdcpp::LocalModel::CHOICES.len());
    for m in image::sdcpp::LocalModel::CHOICES {
        // Sequential rather than a `map`: `ready` hashes the weights on a
        // cold cache and has to happen off the runtime.
        let ready = image::sdcpp::weights_ready_for_async(m).await;
        choices.push(LocalModelChoice {
            id: m.id().to_string(),
            download_gb: m.missing_download_gb(),
            ready,
            large: m.is_large(),
            steps: m.step_choices(),
            default_steps: m.default_steps(),
            min_ram_gib: m.min_ram_gib(),
            ram_ok: ram_gb >= m.min_ram_gib(),
        });
    }
    Ok(choices)
}

#[tauri::command]
pub async fn image_local_model_get(state: State<'_, AppState>) -> AppResult<String> {
    let conn = state.db.lock().await;
    Ok(read_local_model(&conn).id().to_string())
}

#[tauri::command]
pub async fn image_local_model_set(model: String, state: State<'_, AppState>) -> AppResult<()> {
    image::sdcpp::LocalModel::from_id(&model).ok_or_else(|| {
        crate::error::AppError::Config(format!("unknown local image model: {model}"))
    })?;
    let conn = state.db.lock().await;
    db::meta_set(&conn, "image_local_model", &model)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn image_local_model_delete(model: String, state: State<'_, AppState>) -> AppResult<()> {
    let target = image::sdcpp::LocalModel::from_id(&model).ok_or_else(|| {
        crate::error::AppError::Config(format!("unknown local image model: {model}"))
    })?;
    let active = {
        let conn = state.db.lock().await;
        read_local_model(&conn)
    };
    if active == target {
        return Err(crate::error::AppError::Config(
            "cannot delete the active image model; select another model first".into(),
        ));
    }
    let _render_slot = image::sdcpp::try_render_slot().ok_or_else(|| {
        crate::error::AppError::Config(
            "a local render is in progress — wait for it to finish before deleting weights".into(),
        )
    })?;
    let _download_slot = image::sdcpp::try_download_slot().ok_or_else(|| {
        crate::error::AppError::Config(
            "a local image download is in progress — wait for it to finish before deleting weights"
                .into(),
        )
    })?;
    let dir = image::sdcpp::weights_dir()?;
    let download_in_progress = image::sdcpp::download_in_progress_for(target, |filename| {
        dir.join(format!("{filename}.part")).exists()
    });
    if let Some(reason) = image::sdcpp::weight_delete_block_reason(false, download_in_progress) {
        return Err(crate::error::AppError::Config(reason.into()));
    }
    // `plan_weight_deletion` asks "is this other model ready" for every
    // shared file, so the readiness probe runs off the runtime here too.
    let safe_files = {
        tokio::task::spawn_blocking(move || {
            image::sdcpp::plan_weight_deletion(target, image::sdcpp::weights_ready_for)
        })
        .await
        .map_err(|e| crate::error::AppError::Other(format!("weight deletion plan failed: {e}")))?
    };
    for filename in safe_files {
        let p = dir.join(filename);
        if p.exists() {
            std::fs::remove_file(&p)
                .map_err(|e| crate::error::AppError::Other(format!("deleting {filename}: {e}")))?;
        }
        let verification = dir.join(format!("{filename}.sha256"));
        let _ = std::fs::remove_file(verification);
    }
    Ok(())
}

#[tauri::command]
pub async fn image_provider_set(provider: String, state: State<'_, AppState>) -> AppResult<()> {
    ImageProvider::from_str(&provider).ok_or_else(|| {
        crate::error::AppError::Config(format!("unknown image provider: {provider}"))
    })?;
    let conn = state.db.lock().await;
    db::meta_set(
        &conn,
        "image_provider",
        ImageProvider::from_str(&provider).unwrap().as_str(),
    )
}

#[tauri::command]
pub async fn image_provider_get(state: State<'_, AppState>) -> AppResult<String> {
    let conn = state.db.lock().await;
    Ok(selected_provider(&conn).as_str().to_string())
}

// --- Cloud image model selection (live discovery, like chat models) ---

/// BYOK provider holding the API key for a cloud image provider.
fn image_key_provider(provider: ImageProvider) -> crate::inference::cloud::CloudProvider {
    match provider {
        ImageProvider::Openai => crate::inference::cloud::CloudProvider::Openai,
        ImageProvider::Seedream => crate::inference::cloud::CloudProvider::Doubao,
        ImageProvider::Hunyuan => crate::inference::cloud::CloudProvider::Hunyuan,
        ImageProvider::CogView => crate::inference::cloud::CloudProvider::Zhipu,
        ImageProvider::Flux => crate::inference::cloud::CloudProvider::Bfl,
        ImageProvider::Imagen => crate::inference::cloud::CloudProvider::Google,
        ImageProvider::Fal => crate::inference::cloud::CloudProvider::Fal,
        ImageProvider::Local => unreachable!("local has no BYOK key"),
    }
}

/// Read the user's saved cloud image model for `provider` (None = default).
pub fn read_cloud_image_model(
    conn: &rusqlite::Connection,
    provider: ImageProvider,
) -> Option<String> {
    if !provider.is_cloud() {
        return None;
    }
    db::meta_get(conn, image::meta_key_for_model(provider))
        .ok()
        .flatten()
        .filter(|m| !m.trim().is_empty())
}

/// Live list of image-capable models for a cloud image provider, fetched from
/// the provider's own API — never a hand-maintained list, so a newly released
/// model needs no app update. A missing key is a normal optional-provider
/// state (empty list); HTTP/provider failures are returned so the UI can
/// distinguish them. Respects Offline Mode like every cloud-reaching command.
#[tauri::command]
pub async fn image_cloud_models(
    provider: String,
    state: State<'_, AppState>,
) -> AppResult<Vec<String>> {
    let provider = match ImageProvider::from_str(&provider) {
        Some(p) if p.is_cloud() => p,
        _ => return Ok(vec![]),
    };

    let offline = {
        let conn = state.db.lock().await;
        db::meta_get(&conn, "offlineMode")?
            .map(|value| value == "true")
            .unwrap_or(false)
    };
    if offline {
        return Err(crate::error::AppError::Config(
            "offline mode is enabled; disable it to browse cloud models".into(),
        ));
    }

    // BFL's API is task-based (POST /v1/{model}) and exposes no model-list
    // endpoint, so ship the known text-to-image endpoints compatible with the
    // app's sync/async flow. Newer BFL API generations (e.g. flux-pro-2.0)
    // use a different endpoint shape and need an adapter change, not just a
    // model name, so they are intentionally not listed here.
    if provider == ImageProvider::Flux {
        return Ok(vec![
            "flux-pro-1.1".into(),
            "flux-pro-1.1-ultra".into(),
            "flux-dev".into(),
            "flux-schnell".into(),
            "flux-kontext-pro".into(),
            "flux-kontext-dev".into(),
        ]);
    }

    // fal nie ma katalogu modeli — pusta lista sprawia, że ImageModelTab
    // pokazuje pole ręcznego wpisania ("No models returned — enter one
    // manually below"), które jest tu jedyną poprawną drogą.
    if provider == ImageProvider::Fal {
        return Ok(vec![]);
    }

    let key_provider = image_key_provider(provider);
    let key = match crate::byok::read(key_provider)? {
        Some(k) => k,
        None => return Ok(vec![]),
    };
    let headers = crate::commands::cloud::build_models_headers(key_provider, &key)?;
    let client = reqwest::Client::builder()
        .default_headers(headers)
        .timeout(std::time::Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| crate::error::AppError::Other(format!("client build: {e}")))?;

    let url = match provider {
        ImageProvider::Imagen => {
            "https://generativelanguage.googleapis.com/v1beta/models".to_string()
        }
        _ => {
            let base = crate::inference::cloud_chat::base_url_for(key_provider)?;
            format!("{}/models", base.trim_end_matches('/'))
        }
    };
    let response = client.get(&url).send().await.map_err(|e| {
        crate::error::AppError::Other(format!("cloud image model discovery request failed: {e}"))
    })?;
    let status = response.status();
    let body = crate::inference::cloud_chat::bounded_body(response).await?;
    if !status.is_success() {
        return Err(crate::error::AppError::Other(format!(
            "cloud image model discovery failed ({status}): {}",
            crate::inference::cloud::sanitize_error_body(&body)
        )));
    }
    Ok(parse_image_models(provider, &body))
}

/// Parse a provider's model list and keep only image-capable ids. When the
/// keyword filter matches nothing (e.g. a provider listing endpoint ids
/// instead of names), fall back to the full list so the user can still pick.
fn parse_image_models(provider: ImageProvider, body: &str) -> Vec<String> {
    let parsed: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    if provider == ImageProvider::Imagen {
        // Google: models array; keep generateContent-capable image models.
        return parsed["models"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|m| {
                        let supported = m["supportedGenerationMethods"]
                            .as_array()
                            .map(|methods| {
                                methods
                                    .iter()
                                    .any(|m| m.as_str() == Some("generateContent"))
                            })
                            .unwrap_or(false);
                        let name = m["name"].as_str()?;
                        (supported && name.contains("image")).then(|| name.replace("models/", ""))
                    })
                    .collect()
            })
            .unwrap_or_default();
    }
    let ids: Vec<String> = parsed["data"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|m| m["id"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let filtered: Vec<String> = ids
        .iter()
        .filter(|id| match provider {
            ImageProvider::Openai => id.contains("image") || id.starts_with("dall-e"),
            ImageProvider::Seedream => id.contains("seedream"),
            ImageProvider::Hunyuan => id.contains("hunyuan-image"),
            ImageProvider::CogView => id.contains("cogview"),
            _ => true,
        })
        .cloned()
        .collect();
    if filtered.is_empty() {
        ids
    } else {
        filtered
    }
}

/// Saved (or default) cloud image model for a provider.
#[tauri::command]
pub async fn image_cloud_model_get(
    provider: String,
    state: State<'_, AppState>,
) -> AppResult<String> {
    let provider = match ImageProvider::from_str(&provider) {
        Some(p) if p.is_cloud() => p,
        _ => return Ok("local".into()),
    };
    let conn = state.db.lock().await;
    Ok(read_cloud_image_model(&conn, provider)
        .unwrap_or_else(|| image::default_cloud_model(provider).to_string()))
}

/// Persist the user's cloud image model choice. Any non-empty name is
/// accepted (manual entry must keep working when a provider's list API is
/// down or the model is brand new).
#[tauri::command]
pub async fn image_cloud_model_set(
    provider: String,
    model: String,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let provider = match ImageProvider::from_str(&provider) {
        Some(p) if p.is_cloud() => p,
        _ => {
            return Err(crate::error::AppError::Config(
                "not a cloud image provider".into(),
            ))
        }
    };
    let model = model.trim();
    if model.is_empty() {
        return Err(crate::error::AppError::Config(
            "empty image model name".into(),
        ));
    }
    let conn = state.db.lock().await;
    db::meta_set(&conn, image::meta_key_for_model(provider), model)
}

#[cfg(test)]
mod tests {
    use super::cloud_image_allowed;

    #[test]
    fn cloud_images_require_online_cloud_allowed_world() {
        assert!(cloud_image_allowed(false, Some("cloud_allowed")).is_ok());
        assert!(cloud_image_allowed(true, Some("cloud_allowed")).is_err());
        assert!(cloud_image_allowed(false, Some("local_only")).is_err());
        assert!(cloud_image_allowed(false, None).is_err());
    }
}
