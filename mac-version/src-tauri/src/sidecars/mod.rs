pub mod ollama;

use std::sync::Arc;

use tauri::AppHandle;
use tokio::sync::Mutex;
use tracing::warn;

use crate::error::AppResult;
use crate::inference::OllamaClient;

pub struct SidecarManager {
    pub ollama: Mutex<Option<ollama::OllamaSidecar>>,
}

impl SidecarManager {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            ollama: Mutex::new(None),
        })
    }

    /// Serialize cold starts so concurrent frontend requests cannot spawn two
    /// bundled daemons.
    pub async fn ensure_ollama(&self, app: &AppHandle, client: &OllamaClient) -> AppResult<()> {
        let mut owned = self.ollama.lock().await;
        if client.ping().await.unwrap_or(false) {
            return Ok(());
        }
        if let Some(sidecar) = ollama::ensure(app, client).await? {
            owned.replace(sidecar);
        }
        Ok(())
    }

    pub async fn shutdown_all(&self, client: &OllamaClient) -> AppResult<()> {
        // Before Ollama, because it is the one with a grace period: an
        // sd-cli render left running here outlives the app entirely.
        crate::image::sdcpp::kill_active_render().await;
        if let Some(o) = self.ollama.lock().await.take() {
            if let Err(err) = o.kill(client).await {
                warn!(?err, "ollama kill failed");
            }
        }
        Ok(())
    }

    pub async fn unload_owned(&self, client: &OllamaClient) {
        if self.ollama.lock().await.is_some() {
            if let Err(err) = client.unload_all().await {
                warn!(?err, "ollama unload while hidden failed");
            }
        }
    }
}
