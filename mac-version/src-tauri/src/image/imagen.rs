//! Native Google image client (Gemini image / "nano banana"). Reuses the
//! user's Google API key (the same BYOK entry as the Gemini text provider)
//! via the `generateContent` endpoint.

use base64::Engine;

use crate::error::{AppError, AppResult};
use crate::image::{validate_png_payload, ImageRequest, ImageResult};
use crate::inference::cloud::CloudProvider;

/// Default model (see `image::default_cloud_model`); the user can pick a
/// newer one live via `commands::image::image_cloud_models`.
const TIMEOUT_SECS: u64 = 120;
const MAX_PNG_BYTES: usize = 25 * 1024 * 1024;

pub async fn generate(req: ImageRequest, model: &str) -> AppResult<ImageResult> {
    let api_key = tokio::task::spawn_blocking(move || crate::byok::read(CloudProvider::Google))
        .await
        .map_err(|_| AppError::Other("Google image credential lookup failed".into()))??
        .filter(|key| !key.trim().is_empty())
        .ok_or_else(|| {
            AppError::Config("configure a Google API key before generating images".into())
        })?;

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(TIMEOUT_SECS))
        .build()
        .map_err(|_| AppError::Other("Google image HTTP client setup failed".into()))?;

    let url =
        format!("https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent");
    let response = client
        .post(&url)
        .query(&[("key", &api_key)])
        .json(&serde_json::json!({
            "contents": [{"parts": [{"text": req.prompt}]}],
        }))
        .send()
        .await
        .map_err(|_| AppError::Other("Google image request failed".into()))?;
    let status = response.status();
    if response
        .content_length()
        .is_some_and(|n| n > 40 * 1024 * 1024)
    {
        return Err(AppError::Other(
            "Google image response exceeded size limit".into(),
        ));
    }
    let body = response
        .text()
        .await
        .map_err(|_| AppError::Other("Google image response read failed".into()))?;
    if !status.is_success() {
        return Err(AppError::Other(format!(
            "Google image request failed ({status}): {}",
            crate::inference::cloud::sanitize_error_body(&body)
        )));
    }

    let image_b64 = parse_image_b64(&body)?;
    Ok(ImageResult {
        image_b64,
        provider: "google".into(),
        model: model.into(),
        seed: 0,
        steps: 0,
        cfg_scale: 0.0,
        width: 1024,
        height: 1024,
        sampler: "gemini-image".into(),
        scheduler: None,
        prompt_hash: String::new(),
    })
}

fn parse_image_b64(body: &str) -> AppResult<String> {
    let parsed: serde_json::Value = serde_json::from_str(body)
        .map_err(|_| AppError::Other("Google image response was invalid".into()))?;
    let b64 = parsed
        .pointer("/candidates/0/content/parts/0/inlineData/data")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::Other("Google image response had no image data".into()))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|_| AppError::Other("Google image response contained invalid base64".into()))?;
    validate_png_payload(&bytes, MAX_PNG_BYTES)?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}
