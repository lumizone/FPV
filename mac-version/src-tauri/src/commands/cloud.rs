//! Tauri commands for listing cloud provider models via their APIs.
//! Reads the saved BYOK key per provider and calls /v1/models.

use tauri::State;

use crate::byok;
use crate::db;
use crate::error::{AppError, AppResult};
use crate::inference::cloud::CloudProvider;
use crate::state::AppState;

/// Fetch available chat models from a cloud provider's API.
/// A missing key is a normal optional-provider state. Once configured, HTTP
/// and provider failures are returned so the UI can distinguish them from an
/// empty but valid model catalog. Respects Offline Mode like every other
/// cloud-reaching command — no network call is made while it's on.
#[tauri::command(rename_all = "snake_case")]
pub async fn cloud_list_models(
    provider: String,
    state: State<'_, AppState>,
) -> AppResult<Vec<String>> {
    let p = match CloudProvider::from_str(&provider) {
        Some(p) => p,
        None => return Ok(vec![]),
    };

    let offline = {
        let conn = state.db.lock().await;
        db::meta_get(&conn, "offlineMode")?
            .map(|value| value == "true")
            .unwrap_or(false)
    };
    if offline {
        return Err(AppError::Config(
            "offline mode is enabled; disable it to browse cloud models".into(),
        ));
    }

    // AI21 retired its public model-list endpoint (GET /studio/v1/models now
    // returns 410 Gone and /v1/models 404s), so there is nothing to discover
    // there. Return an empty catalog — the UI already falls back to the manual
    // model-name field. Revisit when AI21 ships a list endpoint again.
    if p == CloudProvider::Ai21 {
        return Ok(vec![]);
    }

    // Black Forest Labs is an image-only provider: it has no `/models`
    // route at all (404 even on the live host, checked 2026-08-26) and its
    // FLUX model ids are a fixed list hardcoded in `commands::image`. It was
    // sitting in the chat-discovery group below, so asking for its chat
    // models produced a 404 error instead of "this provider has none".
    if p == CloudProvider::Bfl {
        return Ok(vec![]);
    }

    // fal.ai publikuje żadnej listy modeli — endpoint JEST modelem i podaje
    // go użytkownik. Nie ma też czatu. Pusta lista sprawia, że UI pokazuje
    // pole ręcznego wpisania zamiast błędu 404.
    if p == CloudProvider::Fal {
        return Ok(vec![]);
    }

    let key = match byok::read(p)? {
        Some(k) => k,
        None => return Ok(vec![]),
    };

    let (url, client) = build_models_client(p, &key)?;
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| AppError::Other(format!("cloud model discovery request failed: {e}")))?;
    let status = response.status();
    let body = crate::inference::cloud_chat::bounded_body(response).await?;
    if !status.is_success() {
        return Err(AppError::Other(format!(
            "cloud model discovery failed ({status}): {}",
            crate::inference::cloud::sanitize_error_body(&body)
        )));
    }
    parse_model_ids(p, &body)
}

/// Build the GET request for /v1/models (or provider-specific equivalent).
fn build_models_client(p: CloudProvider, key: &str) -> AppResult<(String, reqwest::Client)> {
    let headers = build_models_headers(p, key)?;
    let client = reqwest::Client::builder()
        .default_headers(headers)
        .timeout(std::time::Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| crate::error::AppError::Other(format!("client build: {e}")))?;

    let url = match p {
        CloudProvider::Openai
        | CloudProvider::Custom
        | CloudProvider::OpenRouter
        | CloudProvider::Groq
        | CloudProvider::Xai
        | CloudProvider::Mistral
        | CloudProvider::Deepseek
        | CloudProvider::Together
        | CloudProvider::Fireworks
        | CloudProvider::Novita
        | CloudProvider::SiliconFlow
        | CloudProvider::Moonshot
        | CloudProvider::Zhipu
        | CloudProvider::Qwen
        | CloudProvider::Baichuan
        | CloudProvider::Minimax
        | CloudProvider::Stepfun
        | CloudProvider::ModelScope
        | CloudProvider::XiaomiMimo
        | CloudProvider::Doubao
        | CloudProvider::Hunyuan
        | CloudProvider::Upstage
        | CloudProvider::Yi
        | CloudProvider::Plamo
        | CloudProvider::Nvidia
        | CloudProvider::Cohere
        | CloudProvider::Cerebras
        | CloudProvider::SambaNova
        | CloudProvider::Venice => {
            let base = crate::inference::cloud_chat::base_url_for(p)?;
            format!("{}/models", base.trim_end_matches('/'))
        }
        // Perplexity's OpenAI-compatible chat lives at /chat/completions on the
        // bare host (base `https://api.perplexity.ai`), but its model list is
        // served only under /v1/models — appending /models to the chat base
        // returns 404.
        CloudProvider::Perplexity => "https://api.perplexity.ai/v1/models".into(),
        CloudProvider::Anthropic => "https://api.anthropic.com/v1/models".into(),
        CloudProvider::Google => "https://generativelanguage.googleapis.com/v1beta/models".into(),
        // AI21 discovery is short-circuited in cloud_list_models (its model-list
        // endpoint is retired), so this arm is never reached.
        CloudProvider::Ai21 => unreachable!("AI21 model discovery is disabled"),
        CloudProvider::Bfl => unreachable!("BFL is image-only; discovery is disabled"),
        CloudProvider::Fal => unreachable!("fal has no model catalogue; discovery is disabled"),
    };
    Ok((url, client))
}

/// Auth headers for a provider's model-discovery GET (also reused by the
/// image pipeline's `image_cloud_models` for the same wire shape).
pub(crate) fn build_models_headers(
    p: CloudProvider,
    key: &str,
) -> AppResult<reqwest::header::HeaderMap> {
    let mut headers = reqwest::header::HeaderMap::new();
    if p == CloudProvider::Google {
        headers.insert(
            reqwest::header::HeaderName::from_static("x-goog-api-key"),
            reqwest::header::HeaderValue::from_str(key)
                .map_err(|_| crate::error::AppError::Config("invalid Google API key".into()))?,
        );
    } else if p == CloudProvider::Anthropic {
        headers.insert(
            reqwest::header::HeaderName::from_static("x-api-key"),
            reqwest::header::HeaderValue::from_str(key)
                .map_err(|_| crate::error::AppError::Config("invalid API key".into()))?,
        );
        headers.insert(
            reqwest::header::HeaderName::from_static("anthropic-version"),
            reqwest::header::HeaderValue::from_static("2023-06-01"),
        );
    } else {
        headers.insert(
            reqwest::header::AUTHORIZATION,
            reqwest::header::HeaderValue::from_str(&format!("Bearer {key}"))
                .map_err(|_| crate::error::AppError::Config("invalid API key".into()))?,
        );
    }
    Ok(headers)
}

/// Parse model IDs from a provider's JSON response.
fn parse_model_ids(p: CloudProvider, body: &str) -> AppResult<Vec<String>> {
    let parsed: serde_json::Value = serde_json::from_str(body).map_err(|e| {
        AppError::Other(format!("cloud model discovery returned invalid JSON: {e}"))
    })?;

    let models = match p {
        CloudProvider::Anthropic => parsed["data"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|m| m["id"].as_str().map(String::from))
                    .collect()
            })
            .ok_or_else(|| {
                AppError::Other("cloud model discovery response had no data array".into())
            })?,
        CloudProvider::Google => parsed["models"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|m| {
                        let id = m["name"].as_str()?;
                        let supported = m["supportedGenerationMethods"]
                            .as_array()
                            .map(|methods| {
                                methods
                                    .iter()
                                    .any(|m| m.as_str() == Some("generateContent"))
                            })
                            .unwrap_or(false);
                        if supported {
                            Some(id.replace("models/", ""))
                        } else {
                            None
                        }
                    })
                    .collect()
            })
            .ok_or_else(|| {
                AppError::Other("Google model discovery response had no models array".into())
            })?,
        // Mistral's /v1/models wraps its array in { data: [...] } like every
        // other OpenAI-compatible provider here — the one thing that's
        // actually Mistral-specific is that each entry carries a
        // capabilities.completion_chat flag, since the same endpoint also
        // lists embedding/FIM/moderation models that can't take a chat call.
        CloudProvider::Mistral => parsed["data"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|m| {
                        let chat_capable = m
                            .pointer("/capabilities/completion_chat")
                            .and_then(|value| value.as_bool())
                            .unwrap_or(false);
                        chat_capable
                            .then(|| m["id"].as_str().map(String::from))
                            .flatten()
                    })
                    .collect()
            })
            .ok_or_else(|| AppError::Other("Mistral response was not a model array".into()))?,
        _ => parsed["data"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|m| m["id"].as_str().map(String::from))
                    .collect()
            })
            .ok_or_else(|| {
                AppError::Other("cloud model discovery response had no data array".into())
            })?,
    };
    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mistral_data_wrapped_array() {
        let ids = parse_model_ids(
            CloudProvider::Mistral,
            r#"{"data":[{"id":"mistral-large-latest","capabilities":{"completion_chat":true}},{"id":"mistral-embed","capabilities":{"completion_chat":false}}]}"#,
        )
        .unwrap();
        assert_eq!(ids, ["mistral-large-latest"]);
    }

    #[test]
    fn retains_openai_compatible_data_parser() {
        let ids = parse_model_ids(CloudProvider::Groq, r#"{"data":[{"id":"llama"}]}"#).unwrap();
        assert_eq!(ids, ["llama"]);
    }

    #[test]
    fn adds_anthropic_discovery_version_header() {
        let headers = build_models_headers(CloudProvider::Anthropic, "key").unwrap();
        assert_eq!(headers["anthropic-version"], "2023-06-01");
    }

    #[test]
    fn perplexity_uses_v1_models_discovery_url() {
        let (url, _) = build_models_client(CloudProvider::Perplexity, "key").unwrap();
        assert_eq!(url, "https://api.perplexity.ai/v1/models");
    }
}
