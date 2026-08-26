use futures_util::future::{AbortHandle, Abortable};
use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};

use crate::error::{AppError, AppResult};
use crate::inference::catalog::{CatalogEntry, CATALOG};
use crate::inference::cloud::CloudProvider;
use crate::inference::codex;
use crate::inference::ollama::{PullProgress, TagsModel};
use crate::state::AppState;

/// Save user's chat model preference to app_meta.
async fn save_chat_model_pref(db: &crate::db::DbHandle, model: &str) -> AppResult<()> {
    let conn = db.lock().await;
    crate::db::meta_set(&conn, "user_chat_model", model)
}

/// Save user's embed model preference to app_meta.
async fn save_embed_model_pref(db: &crate::db::DbHandle, model: &str) -> AppResult<()> {
    let conn = db.lock().await;
    crate::db::meta_set(&conn, "user_embed_model", model)
}

#[derive(Debug, Serialize)]
pub struct ModelListResult {
    pub installed: Vec<TagsModel>,
    pub catalog: Vec<&'static CatalogEntry>,
    pub user_chat_default: Option<String>,
    pub user_embed_default: Option<String>,
    pub hardware_chat_default: String,
}

/// Start the local engine and load the selected narrator when a story session
/// opens. Home, library, and cloud-only use stay completely cold.
#[tauri::command]
pub async fn model_prewarm_active(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    let (model, context_size) = {
        let conn = state.db.lock().await;
        let model = crate::db::meta_get(&conn, "user_chat_model")?
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| state.hardware.recommended_chat_model.clone());
        let context_size = crate::db::meta_get(&conn, "contextSize")?
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(8192)
            .clamp(512, state.context_limit());
        (model, context_size)
    };
    let cloud = model
        .split_once(':')
        .and_then(|(provider, _)| CloudProvider::from_str(provider))
        .is_some();
    if cloud || codex::is_codex_model(&model) {
        return Ok(());
    }

    state.ensure_ollama(&app).await?;
    let installed = state.ollama.list_models().await?;
    if installed.iter().any(|candidate| {
        candidate.name == model || candidate.name.starts_with(&(model.clone() + ":"))
    }) {
        state.ollama.load_model(&model, Some(context_size)).await?;
    }
    Ok(())
}

#[tauri::command]
pub async fn model_unload_active(state: State<'_, AppState>) -> AppResult<()> {
    state.sidecars.unload_owned(&state.ollama).await;
    Ok(())
}

#[tauri::command]
pub async fn model_list(app: AppHandle, state: State<'_, AppState>) -> AppResult<ModelListResult> {
    state.ensure_ollama(&app).await?;
    let installed = state.ollama.list_models().await.unwrap_or_else(|err| {
        tracing::warn!(?err, "list_models failed; ollama may be down");
        Vec::new()
    });
    let catalog: Vec<&CatalogEntry> = CATALOG.iter().collect();

    let user_chat_default = {
        let conn = state.db.lock().await;
        crate::db::meta_get(&conn, "user_chat_model")?
    };
    let user_embed_default = {
        let conn = state.db.lock().await;
        crate::db::meta_get(&conn, "user_embed_model")?
    };

    Ok(ModelListResult {
        installed,
        catalog,
        user_chat_default,
        user_embed_default,
        hardware_chat_default: state.hardware.recommended_chat_model.clone(),
    })
}

#[derive(Debug, Deserialize)]
pub struct ModelPullArgs {
    pub model: String,
}

/// Stream pull progress to the front-end through a Tauri Channel.
/// Front-end usage:
/// ```ts
/// const ch = new Channel<PullProgress>();
/// ch.onmessage = (p) => console.log(p);
/// await invoke("model_pull", { args: { model: "qwen3:8b" }, channel: ch });
/// ```
#[tauri::command]
pub async fn model_pull(
    app: AppHandle,
    state: State<'_, AppState>,
    args: ModelPullArgs,
    channel: Channel<PullProgress>,
) -> AppResult<()> {
    let model = args.model.trim().to_string();
    if model.is_empty() {
        return Err(AppError::Config("empty model name".into()));
    }

    let (abort_handle, abort_registration) = AbortHandle::new_pair();
    {
        let mut pulls = state.model_pull_cancellations.lock().await;
        if pulls.contains_key(&model) {
            return Err(AppError::Config(format!(
                "model pull already active for {model}"
            )));
        }
        pulls.insert(model.clone(), abort_handle);
    }

    let result = Abortable::new(
        async {
            state.ensure_ollama(&app).await?;
            state
                .ollama
                .pull(&model, |p| {
                    let _ = channel.send(p);
                })
                .await
        },
        abort_registration,
    )
    .await;

    state.model_pull_cancellations.lock().await.remove(&model);
    result.map_err(|_| AppError::Ollama("model pull cancelled".into()))?
}

#[derive(Debug, Deserialize)]
pub struct ModelPullCancelArgs {
    pub model: String,
}

/// Abort the streaming pull request for this model. Closing the request stream
/// tells Ollama to stop the active pull; repeated cancels are harmless.
#[tauri::command]
pub async fn model_pull_cancel(
    state: State<'_, AppState>,
    args: ModelPullCancelArgs,
) -> AppResult<()> {
    if let Some(handle) = state
        .model_pull_cancellations
        .lock()
        .await
        .get(args.model.trim())
        .cloned()
    {
        handle.abort();
    }
    Ok(())
}

/// Ollama returns installed model names with an explicit `:latest` tag
/// when the user pulled without specifying a version; catalog ids and
/// saved preferences usually omit it. Strip on both sides before
/// comparing (hard rule 20).
fn strip_latest(s: &str) -> &str {
    s.strip_suffix(":latest").unwrap_or(s)
}

#[derive(Debug, Deserialize)]
pub struct ModelDeleteArgs {
    pub model: String,
}

/// Pure decision extracted for unit-testability: returns `Some(reason)`
/// when `bare` (an already `strip_latest`-d model name) must NOT be
/// deleted, `None` when it's safe. Mirrors `chat_pipeline::
/// respond_inner`'s exact resolution order for the chat default
/// (explicit user pref, else the hardware-tier recommendation) — see
/// hard rule 88, where the frontend-only version of this check missed
/// that fallback and let a user delete the model chat was silently
/// relying on.
///
/// Embed models are protected only while they ARE the active embedder:
/// the explicit `user_embed_model` pref, or — when no pref is set — the
/// implicit `embeddinggemma` fallback that `commands/memory.rs` uses.
/// Unused open-source embed models (nomic-embed-text, bge-m3, …) can be
/// deleted freely.
fn model_delete_block_reason(
    bare: &str,
    user_chat_default: Option<&str>,
    user_embed_default: Option<&str>,
    hardware_chat_default: &str,
) -> Option<String> {
    let effective_chat_default = user_chat_default.unwrap_or(hardware_chat_default);
    if strip_latest(effective_chat_default) == bare {
        return Some(format!(
            "{bare} is your active chat model — set a different default first"
        ));
    }
    let effective_embed_default = user_embed_default
        .map(strip_latest)
        .unwrap_or("embeddinggemma");
    if strip_latest(bare) == effective_embed_default {
        return Some(format!(
            "{bare} is your active embed model — set a different default first"
        ));
    }
    None
}

#[tauri::command]
pub async fn model_delete(
    app: AppHandle,
    state: State<'_, AppState>,
    args: ModelDeleteArgs,
) -> AppResult<()> {
    state.ensure_ollama(&app).await?;
    let model = args.model.trim();
    if model.is_empty() {
        return Err(AppError::Config("empty model name".into()));
    }
    let bare = strip_latest(model);

    let (user_chat_default, user_embed_default) = {
        let conn = state.db.lock().await;
        (
            crate::db::meta_get(&conn, "user_chat_model")?,
            crate::db::meta_get(&conn, "user_embed_model")?,
        )
    };
    if let Some(reason) = model_delete_block_reason(
        bare,
        user_chat_default.as_deref(),
        user_embed_default.as_deref(),
        &state.hardware.recommended_chat_model,
    ) {
        return Err(AppError::Config(reason));
    }

    state.ollama.delete_model(model).await
}

#[cfg(test)]
mod model_delete_guard_tests {
    use super::*;

    #[test]
    fn blocks_implicit_embed_fallback_when_no_pref() {
        // No explicit embed default: embeddinggemma is the implicit fallback
        // (commands/memory.rs) and must stay deletable-proof.
        let reason =
            model_delete_block_reason("embeddinggemma", Some("gemma4:e4b"), None, "gemma4:e4b");
        assert!(reason.is_some());
        assert!(reason.unwrap().contains("active embed model"));
    }

    #[test]
    fn blocks_explicit_chat_default() {
        let reason =
            model_delete_block_reason("qwen3.5:9b", Some("qwen3.5:9b"), None, "gemma4:e4b");
        assert!(reason.is_some());
        assert!(reason.unwrap().contains("active chat model"));
    }

    #[test]
    fn blocks_hardware_fallback_default_when_no_explicit_pref() {
        // The exact bug this guard exists for: no explicit "Set
        // default" click yet, but this IS the model chat is using.
        let reason = model_delete_block_reason("gemma4:e4b", None, None, "gemma4:e4b");
        assert!(reason.is_some());
        assert!(reason.unwrap().contains("active chat model"));
    }

    #[test]
    fn blocks_explicit_embed_default() {
        let reason =
            model_delete_block_reason("bge-m3", Some("qwen3.5:9b"), Some("bge-m3"), "gemma4:e4b");
        assert!(reason.is_some());
    }

    #[test]
    fn allows_deleting_an_unused_installed_model() {
        // A different chat model the user pulled but isn't using —
        // safe to delete regardless of :latest suffix quirks.
        let reason = model_delete_block_reason(
            "qwen3.5:4b",
            Some("qwen3.5:9b"),
            Some("embeddinggemma"),
            "gemma4:e4b",
        );
        assert!(reason.is_none());
    }

    #[test]
    fn allows_deleting_unused_embed_when_another_is_default() {
        // Open-source embeds are deletable as long as they are not the
        // active embedder (explicit pref or the embeddinggemma fallback).
        let reason = model_delete_block_reason(
            "nomic-embed-text",
            Some("qwen3.5:9b"),
            Some("bge-m3"),
            "gemma4:e4b",
        );
        assert!(reason.is_none());
        let fallback_protected =
            model_delete_block_reason("embeddinggemma", Some("qwen3.5:9b"), None, "gemma4:e4b");
        assert!(fallback_protected.is_some());
    }

    #[test]
    fn strips_latest_on_both_sides() {
        let reason = model_delete_block_reason(
            "gemma4:e4b", // already bare, as the command handler passes it
            None,
            None,
            "gemma4:e4b:latest", // hardware default stored with an explicit tag
        );
        assert!(reason.is_some());
    }
}

#[derive(Debug, Deserialize)]
pub struct ModelSetDefaultArgs {
    /// "chat" | "embed"
    pub role: String,
    pub model: String,
}

#[tauri::command]
pub async fn model_set_default(
    app: AppHandle,
    state: State<'_, AppState>,
    args: ModelSetDefaultArgs,
) -> AppResult<()> {
    let model = args.model.trim();
    if model.is_empty() {
        return Err(AppError::Config("empty model name".into()));
    }
    if args.role == "chat" {
        let is_cloud = model
            .split_once(':')
            .and_then(|(provider, _)| CloudProvider::from_str(provider))
            .is_some()
            || codex::is_codex_model(model);
        if !is_cloud {
            state.ensure_ollama(&app).await?;
            let installed = state.ollama.list_models().await?;
            let normalized = model.strip_suffix(":latest").unwrap_or(model);
            let present = installed
                .iter()
                .any(|item| item.name.strip_suffix(":latest").unwrap_or(&item.name) == normalized);
            if !present {
                return Err(AppError::Config(format!("model {model} is not installed")));
            }
        }
    }
    match args.role.as_str() {
        "chat" => save_chat_model_pref(&state.db, model).await,
        "embed" => {
            // Embed models are local too — an uninstalled embed model must
            // not silently become the active semantic-memory embedder.
            state.ensure_ollama(&app).await?;
            let installed = state.ollama.list_models().await?;
            let normalized = model.strip_suffix(":latest").unwrap_or(model);
            let present = installed
                .iter()
                .any(|item| item.name.strip_suffix(":latest").unwrap_or(&item.name) == normalized);
            if !present {
                return Err(AppError::Config(format!("embed model {model} is not installed")));
            }
            save_embed_model_pref(&state.db, model).await
        }
        other => Err(AppError::Config(format!(
            "model role must be 'chat' or 'embed', got {other}"
        ))),
    }
}

/// Reports whether the locally installed Codex CLI has an active ChatGPT
/// session. FPV never receives or stores the credentials used by that CLI.
#[tauri::command]
pub async fn codex_status() -> AppResult<bool> {
    Ok(codex::status().await)
}

#[tauri::command]
pub fn codex_login() -> AppResult<()> {
    codex::start_login()
}

/// Default Codex model when the user has not set one. Kept as a fallback so
/// the Codex narrator works out of the box; the user can change it in
/// Settings without an app update when OpenAI ships a newer model.
const DEFAULT_CODEX_MODEL: &str = "gpt-5.6-terra";

#[tauri::command]
pub async fn codex_model_get(state: State<'_, AppState>) -> AppResult<String> {
    let conn = state.db.lock().await;
    Ok(crate::db::meta_get(&conn, "codex_model")?
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_CODEX_MODEL.to_string()))
}

#[tauri::command]
pub async fn codex_model_set(model: String, state: State<'_, AppState>) -> AppResult<()> {
    let model = model.trim();
    if model.is_empty() {
        return Err(AppError::Config("empty codex model".into()));
    }
    let conn = state.db.lock().await;
    crate::db::meta_set(&conn, "codex_model", model)
}
