//! Native Black Forest Labs (FLUX) image client. BFL's API is NOT
//! OpenAI-compatible: it uses an `x-key` header and returns a
//! `result.sample` URL (or a `Pending` status we poll).

use base64::Engine;

use crate::error::{AppError, AppResult};
use crate::image::{read_response_limited, validate_png_payload, ImageRequest, ImageResult};
use crate::inference::cloud::CloudProvider;

const BASE: &str = "https://api.bfl.ml/v1";
/// Default model (see `image::default_cloud_model`); the user can pick
/// another BFL endpoint live in Settings (`commands::image::image_cloud_models`
/// returns a curated BFL list — the API is task-based and has no model-list
/// endpoint).
const TIMEOUT_SECS: u64 = 120;
const MAX_PNG_BYTES: usize = 25 * 1024 * 1024;
const MAX_JSON_BYTES: usize = 4 * 1024 * 1024;

#[derive(serde::Deserialize)]
struct BflResponse {
    id: Option<String>,
    status: Option<String>,
    result: Option<BflResult>,
}

#[derive(serde::Deserialize)]
struct BflResult {
    sample: Option<String>,
}

pub async fn generate(req: ImageRequest, model: &str) -> AppResult<ImageResult> {
    let api_key = tokio::task::spawn_blocking(move || crate::byok::read(CloudProvider::Bfl))
        .await
        .map_err(|_| AppError::Other("FLUX credential lookup failed".into()))??
        .filter(|key| !key.trim().is_empty())
        .ok_or_else(|| {
            AppError::Config("configure a Black Forest Labs key before generating images".into())
        })?;

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(TIMEOUT_SECS))
        .build()
        .map_err(|_| AppError::Other("FLUX HTTP client setup failed".into()))?;

    let url = format!("{BASE}/{model}");
    let response = client
        .post(&url)
        .header("x-key", &api_key)
        .json(&serde_json::json!({
            "prompt": req.prompt,
            "width": 1024,
            "height": 1024,
        }))
        .send()
        .await
        .map_err(|_| AppError::Other("FLUX request failed".into()))?;
    if !response.status().is_success() {
        return Err(AppError::Other("FLUX request failed (non-2xx)".into()));
    }
    let body = read_response_limited(response, MAX_JSON_BYTES, "FLUX response exceeded size limit").await?;
    let resp: BflResponse = serde_json::from_slice(&body)
        .map_err(|_| AppError::Other("FLUX response was invalid".into()))?;

    let sample = match (resp.status.as_deref(), resp.result.and_then(|r| r.sample)) {
        (Some("Ready"), Some(url)) => Some(url),
        (Some("Pending"), _) => poll_for_result(&client, &api_key, resp.id.as_deref()).await?,
        _ => None,
    };
    let sample = sample.ok_or_else(|| AppError::Other("FLUX response had no image URL".into()))?;

    let bytes = client
        .get(&sample)
        .send()
        .await
        .map_err(|_| AppError::Other("FLUX image download failed".into()))?
        .bytes()
        .await
        .map_err(|_| AppError::Other("FLUX image download read failed".into()))?;
    validate_png_payload(&bytes, MAX_PNG_BYTES)?;

    Ok(ImageResult {
        image_b64: base64::engine::general_purpose::STANDARD.encode(bytes),
        provider: "bfl".into(),
        model: model.into(),
        seed: 0,
        steps: 0,
        cfg_scale: 0.0,
        width: 1024,
        height: 1024,
        sampler: "flux".into(),
        scheduler: None,
        prompt_hash: String::new(),
    })
}

async fn poll_for_result(
    client: &reqwest::Client,
    api_key: &str,
    id: Option<&str>,
) -> AppResult<Option<String>> {
    let id = id.ok_or_else(|| AppError::Other("FLUX async job had no id".into()))?;
    for _ in 0..60 {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let resp: BflResponse = client
            .get(format!("{BASE}/get_result"))
            .header("x-key", api_key)
            .query(&[("id", id)])
            .send()
            .await
            .map_err(|_| AppError::Other("FLUX poll failed".into()))?
            .json()
            .await
            .map_err(|_| AppError::Other("FLUX poll response was invalid".into()))?;
        match resp.status.as_deref() {
            Some("Ready") => return Ok(resp.result.and_then(|r| r.sample)),
            Some("Error") => return Err(AppError::Other("FLUX job failed".into())),
            _ => continue,
        }
    }
    Err(AppError::Other("FLUX job timed out".into()))
}
