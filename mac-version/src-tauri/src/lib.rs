mod app_menu;
mod byok;
mod commands;
mod crash_log;
mod db;
mod error;
mod image;
mod inference;
mod log_file;
mod metrics;
mod secret_store;
mod sidecars;
mod state;
mod storage;
pub mod story;
mod tray;

use std::sync::Arc;

use rand::RngCore;
use tauri::Manager;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use crate::inference::{hardware_tier, OllamaClient};
use crate::sidecars::SidecarManager;
use crate::state::AppState;

const META_HW_UUID: &str = "hardware_uuid";
const META_CRYPTO_SALT_HEX: &str = "crypto_salt_hex";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_tracing();
    crash_log::install_panic_hook();

    let db = match db::open() {
        Ok(handle) => handle,
        Err(err) => {
            error!(?err, "failed to open database");
            std::process::exit(1);
        }
    };

    if let Err(err) = init_security_meta(&db) {
        error!(?err, "failed to init security meta");
        std::process::exit(1);
    }

    // Apply the `useSystemProxy` preference once at boot.
    {
        let conn = db.blocking_lock();
        let use_proxy = db::meta_get(&conn, "useSystemProxy")
            .ok()
            .flatten()
            .map(|v| v != "false")
            .unwrap_or(true);
        if !use_proxy {
            for var in [
                "HTTP_PROXY",
                "HTTPS_PROXY",
                "ALL_PROXY",
                "http_proxy",
                "https_proxy",
                "all_proxy",
            ] {
                std::env::remove_var(var);
            }
            info!("useSystemProxy=false — proxy env vars stripped");
        }
    }

    let hardware = match hardware_tier::detect() {
        Ok(h) => {
            info!(chip = %h.chip, ram_gb = h.ram_gb, tier = ?h.tier, "hardware detected");
            h
        }
        Err(err) => {
            warn!(?err, "hardware detection failed, falling back to LIGHT");
            hardware_tier::HardwareInfo {
                chip: "unknown".into(),
                ram_gb: 16,
                tier: hardware_tier::Tier::Light,
                recommended_chat_model: hardware_tier::Tier::Light.default_chat_model().into(),
            }
        }
    };

    let ollama = match OllamaClient::new() {
        Ok(client) => client,
        Err(err) => {
            tracing::error!(?err, "failed to build Ollama client — aborting boot");
            return;
        }
    };
    let sidecars = SidecarManager::new();
    let (narration_cancel, _) = tokio::sync::broadcast::channel(8);

    let state = AppState {
        db,
        sidecars: Arc::clone(&sidecars),
        ollama: ollama.clone(),
        hardware,
        narration_cancel,
        model_pull_cancellations: tokio::sync::Mutex::new(std::collections::HashMap::new()),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .on_menu_event(|app, event| app_menu::on_menu_event(app, event.id().as_ref()))
        .setup(move |app| {
            let handle = app.handle().clone();

            // A previous hard crash may have killed the daemon but left one
            // or more model runners behind. Cleanup is restricted to orphaned
            // executables inside FPV.app bundles.
            crate::sidecars::ollama::cleanup_orphaned_runners();

            // Native macOS app menu.
            match app_menu::build(&handle) {
                Ok(menu) => {
                    if let Err(err) = handle.set_menu(menu) {
                        warn!(?err, "failed to apply app menu");
                    }
                }
                Err(err) => warn!(?err, "failed to build app menu"),
            }

            // Menu-bar tray icon.
            if let Err(err) = tray::create(&handle) {
                warn!(
                    ?err,
                    "tray icon creation failed; menu-bar entry unavailable"
                );
            }

            // Seed FPV preset worlds on first run.
            {
                let db = app.state::<AppState>().inner().db.clone();
                let conn = db.blocking_lock();
                let resource_dir = app.path().resource_dir();
                if let Ok(ref dir) = resource_dir {
                    if let Err(err) = crate::story::seed::seed_if_empty(&conn, dir) {
                        tracing::warn!(?err, "seed preset worlds failed (non-fatal)");
                    }
                    // Also backfill covers for worlds seeded by older builds
                    // (non-fatal — a cover-less card just falls back to the
                    // genre swatch).
                    if let Err(err) = crate::story::seed::backfill_seed_covers(&conn, dir) {
                        tracing::warn!(?err, "seed cover backfill failed (non-fatal)");
                    }
                } else {
                    tracing::warn!(?resource_dir, "no resource dir — cannot seed preset worlds");
                }
            }

            Ok(())
        })
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                if let Some(state) = window.app_handle().try_state::<AppState>() {
                    let hide = read_hide_to_tray(&state.db);
                    if hide {
                        api.prevent_close();
                        let _ = window.hide();
                        let app = window.app_handle().clone();
                        tauri::async_runtime::spawn(async move {
                            if let Some(state) = app.try_state::<AppState>() {
                                state.sidecars.unload_owned(&state.ollama).await;
                            }
                        });
                    }
                }
            }
            tauri::WindowEvent::Destroyed => {
                let app = window.app_handle().clone();
                tauri::async_runtime::spawn(async move {
                    if let Some(state) = app.try_state::<AppState>() {
                        if let Err(err) = state.sidecars.shutdown_all(&state.ollama).await {
                            warn!(?err, "sidecar shutdown error");
                        }
                    }
                });
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            commands::hardware::hardware_detect,
            commands::models::model_list,
            commands::models::model_pull,
            commands::models::model_pull_cancel,
            commands::models::model_delete,
            commands::models::model_set_default,
            commands::models::model_prewarm_active,
            commands::models::model_unload_active,
            commands::models::codex_status,
            commands::models::codex_login,
            commands::models::codex_model_get,
            commands::models::codex_model_set,
            commands::byok::byok_save,
            commands::byok::byok_delete,
            commands::byok::byok_list,
            commands::byok::byok_get_base_url,
            commands::settings::setting_get,
            commands::settings::setting_get_all,
            commands::settings::setting_set,
            commands::advanced::logs_path,
            commands::advanced::app_log_path,
            commands::advanced::diagnostics_write,
            commands::advanced::log_renderer_crash,
            commands::advanced::reset_all_data,
            commands::image::image_generate,
            commands::image::image_generation_cancel,
            commands::image::image_download_cancel,
            commands::image::image_provider_get,
            commands::image::image_provider_set,
            commands::image::image_cloud_models,
            commands::image::image_cloud_model_get,
            commands::image::image_cloud_model_set,
            commands::image::image_local_check,
            commands::image::image_local_prewarm,
            commands::image::image_local_models,
            commands::image::image_local_model_get,
            commands::image::image_local_model_set,
            commands::image::image_local_model_delete,
            commands::update_safety::pre_update_check,
            commands::update_safety::backup_app_data,
            commands::update_safety::post_update_finalize,
            commands::system::system_ready_get,
            commands::system::system_energy_get,
            commands::system_status::system_status,
            // FPV story commands
            commands::story::world_create,
            commands::story::world_list,
            commands::story::world_get,
            commands::story::world_delete,
            commands::story::world_save,
            commands::story::session_create,
            commands::story::session_delete,
            commands::story::session_branch,
            commands::story::session_checkpoint,
            commands::story::session_fork_from_message,
            commands::story::session_list_messages,
            commands::story::session_get_summary,
            commands::story::session_append_message,
            commands::story::message_update,
            commands::story::message_replace_content,
            commands::story::narration_prepare_regeneration,
            commands::story::story_state_get,
            commands::story::story_state_set,
            commands::story::world_visual_bible_get,
            commands::story::world_visual_bible_set,
            commands::story::story_pinned_canon_list,
            commands::story::story_pinned_canon_add,
            commands::story::story_pinned_canon_delete,
            commands::story::session_update_summary,
            commands::story::session_list_for_world,
            commands::story::session_undo,
            commands::story::codex_upsert,
            commands::story::codex_list_for_world,
            commands::story::world_import_json,
            commands::story::world_upload_cover,
            commands::project::project_export,
            commands::project::project_import,
            commands::scene::scene_image_save,
            commands::scene::scene_image_list,
            commands::memory::story_memory_index,
            commands::memory::story_memory_recall,
            commands::memory::story_memory_list,
            commands::metrics::metrics_read,
            commands::metrics::metrics_reset,
            commands::cloud::cloud_list_models,
            commands::huggingface::hf_search,
            commands::huggingface::hf_quants,
            // Character commands
            commands::characters::character_create,
            commands::characters::character_list,
            commands::characters::character_get,
            commands::characters::character_update,
            commands::characters::character_delete,
            commands::narration::narration_generate,
            commands::narration::narration_generate_stream,
            commands::narration::narration_cancel,
            commands::narration::story_state_extract,
            commands::narration_image::world_generate_cover,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                if let Some(state) = app.try_state::<AppState>() {
                    let sc = state.sidecars.clone();
                    let _ = tauri::async_runtime::block_on(async move {
                        tokio::time::timeout(
                            std::time::Duration::from_secs(5),
                            sc.shutdown_all(&state.ollama),
                        )
                        .await
                    });
                }
            }
        });
}

fn read_hide_to_tray(db: &db::DbHandle) -> bool {
    let conn = db.blocking_lock();
    db::meta_get(&conn, "hideToTray")
        .ok()
        .flatten()
        .map(|v| v != "false")
        .unwrap_or(false)
}

fn init_tracing() {
    use tracing_subscriber::layer::SubscriberExt as _;
    use tracing_subscriber::util::SubscriberInitExt as _;

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,fpv_desktop_lib=debug"));
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_target(true))
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(true)
                .with_ansi(false)
                .with_writer(crate::log_file::AppLogWriter),
        )
        .try_init();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "log opened");
}

fn init_security_meta(db: &db::DbHandle) -> error::AppResult<(String, Vec<u8>)> {
    let conn = db.blocking_lock();

    let hw = if let Some(stored) = db::meta_get(&conn, META_HW_UUID)? {
        stored
    } else {
        let v = uuid::Uuid::new_v4().to_string();
        db::meta_set(&conn, META_HW_UUID, &v)?;
        v
    };

    let salt = if let Some(hex) = db::meta_get(&conn, META_CRYPTO_SALT_HEX)? {
        hex_decode(&hex).unwrap_or_else(|_| generate_salt())
    } else {
        let s = generate_salt();
        db::meta_set(&conn, META_CRYPTO_SALT_HEX, &hex_encode(&s))?;
        s
    };

    Ok((hw, salt))
}

fn generate_salt() -> Vec<u8> {
    let mut buf = vec![0u8; 32];
    rand::thread_rng().fill_bytes(&mut buf);
    buf
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn hex_decode(s: &str) -> Result<Vec<u8>, ()> {
    if s.len() % 2 != 0 {
        return Err(());
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    for i in (0..s.len()).step_by(2) {
        let byte = u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| ())?;
        out.push(byte);
    }
    Ok(out)
}
