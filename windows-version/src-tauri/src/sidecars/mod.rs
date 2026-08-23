/// Windows-only provisioning of the accelerated stable-diffusion.cpp
/// build. Everything in it is Windows API / Windows path handling, so it
/// is not compiled at all on macOS — see the module's own header for why
/// Windows needs this and macOS does not.
#[cfg(windows)]
pub mod gpu_runtime;
/// Windows-only: kills the bundled Ollama sidecar (and any GPU-runner
/// children) if the app dies abnormally — crash, WebView2 fault, Task
/// Manager kill. See the module's own header for the full rationale.
#[cfg(windows)]
pub mod job_object;
/// Windows-only provisioning of the accelerated Ollama GPU runner (CUDA
/// v13 for Blackwell NVIDIA, or AMD ROCm) — a separate concern from
/// `gpu_runtime`, which provisions the image-generation backend. See the
/// module's own header for the full rationale.
#[cfg(windows)]
pub mod ollama_gpu_runtime;
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
