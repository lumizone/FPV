use crate::byok;
use crate::error::{AppError, AppResult};
use crate::inference::cloud::CloudProvider;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct ByokSaveArgs {
    pub provider: String,
    pub api_key: String,
    #[serde(default)]
    pub base_url: Option<String>,
}

#[tauri::command]
pub async fn byok_save(args: ByokSaveArgs) -> AppResult<()> {
    let provider = CloudProvider::from_str(&args.provider)
        .ok_or_else(|| AppError::Config(format!("unknown provider {}", args.provider)))?;
    let key = args.api_key.clone();
    let base_url = args.base_url.clone();
    tokio::task::spawn_blocking(move || {
        if provider == CloudProvider::Custom {
            let url = base_url
                .as_deref()
                .map(str::trim)
                .filter(|u| !u.is_empty())
                .ok_or_else(|| AppError::Config("custom provider requires a base URL".into()))?;
            if !(url.starts_with("https://")
                || (url.starts_with("http://") && byok::is_loopback_base_url(url)))
            {
                return Err(AppError::Config(
                    "base URL must be https:// — plain http:// is only allowed for \
                      localhost (e.g. http://127.0.0.1:1234/v1)"
                        .into(),
                ));
            }
            byok::save_base_url(url)?;
        }
        byok::save(provider, &key)
    })
    .await
    .map_err(|e| AppError::Other(format!("join: {e}")))?
}

#[derive(Debug, Deserialize)]
pub struct ByokDeleteArgs {
    pub provider: String,
}

#[tauri::command]
pub async fn byok_delete(args: ByokDeleteArgs) -> AppResult<()> {
    let provider = CloudProvider::from_str(&args.provider)
        .ok_or_else(|| AppError::Config(format!("unknown provider {}", args.provider)))?;
    tokio::task::spawn_blocking(move || {
        byok::delete(provider)?;
        if provider == CloudProvider::Custom {
            byok::delete_base_url()?;
        }
        Ok(())
    })
    .await
    .map_err(|e| AppError::Other(format!("join: {e}")))?
}

#[derive(Debug, Serialize)]
pub struct ByokListResult {
    pub providers: Vec<&'static str>,
}

#[tauri::command]
pub async fn byok_list() -> AppResult<ByokListResult> {
    let providers = tokio::task::spawn_blocking(byok::list_configured)
        .await
        .map_err(|e| AppError::Other(format!("join: {e}")))??;
    Ok(ByokListResult {
        providers: providers.into_iter().map(|p| p.as_str()).collect(),
    })
}

#[tauri::command]
pub async fn byok_get_base_url() -> AppResult<Option<String>> {
    tokio::task::spawn_blocking(byok::read_base_url)
        .await
        .map_err(|e| AppError::Other(format!("join: {e}")))?
}
