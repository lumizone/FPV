//! Official OpenAI Images API client. This intentionally uses no configurable
//! endpoint: image requests always go to OpenAI over HTTPS.

use base64::Engine;
use serde::Deserialize;

use crate::error::{AppError, AppResult};
use crate::image::{validate_png_payload, ImageRequest, ImageResult};
use crate::inference::cloud::CloudProvider;

const IMAGES_URL: &str = "https://api.openai.com/v1/images/generations";
/// Default model (see `image::default_cloud_model`); the user can pick a
/// newer one live via `commands::image::image_cloud_models`.
const TIMEOUT_SECS: u64 = 120;
const MAX_PNG_BYTES: usize = 25 * 1024 * 1024;
const MAX_B64_BYTES: usize = 34 * 1024 * 1024;

#[derive(Deserialize)]
struct ImagesResponse {
    data: Vec<ImageData>,
}

#[derive(Deserialize)]
struct ImageData {
    b64_json: Option<String>,
}

pub async fn generate(req: ImageRequest, model: &str) -> AppResult<ImageResult> {
    let api_key = tokio::task::spawn_blocking(|| {
        crate::byok::read(crate::inference::cloud::CloudProvider::Openai)
    })
    .await
    .map_err(|_| AppError::Other("OpenAI image credential lookup failed".into()))??
    .filter(|key| !key.trim().is_empty())
    .ok_or_else(|| {
        AppError::Config("configure an OpenAI API key before generating images".into())
    })?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| AppError::Other("OpenAI image HTTP client setup failed".into()))?;
    let response = client
        .post(IMAGES_URL)
        .bearer_auth(api_key)
        .json(&serde_json::json!({
            "model": model,
            "prompt": req.prompt,
            "size": "1024x1024",
            "output_format": "png",
        }))
        .send()
        .await
        .map_err(|_| AppError::Other("OpenAI image request failed".into()))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|_| AppError::Other("OpenAI image response read failed".into()))?;
    if !status.is_success() {
        return Err(AppError::Other(format!(
            "OpenAI image request failed ({status}): {}",
            crate::inference::cloud::sanitize_error_body(&body)
        )));
    }

    let image_b64 = parse_image_b64(&body)?;
    Ok(ImageResult {
        image_b64,
        provider: "openai".into(),
        model: model.into(),
        seed: 0,
        steps: 0,
        cfg_scale: 0.0,
        width: 1024,
        height: 1024,
        sampler: "openai".into(),
        scheduler: None,
        prompt_hash: String::new(),
    })
}

fn parse_image_b64(body: &str) -> AppResult<String> {
    let parsed: ImagesResponse = serde_json::from_str(body)
        .map_err(|_| AppError::Other("OpenAI image response was invalid".into()))?;
    let encoded = parsed
        .data
        .first()
        .and_then(|image| image.b64_json.as_deref())
        .ok_or_else(|| AppError::Other("OpenAI image response contained no image data".into()))?;
    if encoded.len() > MAX_B64_BYTES {
        return Err(AppError::Config("OpenAI image payload is too large".into()));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| AppError::Other("OpenAI image response contained invalid base64".into()))?;
    validate_png_payload(&bytes, MAX_PNG_BYTES)?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// OpenAI-compatible image generation against a configurable provider.
/// `key_provider` is the BYOK account the API key is read from; `base_url`
/// and `model` come from `openai_compat_image_config` in `image::mod`.
pub async fn generate_compat(
    req: ImageRequest,
    base_url: &str,
    model: &str,
    key_provider: CloudProvider,
) -> AppResult<ImageResult> {
    let api_key = tokio::task::spawn_blocking(move || crate::byok::read(key_provider))
        .await
        .map_err(|_| AppError::Other("image credential lookup failed".into()))??
        .filter(|key| !key.trim().is_empty())
        .ok_or_else(|| {
            AppError::Config("configure a provider API key before generating images".into())
        })?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| AppError::Other("image HTTP client setup failed".into()))?;

    let url = format!("{}/images/generations", base_url.trim_end_matches('/'));
    let response = client
        .post(&url)
        .bearer_auth(&api_key)
        .json(&serde_json::json!({
            "model": model,
            "prompt": req.prompt,
            "size": "1024x1024",
        }))
        .send()
        .await
        .map_err(|_| AppError::Other("image request failed".into()))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|_| AppError::Other("image response read failed".into()))?;
    if !status.is_success() {
        return Err(AppError::Other(format!(
            "image request failed ({status}): {}",
            crate::inference::cloud::sanitize_error_body(&body)
        )));
    }

    let image_b64 = parse_image_data(&client, &body).await?;
    Ok(ImageResult {
        image_b64,
        provider: key_provider.as_str().into(),
        model: model.into(),
        seed: 0,
        steps: 0,
        cfg_scale: 0.0,
        width: 1024,
        height: 1024,
        sampler: "openai-compat".into(),
        scheduler: None,
        prompt_hash: String::new(),
    })
}

/// Parse an OpenAI-compatible `/images/generations` response that carries the
/// PNG either inline (`b64_json`) or as a `url` we must download.
async fn parse_image_data(client: &reqwest::Client, body: &str) -> AppResult<String> {
    let parsed: serde_json::Value = serde_json::from_str(body)
        .map_err(|_| AppError::Other("image response was invalid".into()))?;
    let data = parsed
        .pointer("/data/0")
        .ok_or_else(|| AppError::Other("image response contained no image data".into()))?;
    if let Some(b64) = data.get("b64_json").and_then(|v| v.as_str()) {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|_| AppError::Other("image response contained invalid base64".into()))?;
        validate_png_payload(&bytes, MAX_PNG_BYTES)?;
        return Ok(base64::engine::general_purpose::STANDARD.encode(bytes));
    }
    if let Some(url) = data.get("url").and_then(|v| v.as_str()) {
        let parsed = url::Url::parse(url)
            .map_err(|_| AppError::Other("image URL was invalid".into()))?;
        crate::image::validate_public_image_url(&parsed).await?;
        let response = client
            .get(parsed)
            .send()
            .await
            .map_err(|_| AppError::Other("image download failed".into()))?;
        if !response.status().is_success() {
            return Err(AppError::Other("image download returned an error".into()));
        }
        if response
            .content_length()
            .is_some_and(|size| size > MAX_PNG_BYTES as u64)
        {
            return Err(AppError::Other("image download exceeded size limit".into()));
        }
        use futures_util::StreamExt;
        let mut stream = response.bytes_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| AppError::Other("image download read failed".into()))?;
            if bytes.len().saturating_add(chunk.len()) > MAX_PNG_BYTES {
                return Err(AppError::Other("image download exceeded size limit".into()));
            }
            bytes.extend_from_slice(&chunk);
        }
        validate_png_payload(&bytes, MAX_PNG_BYTES)?;
        return Ok(base64::engine::general_purpose::STANDARD.encode(bytes));
    }
    Err(AppError::Other(
        "image response had neither b64_json nor url".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::parse_image_b64;
    use base64::Engine;

    fn png() -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        bytes.extend(512u32.to_be_bytes());
        bytes.extend(512u32.to_be_bytes());
        bytes.extend([8, 6, 0, 0, 0]);
        bytes.extend([0, 0, 0, 0]);
        bytes.extend([0, 0, 0, 0, b'I', b'E', b'N', b'D', 0, 0, 0, 0]);
        bytes
    }

    #[test]
    fn accepts_and_normalizes_openai_b64_png() {
        let b64 = base64::engine::general_purpose::STANDARD.encode(png());
        let parsed = parse_image_b64(&format!(r#"{{"data":[{{"b64_json":"{b64}"}}]}}"#)).unwrap();
        assert_eq!(parsed, b64);
    }

    #[test]
    fn rejects_missing_or_non_png_openai_data() {
        assert!(parse_image_b64(r#"{"data":[{}]}"#).is_err());
        let b64 = base64::engine::general_purpose::STANDARD.encode(b"not a png");
        assert!(parse_image_b64(&format!(r#"{{"data":[{{"b64_json":"{b64}"}}]}}"#)).is_err());
    }
}
