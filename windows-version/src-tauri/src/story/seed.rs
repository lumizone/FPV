use std::path::Path;

use rusqlite::Connection;
use serde::Deserialize;

use crate::error::AppResult;
use crate::storage;
use crate::story::db::{self, NewWorld};

/// A single preset world as stored in `seed-data/*.json`.
#[derive(Debug, Deserialize)]
struct SeedWorld {
    name: String,
    genre: String,
    description: Option<String>,
    system_prompt: String,
    accent_color: Option<String>,
    is_nsfw: bool,
    cover_image_path: Option<String>,
    source: String,
}

/// Copy a bundled seed image (relative to `resource_dir/resources/seed-images/<sub>`)
/// into the app-data images directory so it's reachable through the asset protocol
/// (`$APPDATA/images/*` is in the CSP scope). Returns the absolute path to the
/// copied file.
fn copy_seed_image(
    resource_dir: &Path,
    images_dir: &Path,
    relative: &str,
) -> std::io::Result<std::path::PathBuf> {
    // Guard against path traversal in seed data — covers are `covers/<name>.webp`.
    let rel = Path::new(relative);
    if rel.components().count() != 2 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("unsafe seed image path: {relative}"),
        ));
    }
    // Tauri nests bundled resources under `Contents/Resources/resources/`
    // (the `resources/` prefix in tauri.conf.json is preserved on disk).
    let src = resource_dir.join("resources").join("seed-images").join(rel);
    let dest = images_dir.join(rel);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if dest.exists() {
        return Ok(dest);
    }
    std::fs::copy(&src, &dest)?;
    Ok(dest)
}

fn load_seed_worlds(resource_dir: &Path, filename: &str) -> AppResult<Vec<SeedWorld>> {
    let json_path = resource_dir
        .join("resources")
        .join("seed-data")
        .join(filename);
    if !json_path.exists() {
        return Ok(Vec::new());
    }
    let json_bytes = std::fs::read(&json_path).map_err(|e| {
        crate::error::AppError::Other(format!(
            "failed to read seed data at {}: {e}",
            json_path.display()
        ))
    })?;
    serde_json::from_slice(&json_bytes).map_err(|e| {
        crate::error::AppError::Other(format!(
            "failed to parse seed data {}: {e}",
            json_path.display()
        ))
    })
}

/// If the `worlds` table is empty, populate it from the bundled preset JSON
/// files (`worlds-presets.json` + `worlds-community.json`). Called once at app
/// startup, after the DB migration has run and before the window is shown.
/// Also copies the bundled cover images into app-data so the world grid can
/// render real covers.
pub fn seed_if_empty(conn: &Connection, resource_dir: &Path) -> AppResult<()> {
    if !db::list_worlds(conn)?.is_empty() {
        // Already seeded — no-op.
        return Ok(());
    }

    let mut worlds = load_seed_worlds(resource_dir, "worlds-presets.json")?;
    let community = load_seed_worlds(resource_dir, "worlds-community.json")?;
    worlds.extend(community);

    let images_dir = storage::images_dir()?;
    let mut seeded = 0usize;
    for w in &worlds {
        // Copy the cover into app-data if the seed data references one.
        let cover_path = if let Some(ref rel) = w.cover_image_path {
            match copy_seed_image(resource_dir, &images_dir, rel) {
                Ok(abs) => Some(abs.to_string_lossy().to_string()),
                Err(e) => {
                    tracing::warn!(
                        cover = %rel,
                        ?e,
                        "seed cover copy failed — falling back to no cover"
                    );
                    None
                }
            }
        } else {
            None
        };

        let new_world = NewWorld {
            name: w.name.clone(),
            genre: w.genre.clone(),
            description: w.description.clone(),
            system_prompt: w.system_prompt.clone(),
            accent_color: w.accent_color.clone(),
            is_nsfw: w.is_nsfw,
            cover_image_path: cover_path,
            source: w.source.clone(),
        };
        db::create_world(conn, &new_world)?;
        seeded += 1;
    }

    tracing::info!(count = seeded, "seeded preset worlds from bundled JSON");
    Ok(())
}

/// Backfill cover images for already-seeded worlds whose `cover_image_path`
/// is still NULL (installs that predate the bundled-cover work). Called at
/// every startup alongside `seed_if_empty`; cheap (only touches worlds that
/// actually lack a cover) and idempotent.
pub fn backfill_seed_covers(conn: &Connection, resource_dir: &Path) -> AppResult<()> {
    let mut worlds = load_seed_worlds(resource_dir, "worlds-presets.json")?;
    worlds.extend(load_seed_worlds(resource_dir, "worlds-community.json")?);

    let images_dir = storage::images_dir()?;
    let mut fixed = 0usize;
    for w in &worlds {
        let Some(ref rel) = w.cover_image_path else {
            continue;
        };
        // Only backfill seed-source worlds that are still missing a cover.
        let Some(db_world) = db::list_worlds(conn)?
            .into_iter()
            .find(|dbw| dbw.name == w.name && dbw.source == "seed")
        else {
            continue;
        };
        if db_world.cover_image_path.is_some() {
            continue;
        }
        let Ok(abs) = copy_seed_image(resource_dir, &images_dir, rel) else {
            continue;
        };
        let path_str = abs.to_string_lossy().to_string();
        db::update_cover_image_path(conn, &db_world.id, &path_str)?;
        fixed += 1;
    }
    if fixed > 0 {
        tracing::info!(fixed, "backfilled seed covers");
    }
    Ok(())
}
