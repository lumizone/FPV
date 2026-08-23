use serde_json::Value;
use tauri::State;

use crate::error::AppResult;
use crate::state::AppState;

#[tauri::command]
pub async fn metrics_read(_state: State<'_, AppState>) -> AppResult<Vec<Value>> {
    let Some(path) = crate::metrics::metrics_path() else {
        return Ok(Vec::new());
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(Vec::new());
    };
    Ok(text
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .rev()
        .take(500)
        .collect())
}

#[tauri::command]
pub async fn metrics_reset(_state: State<'_, AppState>) -> AppResult<()> {
    if let Some(path) = crate::metrics::metrics_path() {
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("jsonl.old"));
    }
    Ok(())
}
