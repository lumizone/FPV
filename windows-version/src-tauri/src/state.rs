use std::collections::HashMap;
use std::sync::Arc;

use futures_util::future::AbortHandle;
use tokio::sync::Mutex;

use crate::db::DbHandle;
use crate::error::AppResult;
use crate::inference::{HardwareInfo, OllamaClient};
use crate::sidecars::SidecarManager;

pub struct AppState {
    pub db: DbHandle,
    pub sidecars: Arc<SidecarManager>,
    pub ollama: OllamaClient,
    pub hardware: HardwareInfo,
    pub narration_cancel: tokio::sync::broadcast::Sender<String>,
    pub model_pull_cancellations: Mutex<HashMap<String, AbortHandle>>,
}

impl AppState {
    pub async fn ensure_ollama(&self, app: &tauri::AppHandle) -> AppResult<()> {
        self.sidecars.ensure_ollama(app, &self.ollama).await
    }

    pub fn context_limit(&self) -> i64 {
        match self.hardware.ram_gb {
            0..=16 => 8_192,
            17..=32 => 16_384,
            _ => 32_768,
        }
    }
}
