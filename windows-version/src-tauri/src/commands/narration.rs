use tauri::{Emitter, State};

use crate::byok;
use crate::error::{AppError, AppResult};
use crate::inference::cloud::CloudProvider;
use crate::inference::cloud_chat;
use crate::inference::codex;
use crate::inference::ollama::{ChatMessage, ChatRequest};
use crate::state::AppState;
use crate::story::db::StoryState;

/// True when a model spec is a cloud provider (`deepseek:deepseek-chat`),
/// meaning narration should route to the user's BYOK key instead of Ollama.
fn is_cloud_model(model: &str) -> bool {
    model
        .split_once(':')
        .and_then(|(p, _)| CloudProvider::from_str(p))
        .is_some()
}

fn is_remote_model(model: &str) -> bool {
    is_cloud_model(model) || codex::is_codex_model(model)
}

fn provider_name(model: &str) -> &str {
    model
        .split_once(':')
        .and_then(|(provider, _)| CloudProvider::from_str(provider).map(|_| provider))
        .unwrap_or("ollama")
}

async fn wait_for_request_cancellation(
    cancel: &mut tokio::sync::broadcast::Receiver<String>,
    request_id: &str,
) {
    while let Ok(cancelled) = cancel.recv().await {
        if cancelled == request_id {
            return;
        }
    }
}

/// Run one narration turn against a cloud provider using the saved BYOK key.
async fn cloud_narration(
    _state: &AppState,
    model: &str,
    prompt: &str,
    temperature: f64,
    top_p: f64,
    max_tokens: i64,
) -> AppResult<String> {
    let (provider_str, cloud_model) = model
        .split_once(':')
        .ok_or_else(|| AppError::Config(format!("malformed model spec: {model}")))?;
    let provider = CloudProvider::from_str(provider_str)
        .ok_or_else(|| AppError::Config(format!("unknown provider: {provider_str}")))?;
    let api_key = byok::read(provider)
        .map_err(|e| AppError::Other(format!("reading saved key: {e}")))?
        .ok_or_else(|| {
            AppError::Config(format!(
                "no API key saved for {provider_str} — add one in Settings → AI Cloud Connection"
            ))
        })?;
    let base_url = cloud_chat::base_url_for(provider)?;
    cloud_chat::chat(
        provider,
        cloud_model,
        prompt,
        &api_key,
        &base_url,
        cloud_chat::GenerationOptions {
            temperature,
            top_p,
            max_tokens,
        },
    )
    .await
}

async fn reserve_cloud_spend(
    state: &AppState,
    request_id: &str,
    prompt: &str,
    max_tokens: i64,
) -> AppResult<Option<crate::db::cloud_spend::Reservation>> {
    let mut conn = state.db.lock().await;
    let Some(limit) = crate::db::meta_get(&conn, "cloud_daily_limit_usd")? else {
        return Ok(None);
    };
    if limit.trim().is_empty() {
        return Ok(None);
    }
    crate::db::cloud_spend::reserve(
        &mut conn,
        request_id,
        &limit,
        prompt.len(),
        max_tokens,
        crate::metrics::now(),
    )
    .map(Some)
}

async fn settle_cloud_spend(
    state: &AppState,
    reservation: Option<crate::db::cloud_spend::Reservation>,
    result: &AppResult<String>,
) {
    let Some(reservation) = reservation else {
        return;
    };
    let conn = state.db.lock().await;
    match result {
        Ok(text) => {
            let _ = crate::db::cloud_spend::complete(
                &conn,
                reservation,
                text.len().div_ceil(4),
                crate::metrics::now(),
            );
        }
        // Keep the full reservation for cancelled/failed calls: a provider may
        // already have started billable work before the local request ended.
        // "cancelled" is the ledger's valid status for this (V28 CHECK only
        // allows reserved/completed/cancelled/failed — "interrupted" would
        // violate it and leave the row stuck as `reserved` forever).
        Err(_) => {
            if let Err(err) = crate::db::cloud_spend::retain_reservation(
                &conn,
                reservation,
                "cancelled",
                crate::metrics::now(),
            ) {
                tracing::error!(?err, "failed to settle cloud spend reservation");
            }
        }
    }
}

/// The model narration actually runs on: the user's explicit choice
/// (Settings → Models, or the onboarding engine step, both of which write
/// `user_chat_model` via `model_set_default`) falling back to the
/// hardware-tier recommendation when they have never chosen one.
///
/// Until 2026-07-30 both narration commands read the hardware
/// recommendation directly, so picking a model had no effect on story
/// generation at all — the preference was written and never read.
async fn resolve_chat_model(state: &AppState, world_id: &str) -> AppResult<String> {
    let (pref, offline) = {
        let conn = state.db.lock().await;
        let model = crate::db::meta_get(&conn, "user_chat_model")?;
        let offline = crate::db::meta_get(&conn, "offlineMode")?
            .map(|value| value == "true")
            .unwrap_or(false);
        (model, offline)
    };
    let model = pref
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| state.hardware.recommended_chat_model.clone());
    let custom_is_local = if model
        .split_once(':')
        .and_then(|(provider, _)| CloudProvider::from_str(provider))
        == Some(CloudProvider::Custom)
    {
        tokio::task::spawn_blocking(byok::custom_endpoint_is_local)
            .await
            .map_err(|e| AppError::Other(format!("custom endpoint check failed: {e}")))?
    } else {
        false
    };
    if offline && is_remote_model(&model) && !custom_is_local {
        return Err(AppError::Config(
            "offline mode is enabled; select a local narrator model first".into(),
        ));
    }
    let privacy_mode = {
        let conn = state.db.lock().await;
        crate::story::db::get_world(&conn, world_id)?.privacy_mode
    };
    if privacy_mode == "local_only" && is_remote_model(&model) {
        let custom_is_loopback = if model
            .split_once(':')
            .and_then(|(provider, _)| CloudProvider::from_str(provider))
            == Some(CloudProvider::Custom)
        {
            tokio::task::spawn_blocking(byok::custom_endpoint_is_loopback)
                .await
                .map_err(|e| AppError::Other(format!("custom endpoint check failed: {e}")))?
        } else {
            false
        };
        if !custom_is_loopback {
            return Err(AppError::Config(
                "this story is Local only; choose a local narrator or an on-device custom endpoint"
                    .into(),
            ));
        }
    }
    Ok(model)
}

async fn resolve_context_size(state: &AppState) -> AppResult<i64> {
    let conn = state.db.lock().await;
    let configured = crate::db::meta_get(&conn, "contextSize")?
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(8192);
    Ok(configured.clamp(512, state.context_limit()))
}

/// Build a ChatRequest for narration, optionally overriding generation params.
fn build_narration_request(
    model: String,
    prompt: String,
    context_size: i64,
    temperature: Option<f64>,
    top_p: Option<f64>,
    max_tokens: Option<i64>,
) -> ChatRequest {
    let mut options = serde_json::json!({
        "num_ctx": context_size,
        "temperature": 0.85,
        "top_p": 0.95,
    });
    if let Some(t) = temperature {
        options["temperature"] = serde_json::json!(t);
    }
    if let Some(t) = top_p {
        options["top_p"] = serde_json::json!(t);
    }
    if let Some(m) = max_tokens {
        options["num_predict"] = serde_json::json!(m);
    }
    ChatRequest {
        model,
        messages: vec![ChatMessage {
            role: "user".into(),
            content: prompt,
            images: None,
        }],
        stream: false,
        options: Some(options),
        // Reasoning-capable models otherwise spend the token budget thinking
        // and can return no visible narration at all.
        think: Some(false),
        format: None,
    }
}

#[tauri::command(rename_all = "snake_case")]
#[allow(clippy::too_many_arguments)]
pub async fn narration_generate(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    prompt: String,
    world_id: String,
    request_id: String,
    temperature: Option<f64>,
    top_p: Option<f64>,
    max_tokens: Option<i64>,
) -> AppResult<String> {
    if prompt.len() > 512_000 {
        return Err(AppError::Config("narration prompt exceeds 512 KB".into()));
    }
    let mut cancel = state.narration_cancel.subscribe();
    let started = std::time::Instant::now();
    let input_tokens = prompt.len() / 4;
    let model = resolve_chat_model(&state, &world_id).await?;
    if codex::is_codex_model(&model) {
        let result = codex::narrate(&model, &prompt, &mut cancel, &request_id, |_| {}).await;
        if let Ok(text) = &result {
            crate::metrics::record_generation(crate::metrics::GenerationMetric {
                timestamp: crate::metrics::now(),
                stage: "narration",
                provider: "codex",
                model: &model,
                input_tokens_estimate: input_tokens,
                output_tokens_estimate: text.len() / 4,
                output_tokens_actual: None,
                model_duration_ms: None,
                tokens_per_second: None,
                duration_ms: started.elapsed().as_millis(),
                first_token_ms: None,
                streaming: false,
            });
        }
        return result;
    }
    // Cloud provider spec → route to the user's BYOK key; otherwise local Ollama.
    if is_cloud_model(&model) {
        let max_tokens = max_tokens.unwrap_or(2048).clamp(64, 4096);
        let reservation = reserve_cloud_spend(&state, &request_id, &prompt, max_tokens).await?;
        let result = tokio::select! {
            result = cloud_narration(
                &state,
                &model,
                &prompt,
                temperature.unwrap_or(0.85),
                top_p.unwrap_or(0.95),
                max_tokens,
            ) => result,
            _ = wait_for_request_cancellation(&mut cancel, &request_id) => Err(AppError::Other("Narration cancelled".into())),
        };
        settle_cloud_spend(&state, reservation, &result).await;
        if let Ok(text) = &result {
            crate::metrics::record_generation(crate::metrics::GenerationMetric {
                timestamp: crate::metrics::now(),
                stage: "narration",
                provider: provider_name(&model),
                model: &model,
                input_tokens_estimate: input_tokens,
                output_tokens_estimate: text.len() / 4,
                output_tokens_actual: None,
                model_duration_ms: None,
                tokens_per_second: None,
                duration_ms: started.elapsed().as_millis(),
                first_token_ms: None,
                streaming: false,
            });
        }
        return result;
    }
    state.ensure_ollama(&app).await?;
    let context_size = resolve_context_size(&state).await?;
    let req = build_narration_request(model, prompt, context_size, temperature, top_p, max_tokens);
    let resp = tokio::select! {
        result = state.ollama.chat(req) => result?,
        _ = wait_for_request_cancellation(&mut cancel, &request_id) => return Err(AppError::Other("Narration cancelled".into())),
    };
    let actual_tokens = resp.eval_count;
    let model_duration_ms = resp.total_duration as f64 / 1_000_000.0;
    let tokens_per_second = if resp.total_duration > 0 {
        Some(resp.eval_count as f64 / (resp.total_duration as f64 / 1_000_000_000.0))
    } else {
        None
    };
    let text = resp.message.map(|m| m.content).unwrap_or_default();
    crate::metrics::record_generation(crate::metrics::GenerationMetric {
        timestamp: crate::metrics::now(),
        stage: "narration",
        provider: "ollama",
        model: "local",
        input_tokens_estimate: input_tokens,
        output_tokens_estimate: text.len() / 4,
        output_tokens_actual: Some(actual_tokens),
        model_duration_ms: Some(model_duration_ms),
        tokens_per_second,
        duration_ms: started.elapsed().as_millis(),
        first_token_ms: None,
        streaming: false,
    });
    Ok(text)
}

/// Streaming variant — emits `narration:token` events for each token.
/// Returns `ChatStreamSummary` (full text + model + tokens info).
#[tauri::command(rename_all = "snake_case")]
#[allow(clippy::too_many_arguments)]
pub async fn narration_generate_stream(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    prompt: String,
    world_id: String,
    request_id: String,
    temperature: Option<f64>,
    top_p: Option<f64>,
    max_tokens: Option<i64>,
) -> AppResult<String> {
    if prompt.len() > 512_000 {
        return Err(AppError::Config("narration prompt exceeds 512 KB".into()));
    }
    let mut cancel = state.narration_cancel.subscribe();
    let started = std::time::Instant::now();
    let input_tokens = prompt.len() / 4;
    let model = resolve_chat_model(&state, &world_id).await?;
    if codex::is_codex_model(&model) {
        let app_clone = app.clone();
        let result = codex::narrate(&model, &prompt, &mut cancel, &request_id, move |token| {
            let _ = app_clone.emit("narration:token", token);
        })
        .await;
        if let Ok(text) = &result {
            crate::metrics::record_generation(crate::metrics::GenerationMetric {
                timestamp: crate::metrics::now(),
                stage: "narration",
                provider: "codex",
                model: &model,
                input_tokens_estimate: input_tokens,
                output_tokens_estimate: text.len() / 4,
                output_tokens_actual: None,
                model_duration_ms: None,
                tokens_per_second: None,
                duration_ms: started.elapsed().as_millis(),
                first_token_ms: None,
                streaming: true,
            });
        }
        return result;
    }
    // Cloud path: no SSE here yet — do a single non-stream call and emit
    // the full text as one token event so the narration screen still
    // streams in (token-by-token streaming is a follow-up).
    if is_cloud_model(&model) {
        let (provider_str, cloud_model) = model
            .split_once(':')
            .ok_or_else(|| AppError::Config(format!("malformed model spec: {model}")))?;
        let provider = CloudProvider::from_str(provider_str)
            .ok_or_else(|| AppError::Config(format!("unknown provider: {provider_str}")))?;
        let api_key = byok::read(provider)?
            .ok_or_else(|| AppError::Config(format!("no API key saved for {provider_str}")))?;
        let base_url = cloud_chat::base_url_for(provider)?;
        let options = cloud_chat::GenerationOptions {
            temperature: temperature.unwrap_or(0.85),
            top_p: top_p.unwrap_or(0.95),
            max_tokens: max_tokens.unwrap_or(2048).clamp(64, 4096),
        };
        let reservation =
            reserve_cloud_spend(&state, &request_id, &prompt, options.max_tokens).await?;
        let app_clone = app.clone();
        let first_token = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let first_token_callback = first_token.clone();
        let result = tokio::select! {
            result = cloud_chat::chat_stream(provider, cloud_model, &prompt, &api_key, &base_url, options, move |token| {
                if !token.is_empty() {
                    let elapsed = started.elapsed().as_millis().min(u64::MAX as u128) as u64 + 1;
                    let _ = first_token_callback.compare_exchange(0, elapsed, std::sync::atomic::Ordering::Relaxed, std::sync::atomic::Ordering::Relaxed);
                }
                let _ = app_clone.emit("narration:token", token);
            }) => result,
            _ = wait_for_request_cancellation(&mut cancel, &request_id) => Err(AppError::Other("Narration cancelled".into())),
        };
        settle_cloud_spend(&state, reservation, &result).await;
        let text = result?;
        crate::metrics::record_generation(crate::metrics::GenerationMetric {
            timestamp: crate::metrics::now(),
            stage: "narration",
            provider: provider_str,
            model: cloud_model,
            input_tokens_estimate: input_tokens,
            output_tokens_estimate: text.len() / 4,
            output_tokens_actual: None,
            model_duration_ms: None,
            tokens_per_second: None,
            duration_ms: started.elapsed().as_millis(),
            first_token_ms: first_token
                .load(std::sync::atomic::Ordering::Relaxed)
                .checked_sub(1),
            streaming: true,
        });
        return Ok(text);
    }
    state.ensure_ollama(&app).await?;
    let context_size = resolve_context_size(&state).await?;
    let req = build_narration_request(model, prompt, context_size, temperature, top_p, max_tokens);
    let app_clone = app.clone();
    let first_token = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let first_token_callback = first_token.clone();
    let summary = tokio::select! {
        result = state.ollama.chat_stream(req, move |token| {
            if !token.is_empty() {
                let elapsed = started.elapsed().as_millis().min(u64::MAX as u128) as u64 + 1;
                let _ = first_token_callback.compare_exchange(0, elapsed, std::sync::atomic::Ordering::Relaxed, std::sync::atomic::Ordering::Relaxed);
            }
            let _ = app_clone.emit("narration:token", token);
        }) => result?,
        _ = wait_for_request_cancellation(&mut cancel, &request_id) => return Err(AppError::Other("Narration cancelled".into())),
    };
    crate::metrics::record_generation(crate::metrics::GenerationMetric {
        timestamp: crate::metrics::now(),
        stage: "narration",
        provider: "ollama",
        model: &summary.model,
        input_tokens_estimate: input_tokens,
        output_tokens_estimate: summary.text.len() / 4,
        output_tokens_actual: Some(summary.eval_count),
        model_duration_ms: Some(summary.total_duration as f64 / 1_000_000.0),
        tokens_per_second: if summary.total_duration > 0 {
            Some(summary.eval_count as f64 / (summary.total_duration as f64 / 1_000_000_000.0))
        } else {
            None
        },
        duration_ms: started.elapsed().as_millis(),
        first_token_ms: first_token
            .load(std::sync::atomic::Ordering::Relaxed)
            .checked_sub(1),
        streaming: true,
    });
    Ok(summary.text)
}

#[tauri::command(rename_all = "snake_case")]
pub fn narration_cancel(state: State<'_, AppState>, request_id: String) -> AppResult<()> {
    let _ = state.narration_cancel.send(request_id);
    Ok(())
}

/// Extract durable continuity facts from the finished scene. This is a
/// separate short pass so the visible prose never has to carry JSON markers.
#[tauri::command(rename_all = "snake_case")]
pub async fn story_state_extract(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    user_action: String,
    narration: String,
    current_state: StoryState,
    world_id: String,
) -> AppResult<StoryState> {
    let model = resolve_chat_model(&state, &world_id).await?;
    let local_model = if is_remote_model(&model) && state.ollama.ping().await.unwrap_or(false) {
        let recommended = state.hardware.recommended_chat_model.clone();
        let installed = state.ollama.list_models().await.unwrap_or_default();
        if installed.iter().any(|candidate| {
            candidate.name == recommended
                || candidate.name.starts_with(&(recommended.clone() + ":"))
        }) {
            Some(recommended)
        } else {
            None
        }
    } else {
        None
    };
    if !is_remote_model(&model) {
        state.ensure_ollama(&app).await?;
    }
    let extraction_model = local_model.clone().unwrap_or_else(|| model.clone());
    let prompt = format!(
        r#"You maintain continuity for an interactive story. Return ONLY valid JSON.

Extract facts from the CURRENT SCENE. Never invent facts, motives, items, relationships,
locations, goals, or events that are not explicitly present in the previous state, the
player action, or the scene. Preserve existing entries unless the scene clearly changes
or resolves them. Keep arrays short and concrete (at most 8 items each).

JSON schema:
{{
  "turn": number,
  "location": "current concrete location or previous value",
  "active_goals": ["unfinished goal"],
  "open_threads": ["unresolved question or pressure"],
  "inventory": ["confirmed item"],
  "relationships": ["name: current relationship or change"],
  "facts": ["confirmed world or story fact"],
  "conflicts": ["possible contradiction requiring author review"],
  "last_scene": "one-sentence factual summary",
  "quests": [{{"id":"stable-slug","title":"quest title","status":"active|completed|failed|paused","description":"concrete objective"}}],
  "inventory_items": [{{"name":"item","quantity":1,"condition":"intact"}}],
  "relationship_records": [{{"character":"name","status":"ally|neutral|wary|hostile|romantic","note":"current concrete relationship"}}],
  "pending_changes": [{{"id":"stable-id","kind":"quest_add|quest_update|inventory_add|inventory_remove|relationship_update","summary":"human-readable proposed change","target":"quest/item/character id","value":"compact JSON value"}}]
}}

PREVIOUS STATE:
{previous}

PLAYER ACTION:
{action}

CURRENT SCENE:
{scene}"#,
        previous = serde_json::to_string(&current_state).unwrap_or_else(|_| "{}".into()),
        action = user_action,
        scene = narration,
    );

    let raw = if let Some(local_model) = local_model {
        let context_size = resolve_context_size(&state).await?;
        let mut req = build_narration_request(
            local_model,
            prompt.clone(),
            context_size,
            Some(0.1),
            Some(0.9),
            Some(512),
        );
        req.format = Some(serde_json::json!("json"));
        let response = state.ollama.chat(req).await?;
        response.message.map(|m| m.content).unwrap_or_default()
    } else if is_remote_model(&model) {
        // Do not make a second paid request just to extract continuity. Keep
        // the durable anchor and let the next local turn enrich it.
        return Ok(StoryState {
            turn: current_state.turn,
            location: current_state.location.clone(),
            active_goals: current_state.active_goals.clone(),
            open_threads: current_state.open_threads.clone(),
            inventory: current_state.inventory.clone(),
            relationships: current_state.relationships.clone(),
            facts: current_state.facts.clone(),
            conflicts: current_state.conflicts.clone(),
            last_scene: narration.chars().take(300).collect(),
            source: current_state.source.clone(),
            quests: current_state.quests.clone(),
            inventory_items: current_state.inventory_items.clone(),
            relationship_records: current_state.relationship_records.clone(),
            pending_changes: current_state.pending_changes.clone(),
        });
    } else {
        let context_size = resolve_context_size(&state).await?;
        let mut req = build_narration_request(
            model.clone(),
            prompt.clone(),
            context_size,
            Some(0.1),
            Some(0.9),
            Some(512),
        );
        req.format = Some(serde_json::json!("json"));
        let response = state.ollama.chat(req).await?;
        response.message.map(|m| m.content).unwrap_or_default()
    };

    match parse_story_state_response(&raw, &current_state, &narration) {
        Ok(extracted) => Ok(extracted),
        Err(first_error) => {
            // Small local models occasionally ignore JSON mode after a long
            // prose generation. Retry once with a stricter, lower-temperature
            // request; the finished narration is already durable, so failure
            // here must never invalidate the story turn.
            tracing::warn!(error = %first_error, "story state extraction returned invalid JSON; retrying");
            let retry_prompt = format!(
                "Return ONLY one valid compact JSON object. No markdown, no explanation, no trailing text.\n\n{prompt}"
            );
            let context_size = resolve_context_size(&state).await?;
            let mut req = build_narration_request(
                extraction_model,
                retry_prompt,
                context_size,
                Some(0.0),
                Some(0.8),
                Some(512),
            );
            req.format = Some(serde_json::json!("json"));
            let response = state.ollama.chat(req).await?;
            let retry_raw = response.message.map(|m| m.content).unwrap_or_default();
            parse_story_state_response(&retry_raw, &current_state, &narration)
        }
    }
}

fn parse_story_state_response(
    raw: &str,
    previous: &StoryState,
    narration: &str,
) -> AppResult<StoryState> {
    let json = extract_json_object(raw)
        .ok_or_else(|| AppError::Ollama("story state extractor returned invalid JSON".into()))?;
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| AppError::Ollama(format!("story state JSON invalid: {e}")))?;
    validate_story_state_value(&value)?;
    Ok(normalize_story_state(&value, previous, narration))
}

fn extract_json_object(raw: &str) -> Option<&str> {
    let bytes = raw.as_bytes();
    for start in 0..bytes.len() {
        if bytes[start] != b'{' {
            continue;
        }
        let mut depth = 0usize;
        let mut in_string = false;
        let mut escaped = false;
        for index in start..bytes.len() {
            let byte = bytes[index];
            if in_string {
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == b'"' {
                    in_string = false;
                }
                continue;
            }
            match byte {
                b'"' => in_string = true,
                b'{' => depth += 1,
                b'}' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        let candidate = &raw[start..=index];
                        if serde_json::from_str::<serde_json::Value>(candidate).is_ok() {
                            return Some(candidate);
                        }
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    None
}

fn validate_story_state_value(value: &serde_json::Value) -> AppResult<()> {
    let Some(object) = value.as_object() else {
        return Err(AppError::Ollama("story state must be a JSON object".into()));
    };
    if let Some(turn) = object.get("turn") {
        if !turn.is_i64() && !turn.is_u64() {
            return Err(AppError::Ollama(
                "story state turn must be an integer".into(),
            ));
        }
    }
    for field in [
        "active_goals",
        "open_threads",
        "inventory",
        "relationships",
        "facts",
        "conflicts",
    ] {
        if let Some(value) = object.get(field) {
            if !value.is_array() && !value.is_object() && !value.is_string() {
                return Err(AppError::Ollama(format!(
                    "story state {field} has an invalid shape"
                )));
            }
        }
    }
    if let Some(location) = object.get("location") {
        if !location.is_string() && !location.is_object() {
            return Err(AppError::Ollama(
                "story state location has an invalid shape".into(),
            ));
        }
    }
    Ok(())
}

fn value_text(value: &serde_json::Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return Some(text.trim().to_string()).filter(|text| !text.is_empty());
    }
    value.as_object().and_then(|object| {
        ["description", "summary", "name", "topic", "status"]
            .iter()
            .filter_map(|key| object.get(*key).and_then(value_text))
            .collect::<Vec<_>>()
            .first()
            .cloned()
    })
}

fn list_text(value: Option<&serde_json::Value>) -> Vec<String> {
    match value {
        Some(serde_json::Value::Array(items)) => {
            items.iter().filter_map(value_text).take(8).collect()
        }
        Some(serde_json::Value::Object(items)) => items
            .iter()
            .filter_map(|(name, value)| value_text(value).map(|text| format!("{name}: {text}")))
            .take(8)
            .collect(),
        Some(value) => value_text(value).into_iter().collect(),
        None => Vec::new(),
    }
}

fn merge_state_list(previous: &[String], extracted: Vec<String>) -> Vec<String> {
    let mut merged = previous.to_vec();
    for item in extracted {
        if !merged
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(&item))
        {
            merged.push(item);
        }
    }
    merged.into_iter().take(8).collect()
}

fn normalize_story_state(
    value: &serde_json::Value,
    previous: &StoryState,
    narration: &str,
) -> StoryState {
    let object = value.as_object();
    let turn = object
        .and_then(|o| o.get("turn"))
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(previous.turn);
    let location = object
        .and_then(|o| o.get("location"))
        .and_then(value_text)
        .unwrap_or_else(|| previous.location.clone());
    let active_goals = merge_state_list(
        &previous.active_goals,
        list_text(object.and_then(|o| o.get("active_goals"))),
    );
    let open_threads = merge_state_list(
        &previous.open_threads,
        list_text(object.and_then(|o| o.get("open_threads"))),
    );
    let inventory = merge_state_list(
        &previous.inventory,
        list_text(object.and_then(|o| o.get("inventory"))),
    );
    let relationships = merge_state_list(
        &previous.relationships,
        list_text(object.and_then(|o| o.get("relationships"))),
    );
    let facts = merge_state_list(
        &previous.facts,
        list_text(object.and_then(|o| o.get("facts"))),
    );
    let conflicts = merge_state_list(
        &previous.conflicts,
        list_text(object.and_then(|o| o.get("conflicts"))),
    );
    // Structured changes only become canon through the player's explicit
    // accept action in the UI. The extractor may propose them, but must not
    // overwrite confirmed quests, inventory, or relationships.
    let quests = previous.quests.clone();
    let inventory_items = previous.inventory_items.clone();
    let relationship_records = previous.relationship_records.clone();
    let pending_changes = merge_pending_changes(
        &previous.pending_changes,
        object
            .and_then(|o| o.get("pending_changes"))
            .map(parse_pending_changes)
            .unwrap_or_default(),
    );
    let last_scene = object
        .and_then(|o| o.get("last_scene"))
        .and_then(value_text)
        .unwrap_or_else(|| previous.last_scene.clone());

    StoryState {
        turn: turn.max(previous.turn),
        location,
        active_goals,
        open_threads,
        inventory,
        relationships,
        facts,
        conflicts,
        last_scene: if last_scene.is_empty() {
            narration.chars().take(300).collect()
        } else {
            last_scene
        },
        source: if previous.source == "manual" {
            "manual"
        } else {
            "model"
        }
        .into(),
        quests,
        inventory_items,
        relationship_records,
        pending_changes,
    }
}

fn parse_pending_changes(value: &serde_json::Value) -> Vec<crate::story::db::PendingChange> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let object = item.as_object()?;
                    let summary = object.get("summary").and_then(value_text)?;
                    let value = object.get("value")?;
                    let value = value
                        .as_str()
                        .map(str::to_owned)
                        .or_else(|| serde_json::to_string(value).ok())?;
                    Some(crate::story::db::PendingChange {
                        id: object
                            .get("id")
                            .and_then(value_text)
                            .unwrap_or_else(|| format!("change-{}", summary.len())),
                        kind: object
                            .get("kind")
                            .and_then(value_text)
                            .unwrap_or_else(|| "review".into()),
                        summary,
                        target: object
                            .get("target")
                            .and_then(value_text)
                            .unwrap_or_default(),
                        value,
                    })
                })
                .take(8)
                .collect()
        })
        .unwrap_or_default()
}

fn merge_pending_changes(
    previous: &[crate::story::db::PendingChange],
    extracted: Vec<crate::story::db::PendingChange>,
) -> Vec<crate::story::db::PendingChange> {
    let mut merged = previous.to_vec();
    for change in extracted {
        if let Some(index) = merged.iter().position(|existing| existing.id == change.id) {
            merged[index] = change;
        } else {
            merged.push(change);
        }
    }
    merged.into_iter().take(8).collect()
}

#[cfg(test)]
mod story_state_tests {
    use super::{
        extract_json_object, normalize_story_state, parse_story_state_response,
        validate_story_state_value,
    };
    use crate::story::db::StoryState;

    #[test]
    fn normalizes_richer_model_json_without_losing_previous_state() {
        let previous = StoryState {
            turn: 2,
            location: "station".into(),
            facts: vec!["The gate is locked.".into()],
            ..Default::default()
        };
        let value = serde_json::json!({
            "turn": 3,
            "location": "old station",
            "active_goals": [{"description": "Open the north gate"}],
            "relationships": {"Mara": {"status": "ally"}},
            "last_scene": {"summary": "Mara gives you a brass key."}
        });

        let state = normalize_story_state(&value, &previous, "fallback");
        assert_eq!(state.turn, 3);
        assert_eq!(state.location, "old station");
        assert_eq!(state.active_goals, vec!["Open the north gate"]);
        assert_eq!(state.relationships, vec!["Mara: ally"]);
        assert_eq!(state.facts, previous.facts);
        assert_eq!(state.last_scene, "Mara gives you a brass key.");
    }

    #[test]
    fn merges_new_state_facts_instead_of_replacing_previous_entries() {
        let previous = StoryState {
            active_goals: vec!["Find the gate".into()],
            facts: vec!["The gate is locked.".into()],
            conflicts: vec!["Key location is uncertain.".into()],
            source: "manual".into(),
            ..Default::default()
        };
        let value = serde_json::json!({
            "active_goals": ["Reach the tower"],
            "facts": ["The bell rings at midnight."],
            "conflicts": []
        });

        let state = normalize_story_state(&value, &previous, "fallback");
        assert_eq!(state.active_goals, vec!["Find the gate", "Reach the tower"]);
        assert_eq!(
            state.facts,
            vec!["The gate is locked.", "The bell rings at midnight."]
        );
        assert_eq!(state.conflicts, vec!["Key location is uncertain."]);
        assert_eq!(state.source, "manual");
    }

    #[test]
    fn keeps_important_changes_pending_until_author_review() {
        let previous = StoryState::default();
        let value = serde_json::json!({
            "pending_changes": [{
                "id": "quest-gate",
                "kind": "quest_add",
                "summary": "Add a quest to open the north gate",
                "target": "north-gate",
                "value": "{\"id\":\"north-gate\",\"title\":\"Open the north gate\",\"status\":\"active\",\"description\":\"Find the brass key\"}"
            }]
        });
        let state = normalize_story_state(&value, &previous, "The gate waits.");
        assert!(state.quests.is_empty());
        assert_eq!(state.pending_changes.len(), 1);
        assert_eq!(state.pending_changes[0].kind, "quest_add");
    }

    #[test]
    fn retains_confirmed_state_and_pending_changes_until_reviewed() {
        let previous = StoryState {
            quests: vec![crate::story::db::Quest {
                id: "gate".into(),
                title: "Open the gate".into(),
                status: "active".into(),
                description: "Find the key".into(),
            }],
            pending_changes: vec![crate::story::db::PendingChange {
                id: "old-change".into(),
                kind: "inventory_add".into(),
                summary: "Add the map".into(),
                target: "map".into(),
                value: "{}".into(),
            }],
            ..Default::default()
        };
        let value = serde_json::json!({
            "quests": [],
            "pending_changes": [{
                "id": "new-change",
                "kind": "quest_add",
                "summary": "Add a new quest",
                "target": "tower",
                "value": "{}"
            }]
        });

        let state = normalize_story_state(&value, &previous, "The tower looms.");
        assert_eq!(state.quests, previous.quests);
        assert_eq!(state.pending_changes.len(), 2);
        assert!(state
            .pending_changes
            .iter()
            .any(|change| change.id == "old-change"));
        assert!(state
            .pending_changes
            .iter()
            .any(|change| change.id == "new-change"));
    }

    #[test]
    fn extracts_balanced_json_when_narration_contains_braces() {
        let raw = r#"The scene mentions {a broken gate}, then the state is:
        {"location":"tower","facts":["The gate is {locked}"],"turn":4}"#;
        let json = extract_json_object(raw).unwrap();
        let value: serde_json::Value = serde_json::from_str(json).unwrap();
        validate_story_state_value(&value).unwrap();
        assert_eq!(value["location"], "tower");
    }

    #[test]
    fn rejects_invalid_story_state_shapes() {
        let value = serde_json::json!({"turn": "four", "facts": 42});
        assert!(validate_story_state_value(&value).is_err());
    }

    #[test]
    fn invalid_extractor_output_is_reported_without_mutating_previous_state() {
        let previous = StoryState {
            location: "kitchen".into(),
            facts: vec!["The radio is speaking.".into()],
            ..Default::default()
        };
        let result = parse_story_state_response("not json", &previous, "A finished scene.");
        assert!(result.is_err());
        assert_eq!(previous.location, "kitchen");
        assert_eq!(previous.facts, vec!["The radio is speaking."]);
    }
}
