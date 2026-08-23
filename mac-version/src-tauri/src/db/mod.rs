use std::sync::Arc;

use rusqlite::{params, Connection};
use tokio::sync::Mutex;

use crate::error::AppResult;
use crate::storage;

pub mod characters;
pub mod cloud_spend;
pub mod migrations;
pub mod schema;

pub type DbHandle = Arc<Mutex<Connection>>;

pub fn open() -> AppResult<DbHandle> {
    let conn = open_connection()?;
    Ok(Arc::new(Mutex::new(conn)))
}

/// Open a single configured connection without wrapping in Mutex.
pub fn open_connection() -> AppResult<Connection> {
    let path = storage::db_path()?;
    let conn = Connection::open(&path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "synchronous", "FULL")?;
    let _ = conn.pragma_update(None, "mmap_size", 256i64 * 1024 * 1024);
    let _ = conn.pragma_update(None, "cache_size", -16384i64);
    let _ = conn.pragma_update(None, "temp_store", "MEMORY");
    migrations::run(&conn)?;
    // Housekeeping: keep the cloud-spend ledger bounded (30-day retention
    // of settled rows). Non-fatal — a failed prune must not block boot.
    if let Err(err) = crate::db::cloud_spend::prune_settled(&conn, crate::metrics::now()) {
        tracing::warn!(?err, "cloud spend ledger prune failed (non-fatal)");
    }
    let _ = conn.execute_batch("PRAGMA optimize;");
    Ok(conn)
}

pub fn meta_get(conn: &Connection, key: &str) -> AppResult<Option<String>> {
    // Only "no such row" means unset; a real DB failure (locked/corrupt)
    // must surface instead of masquerading as a missing preference.
    use rusqlite::OptionalExtension;
    let result = conn
        .query_row(
            "SELECT value FROM app_meta WHERE key = ?1",
            params![key],
            |r| r.get::<_, String>(0),
        )
        .optional()?;
    Ok(result)
}

pub fn meta_set(conn: &Connection, key: &str, value: &str) -> AppResult<()> {
    conn.execute(
        "INSERT INTO app_meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}
