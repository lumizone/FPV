//! macOS menu-bar tray icon.
//!
//! Behaviour:
//!   - Tray icon sits in the system menu bar (left-click toggles main
//!     window visibility, right-click opens menu).
//!   - Menu has Show / Hide / Quit. Show focuses the window if hidden.
//!   - Close-to-tray itself is wired in `lib.rs::on_window_event`:
//!     when the user closes the window and the `hideToTray` preference
//!     is true, the close is intercepted and the window hidden instead
//!     of destroyed. This module only provides the icon + menu.
//!
//! Locale: labels follow `app_meta["language"]` ("pl" → Polish, else
//! English). Read on startup AND on every language-change pref write
//! (`setting_set` invokes `relabel()` when the key is "language"), so
//! the tray menu now stays in sync with the rest of the UI without
//! requiring a relaunch.

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Runtime,
};

use crate::db::{self, DbHandle};
use crate::state::AppState;

const TRAY_ID: &str = "main-tray";
const TOOLTIP: &str = "First Person Viewpoint";

struct Labels {
    show: &'static str,
    hide: &'static str,
    quit: &'static str,
}

fn labels_for(lang: &str) -> Labels {
    if lang == "pl" {
        Labels {
            show: "Pokaż okno",
            hide: "Ukryj",
            quit: "Zakończ",
        }
    } else {
        Labels {
            show: "Show window",
            hide: "Hide",
            quit: "Quit",
        }
    }
}

fn read_language(app: &AppHandle<impl Runtime>) -> String {
    let Some(state) = app.try_state::<AppState>() else {
        return "en".into();
    };
    read_language_from_db(&state.db)
}

fn read_language_from_db(db: &DbHandle) -> String {
    let conn = db.blocking_lock();
    db::meta_get(&conn, "language")
        .ok()
        .flatten()
        .unwrap_or_else(|| "en".into())
}

pub fn create<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let lang = read_language(app);
    let l = labels_for(&lang);
    let show = MenuItem::with_id(app, "tray:show", l.show, true, None::<&str>)?;
    let hide = MenuItem::with_id(app, "tray:hide", l.hide, true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "tray:quit", l.quit, true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &hide, &sep, &quit])?;

    let icon = app
        .default_window_icon()
        .ok_or_else(|| tauri::Error::AssetNotFound("default window icon".into()))?
        .clone();

    let _tray = TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .icon_as_template(false)
        .tooltip(TOOLTIP)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "tray:show" => show_main_window(app),
            "tray:hide" => hide_main_window(app),
            "tray:quit" => {
                // app.exit(0) calls std::process::exit which bypasses the
                // window WindowEvent::Destroyed handler — sidecars never
                // get a clean shutdown signal and the bundled Ollama
                // process is left as a zombie until the OS reaps it.
                // Spawn a quick async cleanup, then exit. A 5 s ceiling
                // on the cleanup ensures Quit never hangs the menubar
                // forever if e.g. Ollama is unresponsive — we'd rather
                // leave a zombie than refuse to exit.
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Some(state) = app.try_state::<crate::state::AppState>() {
                        let _ = tokio::time::timeout(
                            std::time::Duration::from_secs(5),
                            state.sidecars.shutdown_all(&state.ollama),
                        )
                        .await;
                    }
                    app.exit(0);
                });
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_main_window(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}

fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn hide_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
}

fn toggle_main_window<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let visible = window.is_visible().unwrap_or(false);
    if visible {
        let _ = window.hide();
    } else {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Rebuild the tray menu using the current `language` preference. Used
/// by `setting_set` whenever the user switches EN↔PL so the tray
/// matches the rest of the UI without a relaunch.
///
/// `set_menu` does NOT re-bind the original `on_menu_event` closure;
/// the closure registered in `create()` continues to fire for the new
/// items because we reuse the same `tray:show` / `tray:hide` /
/// `tray:quit` IDs.
pub fn relabel<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return Ok(());
    };
    let lang = read_language(app);
    let l = labels_for(&lang);
    let show = MenuItem::with_id(app, "tray:show", l.show, true, None::<&str>)?;
    let hide = MenuItem::with_id(app, "tray:hide", l.hide, true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "tray:quit", l.quit, true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &hide, &sep, &quit])?;
    tray.set_menu(Some(menu))?;
    Ok(())
}
