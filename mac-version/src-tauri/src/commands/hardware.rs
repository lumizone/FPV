use tauri::State;

use crate::error::AppResult;
use crate::inference::HardwareInfo;
use crate::state::AppState;

#[tauri::command]
pub async fn hardware_detect(state: State<'_, AppState>) -> AppResult<HardwareInfo> {
    Ok(state.hardware.clone())
}
