//! Native macOS application menu.
//!
//! Tauri 2 replaces the OS default menu the moment you call
//! `Builder::menu(...)`. So if we want a "Check for Updates…" item in
//! the app menu, we own the entire menu bar from here.
//!
//! Layout (App / Edit / View / Window / Help) mirrors the macOS HIG.
//! The menu intentionally contains only native actions until a signed updater
//! endpoint is configured for production.

use tauri::{
    menu::{AboutMetadataBuilder, Menu, SubmenuBuilder},
    AppHandle, Manager, Runtime,
};

const APP_NAME: &str = "FPV";
pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let lang = read_language(app);
    let l = labels_for(&lang);

    let about_metadata = AboutMetadataBuilder::new().name(Some(APP_NAME)).build();

    let app_submenu = SubmenuBuilder::new(app, APP_NAME)
        .about(Some(about_metadata))
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .quit()
        .build()?;

    let edit_submenu = SubmenuBuilder::new(app, l.edit)
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;

    let view_submenu = SubmenuBuilder::new(app, l.view).fullscreen().build()?;

    let window_submenu = SubmenuBuilder::new(app, l.window)
        .minimize()
        .maximize()
        .separator()
        .close_window()
        .build()?;

    let help_submenu = SubmenuBuilder::new(app, l.help).build()?;

    Menu::with_items(
        app,
        &[
            &app_submenu,
            &edit_submenu,
            &view_submenu,
            &window_submenu,
            &help_submenu,
        ],
    )
}

pub fn on_menu_event<R: Runtime>(_app: &AppHandle<R>, _id: &str) {}

struct Labels {
    edit: &'static str,
    view: &'static str,
    window: &'static str,
    help: &'static str,
}

fn labels_for(lang: &str) -> Labels {
    if lang == "pl" {
        Labels {
            edit: "Edycja",
            view: "Widok",
            window: "Okno",
            help: "Pomoc",
        }
    } else {
        Labels {
            edit: "Edit",
            view: "View",
            window: "Window",
            help: "Help",
        }
    }
}

fn read_language<R: Runtime>(app: &AppHandle<R>) -> String {
    let Some(state) = app.try_state::<crate::state::AppState>() else {
        return "en".into();
    };
    let conn = state.db.blocking_lock();
    crate::db::meta_get(&conn, "language")
        .ok()
        .flatten()
        .unwrap_or_else(|| "en".into())
}
