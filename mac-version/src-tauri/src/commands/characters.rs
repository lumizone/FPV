use tauri::State;

use crate::db::characters as db;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

pub use db::Character;

fn validate_text(field: &str, value: &str, max: usize, required: bool) -> AppResult<String> {
    let value = value.trim();
    if required && value.is_empty() {
        return Err(AppError::Config(format!(
            "character {field} cannot be empty"
        )));
    }
    if value.len() > max {
        return Err(AppError::Config(format!("character {field} is too long")));
    }
    Ok(value.to_owned())
}

fn validate_id(id: &str) -> AppResult<()> {
    crate::storage::fs_safe_id(id).map(|_| ())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn character_create(
    state: State<'_, AppState>,
    world_id: String,
    name: String,
    role: Option<String>,
    backstory: Option<String>,
    traits: Option<Vec<String>>,
    avatar_path: Option<String>,
) -> AppResult<String> {
    validate_id(&world_id)?;
    validate_text("name", &name, 240, true)?;
    if let Some(role) = &role {
        validate_text("role", role, 120, false)?;
    }
    if let Some(backstory) = &backstory {
        validate_text("backstory", backstory, 8_000, false)?;
    }
    if let Some(values) = &traits {
        if values.len() > 32 {
            return Err(AppError::Config(
                "character traits are limited to 32 entries".into(),
            ));
        }
        for value in values {
            validate_text("trait", value, 160, true)?;
        }
    }
    if let Some(path) = &avatar_path {
        validate_text("avatar path", path, 1_024, false)?;
    }
    let new_char = db::NewCharacter {
        world_id,
        name,
        role,
        backstory,
        traits,
        avatar_path,
    };
    let conn = state.db.lock().await;
    db::create(&conn, &new_char).map_err(Into::into)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn character_list(
    state: State<'_, AppState>,
    world_id: String,
) -> AppResult<Vec<Character>> {
    let conn = state.db.lock().await;
    db::list_for_world(&conn, &world_id).map_err(Into::into)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn character_get(state: State<'_, AppState>, id: String) -> AppResult<Character> {
    let conn = state.db.lock().await;
    db::get(&conn, &id).map_err(Into::into)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn character_update(
    state: State<'_, AppState>,
    id: String,
    name: Option<String>,
    role: Option<String>,
    backstory: Option<String>,
    traits: Option<Vec<String>>,
    avatar_path: Option<String>,
) -> AppResult<()> {
    validate_id(&id)?;
    let conn = state.db.lock().await;
    db::get(&conn, &id).map_err(|_| AppError::Config("character not found".into()))?;
    if let Some(value) = &name {
        validate_text("name", value, 240, true)?;
    }
    if let Some(value) = &role {
        validate_text("role", value, 120, false)?;
    }
    if let Some(value) = &backstory {
        validate_text("backstory", value, 8_000, false)?;
    }
    if let Some(values) = &traits {
        if values.len() > 32 {
            return Err(AppError::Config(
                "character traits are limited to 32 entries".into(),
            ));
        }
        for value in values {
            validate_text("trait", value, 160, true)?;
        }
    }
    if let Some(value) = &avatar_path {
        validate_text("avatar path", value, 1_024, false)?;
    }
    db::update(
        &conn,
        &id,
        name.as_deref(),
        role.as_deref(),
        backstory.as_deref(),
        traits.as_deref(),
        avatar_path.as_deref(),
    )
    .map_err(Into::into)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn character_delete(state: State<'_, AppState>, id: String) -> AppResult<()> {
    validate_id(&id)?;
    let conn = state.db.lock().await;
    db::get(&conn, &id).map_err(|_| AppError::Config("character not found".into()))?;
    db::delete(&conn, &id).map_err(Into::into)
}
