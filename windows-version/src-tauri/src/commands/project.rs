//! Portable project backup/restore.
//!
//! This format deliberately excludes app preferences, provider settings,
//! credentials, model weights, logs, and absolute filesystem paths.

use base64::Engine;
use rusqlite::{
    params_from_iter,
    types::{Value as SqlValue, ValueRef},
    Connection,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use tauri::State;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

const FORMAT_VERSION: u32 = 1;
const MAX_PROJECT_BYTES: u64 = 768 * 1024 * 1024;
const MAX_ASSET_BYTES: u64 = 50 * 1024 * 1024;
const MAX_TOTAL_ASSET_BYTES: u64 = 512 * 1024 * 1024;
const TABLES: &[&str] = &[
    "worlds",
    "world_visual_bibles",
    "fpv_characters",
    "codex_entries",
    "sessions",
    "messages",
    "fpv_story_state",
    "story_pinned_canon",
    "story_memory_records",
];
const ASSET_DIRECTORIES: &[&str] = &["covers", "images/covers", "images/scenes", "portraits"];

#[derive(Debug, Serialize, Deserialize)]
struct ProjectBackup {
    format_version: u32,
    product: String,
    created_at: String,
    tables: BTreeMap<String, Vec<BTreeMap<String, Value>>>,
    assets: Vec<ProjectAsset>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ProjectAsset {
    path: String,
    data_base64: String,
}

#[derive(Debug, Serialize)]
pub struct ProjectTransferSummary {
    pub worlds: usize,
    pub sessions: usize,
    pub assets: usize,
}

fn project_path(path: &str, extension: &str) -> AppResult<PathBuf> {
    let destination = PathBuf::from(path);
    if !destination.is_absolute()
        || destination.extension().and_then(|ext| ext.to_str()) != Some(extension)
    {
        return Err(AppError::Config(format!(
            "project path must be an absolute .{extension} path"
        )));
    }
    let parent = destination
        .parent()
        .filter(|value| !value.as_os_str().is_empty())
        .ok_or_else(|| AppError::Config("project path has no parent directory".into()))?;
    let parent = parent.canonicalize().map_err(|error| {
        AppError::Config(format!("cannot resolve project destination: {error}"))
    })?;
    if destination
        .symlink_metadata()
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(AppError::Config(
            "refusing to write through a symlinked project path".into(),
        ));
    }
    let app_data = crate::storage::app_data_dir()?;
    let app_data = app_data.canonicalize().unwrap_or(app_data);
    if parent.starts_with(&app_data) {
        return Err(AppError::Config(
            "refusing to read or write a project file inside app data".into(),
        ));
    }
    Ok(parent.join(destination.file_name().expect("validated filename")))
}

fn row_value(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(value) => Value::Number(value.into()),
        ValueRef::Real(value) => serde_json::Number::from_f64(value)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        ValueRef::Text(value) => Value::String(String::from_utf8_lossy(value).into_owned()),
        ValueRef::Blob(value) => serde_json::json!({
            "__fpv_blob_base64": base64::engine::general_purpose::STANDARD.encode(value)
        }),
    }
}

fn table_columns(conn: &Connection, table: &str) -> rusqlite::Result<Vec<String>> {
    let mut statement = conn.prepare(&format!("PRAGMA table_info(\"{table}\")"))?;
    let rows = statement.query_map([], |row| row.get(1))?;
    rows.collect()
}

fn export_table(conn: &Connection, table: &str) -> rusqlite::Result<Vec<BTreeMap<String, Value>>> {
    let columns = table_columns(conn, table)?;
    let projection = columns
        .iter()
        .map(|column| format!("\"{column}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let mut statement = conn.prepare(&format!("SELECT {projection} FROM \"{table}\""))?;
    let rows = statement.query_map([], |row| {
        let mut output = BTreeMap::new();
        for (index, column) in columns.iter().enumerate() {
            output.insert(column.clone(), row_value(row.get_ref(index)?));
        }
        Ok(output)
    })?;
    rows.collect()
}

fn collect_files(
    root: &Path,
    current: &Path,
    assets: &mut Vec<ProjectAsset>,
    total: &mut u64,
) -> AppResult<()> {
    if !current.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            collect_files(root, &path, assets, total)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        if path.extension().and_then(|extension| extension.to_str()) == Some("part") {
            continue;
        }
        let size = entry.metadata()?.len();
        if size > MAX_ASSET_BYTES || total.saturating_add(size) > MAX_TOTAL_ASSET_BYTES {
            return Err(AppError::Config(
                "project assets exceed the backup size limit".into(),
            ));
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|_| AppError::Config("project asset is outside app data".into()))?
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");
        let bytes = std::fs::read(&path)?;
        assets.push(ProjectAsset {
            path: relative,
            data_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
        });
        *total += size;
    }
    Ok(())
}

fn build_database_backup(conn: &Connection) -> AppResult<ProjectBackup> {
    let mut tables = BTreeMap::new();
    for table in TABLES {
        let mut rows = export_table(conn, table)?;
        for row in &mut rows {
            for column in ["cover_image_path", "avatar_path"] {
                if let Some(Value::String(path)) = row.get(column).cloned() {
                    row.insert(column.into(), portable_asset_path(&path)?);
                }
            }
        }
        tables.insert((*table).into(), rows);
    }

    Ok(ProjectBackup {
        format_version: FORMAT_VERSION,
        product: "FPV".into(),
        created_at: chrono::Utc::now().to_rfc3339(),
        tables,
        assets: Vec::new(),
    })
}

fn collect_backup_assets(app_data: &Path) -> AppResult<Vec<ProjectAsset>> {
    let mut assets = Vec::new();
    let mut total = 0;
    for directory in ASSET_DIRECTORIES {
        collect_files(app_data, &app_data.join(directory), &mut assets, &mut total)?;
    }
    Ok(assets)
}

fn portable_asset_path(path: &str) -> AppResult<Value> {
    if path.is_empty() {
        return Ok(Value::Null);
    }
    let app_data = crate::storage::app_data_dir()?;
    let path = Path::new(path);
    let relative = if path.is_absolute() {
        path.strip_prefix(&app_data).map_err(|_| {
            AppError::Config("project contains an asset path outside app data".into())
        })?
    } else {
        path
    };
    let relative = relative
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/");
    validate_asset_path(&relative)?;
    Ok(Value::String(relative))
}

fn sql_value(value: &Value) -> AppResult<SqlValue> {
    match value {
        Value::Null => Ok(SqlValue::Null),
        Value::Bool(value) => Ok(SqlValue::Integer(i64::from(*value))),
        Value::Number(value) if value.is_i64() => Ok(SqlValue::Integer(value.as_i64().unwrap())),
        Value::Number(value) if value.is_u64() => Ok(SqlValue::Integer(
            i64::try_from(value.as_u64().unwrap())
                .map_err(|_| AppError::Config("project integer is too large".into()))?,
        )),
        Value::Number(value) => Ok(SqlValue::Real(value.as_f64().unwrap_or_default())),
        Value::String(value) => Ok(SqlValue::Text(value.clone())),
        Value::Object(value) => {
            let encoded = value
                .get("__fpv_blob_base64")
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::Config("invalid project value".into()))?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|_| AppError::Config("invalid project blob".into()))?;
            Ok(SqlValue::Blob(bytes))
        }
        _ => Err(AppError::Config("unsupported project value".into())),
    }
}

fn validate_asset_path(path: &str) -> AppResult<PathBuf> {
    let relative = Path::new(path);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
        || !ASSET_DIRECTORIES
            .iter()
            .any(|prefix| relative.starts_with(prefix))
    {
        return Err(AppError::Config(
            "project contains an unsafe asset path".into(),
        ));
    }
    Ok(relative.to_path_buf())
}

fn restore_table(
    conn: &Connection,
    table: &str,
    rows: &[BTreeMap<String, Value>],
) -> AppResult<()> {
    let columns = table_columns(conn, table)?;
    let allowed = columns.into_iter().collect::<BTreeSet<_>>();
    for row in rows {
        let entries = row
            .iter()
            .filter(|(column, _)| allowed.contains(*column))
            .collect::<Vec<_>>();
        if entries.is_empty() {
            continue;
        }
        let names = entries
            .iter()
            .map(|(column, _)| format!("\"{column}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let placeholders = (1..=entries.len())
            .map(|index| format!("?{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        let values = entries
            .iter()
            .map(|(column, value)| restore_value_path(column, value))
            .map(|value| value.and_then(|value| sql_value(&value)))
            .collect::<AppResult<Vec<_>>>()?;
        conn.execute(
            &format!("INSERT INTO \"{table}\" ({names}) VALUES ({placeholders})"),
            params_from_iter(values),
        )?;
    }
    Ok(())
}

fn restore_value_path(column: &str, value: &Value) -> AppResult<Value> {
    if column != "cover_image_path" && column != "avatar_path" {
        return Ok(value.clone());
    }
    let Some(path) = value.as_str() else {
        return Ok(value.clone());
    };
    let relative = validate_asset_path(path)?;
    let absolute = crate::storage::app_data_dir()?.join(relative);
    Ok(Value::String(absolute.to_string_lossy().into_owned()))
}

fn restore_backup(conn: &Connection, backup: &ProjectBackup) -> AppResult<()> {
    if backup.format_version != FORMAT_VERSION || backup.product != "FPV" {
        return Err(AppError::Config("unsupported FPV project backup".into()));
    }
    if backup
        .tables
        .keys()
        .any(|table| !TABLES.contains(&table.as_str()))
    {
        return Err(AppError::Config(
            "project contains unsupported database tables".into(),
        ));
    }
    if TABLES
        .iter()
        .any(|table| !backup.tables.contains_key(*table))
    {
        return Err(AppError::Config(
            "project backup is missing required database tables".into(),
        ));
    }

    let mut assets = Vec::new();
    let mut total = 0u64;
    for asset in &backup.assets {
        let relative = validate_asset_path(&asset.path)?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&asset.data_base64)
            .map_err(|_| AppError::Config("project contains invalid asset data".into()))?;
        if bytes.len() as u64 > MAX_ASSET_BYTES
            || total + bytes.len() as u64 > MAX_TOTAL_ASSET_BYTES
        {
            return Err(AppError::Config(
                "project assets exceed the restore limit".into(),
            ));
        }
        total += bytes.len() as u64;
        assets.push((relative, bytes));
    }

    let app_data = crate::storage::app_data_dir()?;
    let stage_root = app_data.join(format!(".fpv-project-restore-{}", Uuid::new_v4()));
    let stage_result = (|| -> AppResult<()> {
        let staged_assets_root = stage_root.join("new");
        for directory in ASSET_DIRECTORIES {
            std::fs::create_dir_all(staged_assets_root.join(directory))?;
        }
        for (relative, bytes) in assets {
            let staged = staged_assets_root.join(&relative);
            if let Some(parent) = staged.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&staged, bytes)?;
        }
        Ok(())
    })();
    if let Err(error) = stage_result {
        let _ = std::fs::remove_dir_all(&stage_root);
        return Err(error);
    }

    if let Err(error) = conn.execute_batch("BEGIN IMMEDIATE") {
        let _ = std::fs::remove_dir_all(&stage_root);
        return Err(error.into());
    }
    let staged_assets_root = stage_root.join("new");
    let previous_assets_root = stage_root.join("old");
    let mut moved_old = Vec::new();
    let mut installed_new = Vec::new();
    let install_result = (|| -> AppResult<()> {
        for directory in ASSET_DIRECTORIES {
            let target = app_data.join(directory);
            let previous = previous_assets_root.join(directory);
            if target.exists() {
                if let Some(parent) = previous.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::rename(&target, &previous)?;
                moved_old.push((previous, target.clone()));
            }
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::rename(staged_assets_root.join(directory), &target)?;
            installed_new.push(target);
        }
        Ok(())
    })();
    if let Err(error) = install_result {
        let _ = conn.execute_batch("ROLLBACK");
        for target in installed_new.iter().rev() {
            let _ = std::fs::remove_dir_all(target);
        }
        for (previous, target) in moved_old.iter().rev() {
            let _ = std::fs::rename(previous, target);
        }
        let _ = std::fs::remove_dir_all(&stage_root);
        return Err(error);
    }
    let result = (|| -> AppResult<()> {
        for table in [
            "story_memory_records",
            "story_pinned_canon",
            "fpv_story_state",
            "messages",
            "codex_entries",
            "sessions",
            "fpv_characters",
            "world_visual_bibles",
            "worlds",
        ] {
            conn.execute(&format!("DELETE FROM \"{table}\""), [])?;
        }
        for table in TABLES {
            restore_table(conn, table, &backup.tables[*table])?;
        }
        Ok(())
    })();
    match result {
        Ok(()) => {
            if let Err(error) = conn.execute_batch("COMMIT") {
                let _ = conn.execute_batch("ROLLBACK");
                for target in installed_new.iter().rev() {
                    let _ = std::fs::remove_dir_all(target);
                }
                for (previous, target) in moved_old.iter().rev() {
                    let _ = std::fs::rename(previous, target);
                }
                let _ = std::fs::remove_dir_all(&stage_root);
                return Err(error.into());
            }
            let _ = std::fs::remove_dir_all(&stage_root);
            Ok(())
        }
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            for target in installed_new.iter().rev() {
                let _ = std::fs::remove_dir_all(target);
            }
            for (previous, target) in moved_old.iter().rev() {
                let _ = std::fs::rename(previous, target);
            }
            let _ = std::fs::remove_dir_all(&stage_root);
            Err(error)
        }
    }
}

#[tauri::command(rename_all = "snake_case")]
pub async fn project_export(
    state: State<'_, AppState>,
    dest_path: String,
) -> AppResult<ProjectTransferSummary> {
    let destination = project_path(&dest_path, "fpv-project")?;
    let app_data = crate::storage::app_data_dir()?;
    let backup = {
        let conn = state.db.lock().await;
        conn.execute_batch("BEGIN")?;
        let result = (|| -> AppResult<ProjectBackup> {
            let mut backup = build_database_backup(&conn)?;
            backup.assets = collect_backup_assets(&app_data)?;
            Ok(backup)
        })();
        match result {
            Ok(backup) => {
                conn.execute_batch("COMMIT")?;
                backup
            }
            Err(error) => {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(error);
            }
        }
    };
    let summary = ProjectTransferSummary {
        worlds: backup.tables.get("worlds").map_or(0, Vec::len),
        sessions: backup.tables.get("sessions").map_or(0, Vec::len),
        assets: backup.assets.len(),
    };
    let json = tokio::task::spawn_blocking(move || serde_json::to_vec_pretty(&backup))
        .await
        .map_err(|error| AppError::Other(format!("project serialization join error: {error}")))??;
    if json.len() as u64 > MAX_PROJECT_BYTES {
        return Err(AppError::Config(
            "project backup exceeds the size limit".into(),
        ));
    }
    tokio::task::spawn_blocking(move || {
        let temp = destination.with_file_name(format!(".{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| {
            std::fs::write(&temp, &json)?;
            std::fs::rename(&temp, &destination)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        result
    })
    .await
    .map_err(|error| AppError::Other(format!("project export join error: {error}")))??;
    Ok(summary)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn project_import(
    state: State<'_, AppState>,
    source_path: String,
) -> AppResult<ProjectTransferSummary> {
    let source = project_path(&source_path, "fpv-project")?;
    let backup: ProjectBackup = tokio::task::spawn_blocking(move || {
        let metadata = std::fs::metadata(&source)?;
        if metadata.len() > MAX_PROJECT_BYTES {
            return Err(AppError::Config(
                "project backup exceeds the size limit".into(),
            ));
        }
        serde_json::from_slice(&std::fs::read(source)?)
            .map_err(|error| AppError::Config(format!("invalid FPV project backup: {error}")))
    })
    .await
    .map_err(|error| AppError::Other(format!("project import join error: {error}")))??;
    let summary = ProjectTransferSummary {
        worlds: backup.tables.get("worlds").map_or(0, Vec::len),
        sessions: backup.tables.get("sessions").map_or(0, Vec::len),
        assets: backup.assets.len(),
    };
    let conn = state.db.lock().await;
    restore_backup(&conn, &backup)?;
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_absolute_and_parent_asset_paths() {
        assert!(validate_asset_path("/tmp/story.png").is_err());
        assert!(validate_asset_path("covers/../secret.txt").is_err());
        assert!(validate_asset_path("models/secret.bin").is_err());
        assert!(validate_asset_path("images/scenes/session/message.png").is_ok());
    }

    #[test]
    fn accepts_cover_assets_under_images() {
        assert!(validate_asset_path("images/covers/world.webp").is_ok());
    }

    #[test]
    fn rejects_backup_missing_required_tables_before_deleting_data() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE worlds (id TEXT)", []).unwrap();
        conn.execute("INSERT INTO worlds (id) VALUES ('existing')", [])
            .unwrap();
        let backup = ProjectBackup {
            format_version: FORMAT_VERSION,
            product: "FPV".into(),
            created_at: String::new(),
            tables: BTreeMap::new(),
            assets: Vec::new(),
        };

        assert!(restore_backup(&conn, &backup).is_err());
        assert_eq!(
            conn.query_row("SELECT id FROM worlds", [], |row| row.get::<_, String>(0))
                .unwrap(),
            "existing"
        );
    }
}
