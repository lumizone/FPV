//! Tauri commands for the Hugging Face model browser.

use crate::error::AppResult;
use crate::inference::huggingface::{self, HfModel, HfQuant};

#[tauri::command]
pub async fn hf_search(query: String) -> AppResult<Vec<HfModel>> {
    huggingface::search(&query).await
}

#[tauri::command]
pub async fn hf_quants(repo: String) -> AppResult<Vec<HfQuant>> {
    huggingface::quants(&repo).await
}
