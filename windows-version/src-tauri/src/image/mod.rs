//! FPV image generation — local stable-diffusion.cpp and optional OpenAI Images.
//! Generates world covers and narrative scene illustrations.

pub mod flux;
pub mod imagen;
pub mod local;
pub mod openai;
pub mod sdcpp;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::inference::cloud::CloudProvider;

pub(crate) async fn read_response_limited(
    response: reqwest::Response,
    max_bytes: usize,
    error: &'static str,
) -> AppResult<Vec<u8>> {
    use futures_util::StreamExt;

    if response
        .content_length()
        .is_some_and(|size| size > max_bytes as u64)
    {
        return Err(AppError::Other(error.into()));
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| AppError::Other(error.into()))?;
        if bytes.len().saturating_add(chunk.len()) > max_bytes {
            return Err(AppError::Other(error.into()));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

pub(crate) fn validate_png_payload(bytes: &[u8], max_bytes: usize) -> AppResult<()> {
    const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if bytes.len() > max_bytes {
        return Err(AppError::Config("PNG payload is too large".into()));
    }
    if bytes.len() < 33 || &bytes[..8] != PNG_SIGNATURE || &bytes[12..16] != b"IHDR" {
        return Err(AppError::Config("payload is not a valid PNG".into()));
    }
    let ihdr_len = u32::from_be_bytes(bytes[8..12].try_into().unwrap());
    if ihdr_len != 13 { return Err(AppError::Config("PNG IHDR is invalid".into())); }
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    if width == 0 || height == 0 || width > 8192 || height > 8192 { return Err(AppError::Config("PNG has invalid dimensions".into())); }
    let mut offset = 8usize;
    let mut saw_ihdr = false;
    let mut saw_iend = false;
    while offset < bytes.len() {
        if bytes.len() - offset < 12 { return Err(AppError::Config("PNG chunk is truncated".into())); }
        let len = u32::from_be_bytes(bytes[offset..offset+4].try_into().unwrap()) as usize;
        let end = offset.checked_add(12).and_then(|n| n.checked_add(len)).ok_or_else(|| AppError::Config("PNG chunk is too large".into()))?;
        if end > bytes.len() { return Err(AppError::Config("PNG chunk is truncated".into())); }
        let kind = &bytes[offset+4..offset+8];
        if !saw_ihdr && kind != b"IHDR" { return Err(AppError::Config("PNG IHDR must be first".into())); }
        if kind == b"IHDR" { if saw_ihdr || len != 13 { return Err(AppError::Config("PNG IHDR is invalid".into())); } saw_ihdr = true; }
        if kind == b"IEND" { if len != 0 { return Err(AppError::Config("PNG IEND is invalid".into())); } saw_iend = true; if end != bytes.len() { return Err(AppError::Config("PNG has trailing data".into())); } break; }
        offset = end;
    }
    if !saw_ihdr || !saw_iend { return Err(AppError::Config("PNG is missing IEND".into())); }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ImageProvider {
    #[default]
    Local,
    Openai,
    Seedream,
    Hunyuan,
    CogView,
    Flux,
    Imagen,
}

impl ImageProvider {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "local" => Some(Self::Local),
            "openai" => Some(Self::Openai),
            "seedream" => Some(Self::Seedream),
            "hunyuan" => Some(Self::Hunyuan),
            "cogview" => Some(Self::CogView),
            "flux" => Some(Self::Flux),
            "imagen" => Some(Self::Imagen),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Openai => "openai",
            Self::Seedream => "seedream",
            Self::Hunyuan => "hunyuan",
            Self::CogView => "cogview",
            Self::Flux => "flux",
            Self::Imagen => "imagen",
        }
    }

    /// Cloud (non-local) image providers use a BYOK key + HTTP endpoint.
    pub const fn is_cloud(self) -> bool {
        !matches!(self, Self::Local)
    }
}

#[derive(Clone)]
pub struct ImageRequest {
    pub prompt: String,
    pub style: ImageStyle,
    /// Optional reference image (base64) for img2img seed.
    pub reference_image_b64: Option<String>,
    /// Sampling steps for the LOCAL backend (sd.cpp).
    pub local_steps: Option<u32>,
    /// Which local checkpoint to render with.
    pub local_model: Option<String>,
    /// Kontext-style reference images (base64), 0-2 entries.
    pub kontext_refs_b64: Vec<String>,
    /// Extra terms appended to the LOCAL backend's negative prompt.
    pub extra_negative: Option<String>,
    pub seed: Option<i64>,
}

impl std::fmt::Debug for ImageRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageRequest")
            .field("prompt", &self.prompt)
            .field("style", &self.style)
            .field(
                "reference_image_b64",
                &self
                    .reference_image_b64
                    .as_ref()
                    .map(|s| format!("<{} bytes>", s.len())),
            )
            .field("local_steps", &self.local_steps)
            .field("local_model", &self.local_model)
            .field("kontext_refs_b64_count", &self.kontext_refs_b64.len())
            .field("extra_negative", &self.extra_negative)
            .finish()
    }
}

/// Minimum system RAM for the local backend (12 GB).
pub const fn local_min_ram_gb() -> u64 {
    12
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageStyle {
    #[default]
    Anime,
    Photo,
    Realistic,
    Watercolor,
    Ink,
    Cinematic,
    DarkFantasy,
    Manga,
    Raw,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImageResult {
    pub image_b64: String,
    pub provider: String,
    pub model: String,
    pub seed: i64,
    pub steps: u32,
    pub cfg_scale: f32,
    pub width: u32,
    pub height: u32,
    pub sampler: String,
    pub scheduler: Option<String>,
    pub prompt_hash: String,
}

/// Dispatch an image generation request. `cloud_model` is the user's saved
/// model choice for cloud providers (None = that provider's default);
/// ignored by the local backend.
pub async fn generate(
    provider: ImageProvider,
    req: ImageRequest,
    cloud_model: Option<&str>,
) -> AppResult<ImageResult> {
    let model = cloud_model.unwrap_or_else(|| default_cloud_model(provider));
    match provider {
        ImageProvider::Local => local::generate(req).await,
        ImageProvider::Openai => openai::generate(req, model).await,
        ImageProvider::Seedream | ImageProvider::Hunyuan | ImageProvider::CogView => {
            let (base_url, key_provider) = openai_compat_image_config(provider);
            openai::generate_compat(req, base_url, model, key_provider).await
        }
        ImageProvider::Flux => flux::generate(req, model).await,
        ImageProvider::Imagen => imagen::generate(req, model).await,
    }
}

/// Default cloud image model per provider. Used when the user has not picked
/// one explicitly (Settings → Image Generation → Cloud model). Kept as a
/// fallback so a fresh install works even if a model-list API is unreachable;
/// the live lists from `commands::image::image_cloud_models` supersede these
/// so a newly released provider model needs no app update.
pub fn default_cloud_model(provider: ImageProvider) -> &'static str {
    match provider {
        ImageProvider::Local => "local",
        ImageProvider::Openai => "gpt-image-2",
        ImageProvider::Seedream => "doubao-seedream-4-0",
        ImageProvider::Hunyuan => "hunyuan-image",
        ImageProvider::CogView => "cogview-3-flash",
        ImageProvider::Flux => "flux-pro-1.1",
        ImageProvider::Imagen => "gemini-2.5-flash-image",
    }
}

/// app_meta key holding the user's explicit cloud image model choice.
pub fn meta_key_for_model(provider: ImageProvider) -> &'static str {
    match provider {
        ImageProvider::Local => "image_model_local",
        ImageProvider::Openai => "image_model_openai",
        ImageProvider::Seedream => "image_model_seedream",
        ImageProvider::Hunyuan => "image_model_hunyuan",
        ImageProvider::CogView => "image_model_cogview",
        ImageProvider::Flux => "image_model_flux",
        ImageProvider::Imagen => "image_model_imagen",
    }
}

/// OpenAI-compatible `/images/generations` endpoint per provider:
/// (base URL, which BYOK provider holds the API key). The model is resolved
/// separately via `default_cloud_model` + the user's saved choice.
fn openai_compat_image_config(provider: ImageProvider) -> (&'static str, CloudProvider) {
    match provider {
        ImageProvider::Seedream => (
            "https://ark.cn-beijing.volces.com/api/v3",
            CloudProvider::Doubao,
        ),
        ImageProvider::Hunyuan => (
            "https://api.hunyuan.cloud.tencent.com/v1",
            CloudProvider::Hunyuan,
        ),
        ImageProvider::CogView => (
            "https://open.bigmodel.cn/api/paas/v4",
            CloudProvider::Zhipu,
        ),
        _ => unreachable!("only OpenAI-compatible image providers reach here"),
    }
}

#[cfg(test)]
mod tests {
    use super::{validate_png_payload, ImageProvider};

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        bytes.extend(width.to_be_bytes());
        bytes.extend(height.to_be_bytes());
        bytes.extend([8, 6, 0, 0, 0]);
        bytes.extend([0, 0, 0, 0]);
        bytes.extend([0, 0, 0, 0, b'I', b'E', b'N', b'D', 0, 0, 0, 0]);
        bytes
    }

    #[test]
    fn png_payload_rejects_non_png_and_invalid_dimensions() {
        assert!(validate_png_payload(b"not an image", 1024).is_err());
        assert!(validate_png_payload(&png(0, 512), 1024).is_err());
        assert!(validate_png_payload(&png(512, 512), 1024).is_ok());
    }

    #[test]
    fn image_provider_parses_and_serializes_openai() {
        assert_eq!(
            ImageProvider::from_str("OPENAI"),
            Some(ImageProvider::Openai)
        );
        assert_eq!(ImageProvider::Openai.as_str(), "openai");
    }
}
