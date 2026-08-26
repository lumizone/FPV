use tauri::State;

use crate::error::AppResult;
use crate::inference::HardwareInfo;
use crate::state::AppState;

#[tauri::command]
pub async fn hardware_detect(state: State<'_, AppState>) -> AppResult<HardwareInfo> {
    Ok(state.hardware.clone())
}

/// GPU-acceleration status for the two independent Windows GPU-runtime
/// subsystems: Ollama's own runner set (`sidecars::ollama_gpu_runtime`)
/// and the local image backend's sd.cpp build (`sidecars::gpu_runtime`).
/// `platform_supported` is `false` on macOS, where nothing here applies —
/// Metal is compiled into both binaries, so there is no provisioning step
/// to report on.
///
/// Single always-registered command with a platform-gated body, not a
/// `#[cfg(windows)]`-only command with a stub twin: `tauri::generate_handler!`
/// does not honor `#[cfg]` on individual entries, so a command that only
/// exists on one platform must still be a single stable name whose BODY
/// branches — never two commands under different names.
#[derive(Debug, serde::Serialize)]
pub struct GpuRuntimeStatus {
    pub platform_supported: bool,
    pub vendor: String,
    pub ollama_accelerated: bool,
    pub image_accelerated: bool,
}

#[tauri::command]
pub async fn gpu_runtime_status() -> AppResult<GpuRuntimeStatus> {
    #[cfg(windows)]
    {
        let vendor = crate::sidecars::gpu_runtime::detect_vendor();
        let ollama_accelerated = match vendor {
            // The shipped Windows bundle already includes cuda_v12 for
            // supported NVIDIA cards; downloaded runtime is only needed for
            // newer cards. Status reports actual acceleration, not merely a
            // successful optional download.
            crate::sidecars::gpu_runtime::GpuVendor::Nvidia => {
                !crate::sidecars::ollama_gpu_runtime::nvidia_needs_runtime_fetch()
                    || crate::sidecars::ollama_gpu_runtime::provisioned_runtime_dir().is_some()
            }
            crate::sidecars::gpu_runtime::GpuVendor::Amd => {
                crate::sidecars::ollama_gpu_runtime::provisioned_runtime_dir().is_some()
            }
            crate::sidecars::gpu_runtime::GpuVendor::Other => false,
        };
        Ok(GpuRuntimeStatus {
            platform_supported: true,
            vendor: vendor.tag().to_string(),
            ollama_accelerated,
            image_accelerated: crate::sidecars::gpu_runtime::current_sd_cli_path().is_some(),
        })
    }
    #[cfg(not(windows))]
    {
        Ok(GpuRuntimeStatus {
            platform_supported: false,
            vendor: "n/a".into(),
            ollama_accelerated: false,
            image_accelerated: false,
        })
    }
}

/// User-visible "Retry GPU setup" for Ollama's own runner — the backend
/// half of the Settings action. No-op on macOS.
#[tauri::command]
pub async fn gpu_runtime_retry_ollama(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> AppResult<()> {
    #[cfg(windows)]
    {
        crate::sidecars::ollama_gpu_runtime::reprovision(&app, &state.db).await
    }
    #[cfg(not(windows))]
    {
        let _ = (app, state);
        Ok(())
    }
}

/// User-visible "Retry GPU setup" for the local image backend's sd.cpp
/// build — the backend half of the Settings action. No-op on macOS.
#[tauri::command]
pub async fn gpu_runtime_retry_image(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> AppResult<()> {
    #[cfg(windows)]
    {
        // Retry is deliberately subject to the same provider + weights gates
        // as boot provisioning; a button must not bypass cloud-only privacy
        // or trigger a large download before the selected checkpoint exists.
        let (local, model) = {
            let conn = state.db.lock().await;
            let provider = crate::db::meta_get(&conn, "image_provider")
                .ok()
                .flatten()
                .and_then(|s| crate::image::ImageProvider::from_str(&s))
                .unwrap_or_default();
            (
                matches!(provider, crate::image::ImageProvider::Local),
                crate::commands::image::read_local_model(&conn),
            )
        };
        if !local || crate::image::local::weights_status_of_async(model).await != Some(true) {
            return Ok(());
        }
        crate::sidecars::gpu_runtime::fetch_gpu_runtime(&app).await
    }
    #[cfg(not(windows))]
    {
        let _ = (app, state);
        Ok(())
    }
}
