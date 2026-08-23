//! `system_status` — reports which capabilities are live so the UI can
//! surface degraded state instead of failing silently.

use serde::Serialize;
use tauri::State;

use crate::error::AppResult;
use crate::state::AppState;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SystemStatus {
    /// The bundled Ollama daemon answered.
    pub ollama_up: bool,
    /// A usable chat model exists (installed locally).
    pub chat_model_present: bool,
    /// Image generation can run (local sd.cpp ready).
    pub image_provider_ready: bool,
}

/// Live capability snapshot. Best-effort — every probe degrades to `false`
/// on error rather than failing the whole call.
#[tauri::command]
pub async fn system_status(state: State<'_, AppState>) -> AppResult<SystemStatus> {
    let list = state.ollama.list_models().await;
    let ollama_up = list.is_ok();
    let installed: Vec<String> = list
        .map(|v| v.into_iter().map(|m| m.name).collect())
        .unwrap_or_default();

    let chat_model = {
        let conn = state.db.lock().await;
        crate::db::meta_get(&conn, "user_chat_model")?
    }
    .unwrap_or_else(|| state.hardware.recommended_chat_model.clone());

    let chat_model_present = installed.iter().any(|m| {
        let bare = m.strip_suffix(":latest").unwrap_or(m);
        let target = chat_model.strip_suffix(":latest").unwrap_or(&chat_model);
        bare == target
    });

    let local_model = {
        let conn = state.db.lock().await;
        crate::commands::image::read_local_model(&conn)
    };
    let (_, ram_gb) = crate::image::local::check();
    let image_provider_ready =
        ram_gb >= local_model.min_ram_gib() && crate::image::sdcpp::weights_ready_for(local_model);

    Ok(SystemStatus {
        ollama_up,
        chat_model_present,
        image_provider_ready,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_defaults() {
        let s = SystemStatus {
            ollama_up: false,
            chat_model_present: false,
            image_provider_ready: false,
        };
        assert!(!s.ollama_up);
        assert!(!s.chat_model_present);
        assert!(!s.image_provider_ready);
    }
}
