use rusqlite::{params, types::Type, Connection, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewWorld {
    pub name: String,
    pub genre: String,
    pub description: Option<String>,
    pub system_prompt: String,
    pub accent_color: Option<String>,
    pub is_nsfw: bool,
    pub cover_image_path: Option<String>,
    pub source: String, // "seed" | "user"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct World {
    pub id: String,
    pub name: String,
    pub genre: String,
    pub description: Option<String>,
    pub system_prompt: String,
    pub accent_color: Option<String>,
    pub is_nsfw: bool,
    pub cover_image_path: Option<String>,
    pub source: String,
    pub privacy_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StoryState {
    pub turn: i64,
    pub location: String,
    pub active_goals: Vec<String>,
    pub open_threads: Vec<String>,
    pub inventory: Vec<String>,
    pub relationships: Vec<String>,
    pub facts: Vec<String>,
    pub conflicts: Vec<String>,
    pub last_scene: String,
    pub source: String,
    pub quests: Vec<Quest>,
    pub inventory_items: Vec<InventoryItem>,
    pub relationship_records: Vec<RelationshipRecord>,
    pub pending_changes: Vec<PendingChange>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PendingChange {
    pub id: String,
    pub kind: String,
    pub summary: String,
    pub target: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Quest {
    pub id: String,
    pub title: String,
    pub status: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct InventoryItem {
    pub name: String,
    pub quantity: i64,
    pub condition: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct RelationshipRecord {
    pub character: String,
    pub status: String,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct VisualBible {
    pub style: String,
    pub palette: String,
    pub character_anchors: String,
    pub location_anchors: String,
    pub negative_prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexSaveEntry {
    pub id: Option<String>,
    pub title: String,
    pub content: String,
    pub triggers: String,
}

impl Default for StoryState {
    fn default() -> Self {
        Self {
            turn: 0,
            location: String::new(),
            active_goals: Vec::new(),
            open_threads: Vec::new(),
            inventory: Vec::new(),
            relationships: Vec::new(),
            facts: Vec::new(),
            conflicts: Vec::new(),
            last_scene: String::new(),
            source: "unknown".into(),
            quests: Vec::new(),
            inventory_items: Vec::new(),
            relationship_records: Vec::new(),
            pending_changes: Vec::new(),
        }
    }
}

pub fn create_world(conn: &Connection, w: &NewWorld) -> Result<String> {
    let id = Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO worlds (id, name, genre, description, system_prompt, accent_color, is_nsfw, cover_image_path, source)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            id,
            w.name,
            w.genre,
            w.description,
            w.system_prompt,
            w.accent_color,
            w.is_nsfw as i32,
            w.cover_image_path,
            w.source,
        ],
    )?;
    Ok(id)
}

pub fn list_worlds(conn: &Connection) -> Result<Vec<World>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, genre, description, system_prompt, accent_color, is_nsfw, cover_image_path, source, privacy_mode \
         FROM worlds ORDER BY created_at DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(World {
            id: row.get(0)?,
            name: row.get(1)?,
            genre: row.get(2)?,
            description: row.get(3)?,
            system_prompt: row.get(4)?,
            accent_color: row.get(5)?,
            is_nsfw: row.get::<_, i32>(6)? != 0,
            cover_image_path: row.get(7)?,
            source: row.get(8)?,
            privacy_mode: row.get(9)?,
        })
    })?;
    rows.collect()
}

pub fn get_world(conn: &Connection, id: &str) -> Result<World> {
    conn.query_row(
        "SELECT id, name, genre, description, system_prompt, accent_color, is_nsfw, cover_image_path, source, privacy_mode \
         FROM worlds WHERE id = ?1",
        params![id],
        |row| {
            Ok(World {
                id: row.get(0)?,
                name: row.get(1)?,
                genre: row.get(2)?,
                description: row.get(3)?,
                system_prompt: row.get(4)?,
                accent_color: row.get(5)?,
                is_nsfw: row.get::<_, i32>(6)? != 0,
                cover_image_path: row.get(7)?,
                source: row.get(8)?,
                privacy_mode: row.get(9)?,
            })
        },
    )
}

pub fn get_visual_bible(conn: &Connection, world_id: &str) -> Result<VisualBible> {
    conn.query_row(
        "SELECT style, palette, character_anchors, location_anchors, negative_prompt
         FROM world_visual_bibles WHERE world_id = ?1",
        params![world_id],
        |row| {
            Ok(VisualBible {
                style: row.get(0)?,
                palette: row.get(1)?,
                character_anchors: row.get(2)?,
                location_anchors: row.get(3)?,
                negative_prompt: row.get(4)?,
            })
        },
    )
    .or_else(|error| match error {
        rusqlite::Error::QueryReturnedNoRows => Ok(VisualBible::default()),
        other => Err(other),
    })
}

pub fn set_visual_bible(conn: &Connection, world_id: &str, bible: &VisualBible) -> Result<()> {
    conn.execute(
        "INSERT INTO world_visual_bibles
         (world_id, style, palette, character_anchors, location_anchors, negative_prompt, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
         ON CONFLICT(world_id) DO UPDATE SET style = excluded.style,
         palette = excluded.palette, character_anchors = excluded.character_anchors,
         location_anchors = excluded.location_anchors, negative_prompt = excluded.negative_prompt,
         updated_at = excluded.updated_at",
        params![world_id, bible.style, bible.palette, bible.character_anchors, bible.location_anchors, bible.negative_prompt],
    )?;
    Ok(())
}

pub fn delete_world(conn: &Connection, id: &str) -> Result<()> {
    let changed = conn.execute("DELETE FROM worlds WHERE id = ?1", params![id])?;
    if changed == 0 {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    Ok(())
}

pub fn delete_session(conn: &Connection, id: &str) -> Result<()> {
    let changed = conn.execute("DELETE FROM sessions WHERE id = ?1", params![id])?;
    if changed == 0 {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    Ok(())
}

pub fn update_world(conn: &Connection, id: &str, w: &NewWorld) -> Result<()> {
    let changed = conn.execute(
        "UPDATE worlds SET name = ?2, genre = ?3, description = ?4, system_prompt = ?5, \
         accent_color = ?6, is_nsfw = ?7, source = ?8 WHERE id = ?1",
        params![
            id,
            w.name,
            w.genre,
            w.description,
            w.system_prompt,
            w.accent_color,
            w.is_nsfw as i32,
            w.source,
        ],
    )?;
    if changed == 0 {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    Ok(())
}

/// Atomically save a world and all of its editor-owned data.
///
/// The editor can fail after the world row has been written, so this keeps
/// the world, Visual Bible, Codex deletions, and Codex upserts in one commit.
pub fn save_world(
    conn: &Connection,
    id: Option<&str>,
    world: &NewWorld,
    visual_bible: &VisualBible,
    codex_entries: &[CodexSaveEntry],
    removed_codex_ids: &[String],
    privacy_mode: &str,
) -> Result<String> {
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let result = (|| -> Result<String> {
        let world_id = match id {
            Some(existing_id) => {
                update_world(conn, existing_id, world)?;
                existing_id.to_string()
            }
            None => create_world(conn, world)?,
        };
        conn.execute(
            "UPDATE worlds SET privacy_mode = ?2 WHERE id = ?1",
            params![world_id, privacy_mode],
        )?;

        set_visual_bible(conn, &world_id, visual_bible)?;

        for codex_id in removed_codex_ids {
            // A stale deletion list (entry already gone) is a no-op, not a
            // reason to roll back the whole world save.
            match delete_codex_entry_for_world(conn, codex_id, &world_id) {
                Ok(()) => {}
                Err(rusqlite::Error::QueryReturnedNoRows) => {}
                Err(error) => return Err(error),
            }
        }

        for entry in codex_entries {
            if let Some(entry_id) = entry.id.as_deref() {
                if !codex_belongs_to_world(conn, entry_id, &world_id)? {
                    return Err(rusqlite::Error::QueryReturnedNoRows);
                }
            }
            upsert_codex_entry(
                conn,
                entry.id.as_deref(),
                &world_id,
                &entry.title,
                &entry.content,
                &entry.triggers,
            )?;
        }

        Ok(world_id)
    })();

    match result {
        Ok(world_id) => {
            conn.execute_batch("COMMIT")?;
            Ok(world_id)
        }
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

pub fn create_session(conn: &Connection, world_id: &str) -> Result<String> {
    let id = Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO sessions (id, world_id) VALUES (?1, ?2)",
        params![id, world_id],
    )?;
    Ok(id)
}

pub fn branch_session(conn: &Connection, session_id: &str, label: &str) -> Result<String> {
    copy_session(conn, session_id, label, false)
}

pub fn checkpoint_session(conn: &Connection, session_id: &str, label: &str) -> Result<String> {
    copy_session(conn, session_id, label, true)
}

/// Create a clean retcon branch containing history only through `message_id`.
/// Derived state is intentionally reset: it may contradict the edited past.
pub fn fork_session_at_message(
    conn: &Connection,
    session_id: &str,
    message_id: &str,
    label: &str,
) -> Result<String> {
    let (world_id, sequence): (String, i64) = conn.query_row(
        "SELECT s.world_id, m.sequence FROM messages m JOIN sessions s ON s.id = m.session_id WHERE m.id = ?1 AND m.session_id = ?2",
        params![message_id, session_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let result = (|| -> Result<String> {
        let new_id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO sessions (id, world_id, parent_session_id, branch_label) VALUES (?1, ?2, ?3, ?4)",
            params![new_id, world_id, session_id, label],
        )?;
        let mut statement = conn.prepare(
            "SELECT role, content FROM messages WHERE session_id = ?1 AND sequence <= ?2 ORDER BY sequence ASC",
        )?;
        let rows = statement.query_map(params![session_id, sequence], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (role, content) = row?;
            append_message(conn, &new_id, &role, &content)?;
        }
        for canon in list_pinned_canon(conn, session_id)? {
            add_pinned_canon(conn, &new_id, &canon.content)?;
        }
        Ok(new_id)
    })();
    match result {
        Ok(id) => {
            conn.execute_batch("COMMIT")?;
            Ok(id)
        }
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

fn copy_session(
    conn: &Connection,
    session_id: &str,
    label: &str,
    is_checkpoint: bool,
) -> Result<String> {
    let (world_id, summary): (String, String) = conn.query_row(
        "SELECT world_id, summary FROM sessions WHERE id = ?1",
        params![session_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let result = (|| -> Result<String> {
        let new_id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO sessions (id, world_id, summary, parent_session_id, branch_label, is_checkpoint)
              VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![new_id, world_id, summary, session_id, label, is_checkpoint as i32],
        )?;

        for message in list_messages(conn, session_id)? {
            append_message(conn, &new_id, &message.role, &message.content)?;
        }
        let state = get_story_state(conn, session_id)?;
        set_story_state_in_txn(conn, &new_id, &state)?;
        for canon in list_pinned_canon(conn, session_id)? {
            add_pinned_canon(conn, &new_id, &canon.content)?;
        }
        Ok(new_id)
    })();
    match result {
        Ok(id) => {
            conn.execute_batch("COMMIT")?;
            Ok(id)
        }
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub world_id: String,
    pub summary: String,
    pub created_at: String,
    pub updated_at: String,
    pub message_count: i64,
    pub parent_session_id: Option<String>,
    pub branch_label: String,
    pub is_checkpoint: bool,
}

pub fn list_sessions_for_world(conn: &Connection, world_id: &str) -> Result<Vec<SessionSummary>> {
    let mut stmt = conn.prepare(
        "SELECT s.id, s.world_id, s.summary, s.created_at, s.updated_at, \
          COALESCE((SELECT COUNT(*) FROM messages WHERE session_id = s.id), 0) AS message_count \
          , s.parent_session_id, s.branch_label, s.is_checkpoint \
          FROM sessions s WHERE s.world_id = ?1 ORDER BY s.updated_at DESC",
    )?;
    let rows = stmt.query_map(params![world_id], |row| {
        Ok(SessionSummary {
            id: row.get(0)?,
            world_id: row.get(1)?,
            summary: row.get(2)?,
            created_at: row.get(3)?,
            updated_at: row.get(4)?,
            message_count: row.get(5)?,
            parent_session_id: row.get(6)?,
            branch_label: row.get(7)?,
            is_checkpoint: row.get::<_, i32>(8)? != 0,
        })
    })?;
    rows.collect()
}

pub fn get_session_summary(conn: &Connection, session_id: &str) -> Result<String> {
    conn.query_row(
        "SELECT summary FROM sessions WHERE id = ?1",
        params![session_id],
        |row| row.get(0),
    )
}

pub fn append_message(
    conn: &Connection,
    session_id: &str,
    role: &str,
    content: &str,
) -> Result<String> {
    let id = Uuid::new_v4().to_string();
    let sequence: i64 = conn.query_row(
        "SELECT COALESCE(MAX(sequence), -1) + 1 FROM messages WHERE session_id = ?1",
        params![session_id],
        |row| row.get(0),
    )?;
    conn.execute(
        "INSERT INTO messages (id, session_id, role, content, sequence) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, session_id, role, content, sequence],
    )?;
    conn.execute(
        "UPDATE sessions SET updated_at = datetime('now') WHERE id = ?1",
        params![session_id],
    )?;
    Ok(id)
}

pub fn update_message(conn: &Connection, id: &str, content: &str) -> Result<()> {
    let changed = conn.execute(
        "UPDATE messages SET content = ?1 WHERE id = ?2",
        params![content, id],
    )?;
    if changed == 0 {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    conn.execute(
        "UPDATE sessions SET updated_at = datetime('now') WHERE id = (SELECT session_id FROM messages WHERE id = ?1)",
        params![id],
    )?;
    Ok(())
}

/// Replace a newly generated narrator row with an existing narrator row.
/// Both changes are committed together so regeneration cannot leave duplicate
/// responses when the second write fails.
pub fn replace_message(
    conn: &Connection,
    old_message_id: &str,
    new_message_id: &str,
    content: &str,
) -> Result<()> {
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let result = (|| {
        let old_role: String = conn.query_row(
            "SELECT role FROM messages WHERE id = ?1",
            params![old_message_id],
            |row| row.get(0),
        )?;
        let new_role: String = conn.query_row(
            "SELECT role FROM messages WHERE id = ?1",
            params![new_message_id],
            |row| row.get(0),
        )?;
        let old_session: String = conn.query_row(
            "SELECT session_id FROM messages WHERE id = ?1",
            params![old_message_id],
            |row| row.get(0),
        )?;
        let new_session: String = conn.query_row(
            "SELECT session_id FROM messages WHERE id = ?1",
            params![new_message_id],
            |row| row.get(0),
        )?;
        if old_role != "narrator" || new_role != "narrator" {
            return Err(rusqlite::Error::InvalidParameterName(
                "replace requires narrator messages".into(),
            ));
        }
        if old_message_id == new_message_id || old_session != new_session {
            return Err(rusqlite::Error::InvalidParameterName(
                "replace requires messages from the same session".into(),
            ));
        }
        let changed = conn.execute(
            "UPDATE messages SET content = ?1 WHERE id = ?2",
            params![content, old_message_id],
        )?;
        if changed != 1 {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }
        let deleted = conn.execute(
            "DELETE FROM messages WHERE id = ?1",
            params![new_message_id],
        )?;
        if deleted != 1 {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }
        conn.execute(
            "UPDATE sessions SET updated_at = datetime('now') WHERE id = (SELECT session_id FROM messages WHERE id = ?1)",
            params![old_message_id],
        )?;
        Ok(())
    })();
    match result {
        Ok(()) => conn.execute_batch("COMMIT"),
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

/// Replace an existing narrator response in one database write.
pub fn replace_message_content(
    conn: &Connection,
    session_id: &str,
    message_id: &str,
    content: &str,
) -> Result<()> {
    let changed = conn.execute(
        "UPDATE messages SET content = ?1 WHERE id = ?2 AND session_id = ?3 AND role = 'narrator'",
        params![content, message_id, session_id],
    )?;
    if changed != 1 {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    conn.execute(
        "UPDATE sessions SET updated_at = datetime('now') WHERE id = ?1",
        params![session_id],
    )?;
    Ok(())
}

/// Remove derived data that was computed from a narrator scene before that
/// scene is regenerated. Message history remains the source of truth.
pub fn clear_regeneration_derived_data(
    conn: &Connection,
    session_id: &str,
    message_id: &str,
) -> Result<()> {
    let narrator_exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM messages WHERE id = ?1 AND session_id = ?2 AND role = 'narrator')",
        params![message_id, session_id],
        |row| row.get(0),
    )?;
    if !narrator_exists {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    conn.execute_batch("BEGIN")?;
    let result = (|| {
        conn.execute(
            "DELETE FROM story_memory_records WHERE session_id = ?1 AND message_id = ?2",
            params![session_id, message_id],
        )?;
        conn.execute(
            "DELETE FROM fpv_story_state WHERE session_id = ?1",
            params![session_id],
        )?;
        conn.execute(
            "DELETE FROM fpv_story_state_history WHERE session_id = ?1",
            params![session_id],
        )?;
        let changed = conn.execute(
            "UPDATE sessions SET summary = '', updated_at = datetime('now') WHERE id = ?1",
            params![session_id],
        )?;
        if changed != 1 {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }
        Ok(())
    })();
    match result {
        Ok(()) => conn.execute_batch("COMMIT"),
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

pub fn get_story_state(conn: &Connection, session_id: &str) -> Result<StoryState> {
    use rusqlite::OptionalExtension;
    let raw: Option<String> = conn
        .query_row(
            "SELECT state_json FROM fpv_story_state WHERE session_id = ?1",
            params![session_id],
            |row| row.get(0),
        )
        .optional()?;
    match raw {
        None => Ok(StoryState::default()),
        Some(json) => serde_json::from_str(&json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(0, Type::Text, Box::new(error))
        }),
    }
}

/// How many undo snapshots a session keeps.
///
/// Every turn writes one full state JSON here and nothing ever removed
/// them, so a long story grew this table without bound — and it rides
/// along in `project_export` and in every `backup_app_data`. 200 is far
/// past any undo a person actually performs; the oldest snapshot beyond
/// it is dropped in the same transaction that adds the new one.
const STORY_STATE_HISTORY_DEPTH: i64 = 200;

/// Transactional entry point. Callers that are ALREADY inside a
/// transaction must use [`set_story_state_in_txn`] instead — SQLite has no
/// nested `BEGIN`, and `copy_session` (branch / checkpoint / fork) runs
/// exactly there.
pub fn set_story_state(conn: &Connection, session_id: &str, state: &StoryState) -> Result<()> {
    // Both writes in one transaction. Separately, a failure on the second
    // left a snapshot in the history with no state change to match it —
    // the next undo would then rewind one turn too far, silently.
    conn.execute_batch("BEGIN IMMEDIATE")?;
    match set_story_state_in_txn(conn, session_id, state) {
        Ok(()) => {
            conn.execute_batch("COMMIT")?;
            Ok(())
        }
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

/// The writes themselves, for callers that already hold a transaction.
pub fn set_story_state_in_txn(
    conn: &Connection,
    session_id: &str,
    state: &StoryState,
) -> Result<()> {
    let json = serde_json::to_string(state)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    // Snapshot the PREVIOUS state (default '{}' when none exists yet) so
    // multi-step undo can rewind all the way to the session start (V29).
    conn.execute(
        "INSERT INTO fpv_story_state_history (session_id, state_json)
         SELECT ?1, COALESCE(
           (SELECT state_json FROM fpv_story_state WHERE session_id = ?1),
           '{}'
         )",
        params![session_id],
    )?;
    conn.execute(
        "DELETE FROM fpv_story_state_history
         WHERE session_id = ?1 AND rowid NOT IN (
           SELECT rowid FROM fpv_story_state_history
           WHERE session_id = ?1
           ORDER BY saved_at DESC, rowid DESC
           LIMIT ?2
         )",
        params![session_id, STORY_STATE_HISTORY_DEPTH],
    )?;
    conn.execute(
        "INSERT INTO fpv_story_state (session_id, state_json, updated_at)
         VALUES (?1, ?2, datetime('now'))
         ON CONFLICT(session_id) DO UPDATE SET
           state_json = excluded.state_json,
           updated_at = excluded.updated_at",
        params![session_id, json],
    )?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinnedCanon {
    pub id: String,
    pub content: String,
    pub created_at: String,
}

pub fn list_pinned_canon(conn: &Connection, session_id: &str) -> Result<Vec<PinnedCanon>> {
    let mut statement = conn.prepare(
        "SELECT id, content, created_at FROM story_pinned_canon WHERE session_id = ?1 ORDER BY created_at ASC",
    )?;
    let canon = statement
        .query_map(params![session_id], |row| {
            Ok(PinnedCanon {
                id: row.get(0)?,
                content: row.get(1)?,
                created_at: row.get(2)?,
            })
        })?
        .collect();
    canon
}

pub fn add_pinned_canon(conn: &Connection, session_id: &str, content: &str) -> Result<String> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM story_pinned_canon WHERE session_id = ?1",
        params![session_id],
        |row| row.get(0),
    )?;
    if count >= 12 {
        return Err(rusqlite::Error::InvalidParameterName(
            "A story can have at most 12 pinned canon facts".into(),
        ));
    }
    let id = Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO story_pinned_canon (id, session_id, content) VALUES (?1, ?2, ?3)",
        params![id, session_id, content],
    )?;
    Ok(id)
}

pub fn delete_pinned_canon(conn: &Connection, id: &str) -> Result<()> {
    let changed = conn.execute("DELETE FROM story_pinned_canon WHERE id = ?1", params![id])?;
    if changed == 0 {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    Ok(())
}

pub fn list_messages(conn: &Connection, session_id: &str) -> Result<Vec<Message>> {
    let mut stmt = conn.prepare(
        "SELECT id, session_id, role, content, created_at \
          FROM messages WHERE session_id = ?1 ORDER BY sequence ASC",
    )?;
    let rows = stmt.query_map(params![session_id], |row| {
        Ok(Message {
            id: row.get(0)?,
            session_id: row.get(1)?,
            role: row.get(2)?,
            content: row.get(3)?,
            created_at: row.get(4)?,
        })
    })?;
    rows.collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexEntry {
    pub id: String,
    pub world_id: String,
    pub title: String,
    pub content: String,
    pub triggers: String,
}

/// Insert a new codex entry and return its generated id.
pub fn create_codex_entry(
    conn: &Connection,
    world_id: &str,
    title: &str,
    content: &str,
    triggers: &str,
) -> Result<String> {
    let id = Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO codex_entries (id, world_id, title, content, triggers) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, world_id, title, content, triggers],
    )?;
    Ok(id)
}

/// Upsert a codex entry: insert a new row, or update the existing row
/// when an entry with the same `id` already exists.
pub fn upsert_codex_entry(
    conn: &Connection,
    id: Option<&str>,
    world_id: &str,
    title: &str,
    content: &str,
    triggers: &str,
) -> Result<String> {
    let entry_id = id
        .map(|s| s.to_string())
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    // Ownership guard: an existing entry may only be updated when it
    // belongs to THIS world. A mismatched id + world_id pair would
    // otherwise silently rewrite another world's codex — the same guard
    // save_world applies via codex_belongs_to_world. A fresh id (no row
    // yet) is a legitimate insert and passes through.
    use rusqlite::OptionalExtension;
    let existing_world: Option<String> = conn
        .query_row(
            "SELECT world_id FROM codex_entries WHERE id = ?1",
            params![entry_id],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(existing_world) = existing_world {
        if existing_world != world_id {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }
    }
    conn.execute(
        "INSERT INTO codex_entries (id, world_id, title, content, triggers)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(id) DO UPDATE SET
           title = excluded.title,
           content = excluded.content,
           triggers = excluded.triggers",
        params![entry_id, world_id, title, content, triggers],
    )?;
    Ok(entry_id)
}

/// Undo the last turn of a session: delete the newest message (insertion
/// order via rowid — created_at can tie within the same second), restore the
/// story state from the most recent V29 snapshot (popping it), and drop the
/// memory record for the deleted message. Returns the deleted message id, or
/// `None` when the session has no messages.
pub fn undo_last_turn(conn: &Connection, session_id: &str) -> Result<Option<String>> {
    use rusqlite::OptionalExtension;
    let last: Option<String> = conn
        .query_row(
            "SELECT id FROM messages WHERE session_id = ?1 ORDER BY rowid DESC LIMIT 1",
            params![session_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(message_id) = last else {
        return Ok(None);
    };

    conn.execute_batch("BEGIN IMMEDIATE")?;
    let result = (|| -> Result<()> {
        conn.execute("DELETE FROM messages WHERE id = ?1", params![message_id])?;
        // Restore the story state from the newest snapshot, then pop it.
        let snapshot: Option<String> = conn
            .query_row(
                "SELECT state_json FROM fpv_story_state_history
                 WHERE session_id = ?1 ORDER BY saved_at DESC, rowid DESC LIMIT 1",
                params![session_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(state_json) = snapshot {
            conn.execute(
                "UPDATE fpv_story_state SET state_json = ?2, updated_at = datetime('now')
                 WHERE session_id = ?1",
                params![session_id, state_json],
            )?;
            conn.execute(
                "DELETE FROM fpv_story_state_history WHERE rowid IN (
                    SELECT rowid FROM fpv_story_state_history
                    WHERE session_id = ?1 ORDER BY saved_at DESC, rowid DESC LIMIT 1
                 )",
                params![session_id],
            )?;
        }
        conn.execute(
            "DELETE FROM story_memory_records WHERE session_id = ?1 AND message_id = ?2",
            params![session_id, message_id],
        )?;
        Ok(())
    })();

    match result {
        Ok(()) => {
            conn.execute_batch("COMMIT")?;
            Ok(Some(message_id))
        }
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

pub fn list_codex_for_world(conn: &Connection, world_id: &str) -> Result<Vec<CodexEntry>> {
    let mut stmt = conn.prepare(
        "SELECT id, world_id, title, content, triggers FROM codex_entries WHERE world_id = ?1 ORDER BY title ASC"
    )?;
    let rows = stmt.query_map(params![world_id], |row| {
        Ok(CodexEntry {
            id: row.get(0)?,
            world_id: row.get(1)?,
            title: row.get(2)?,
            content: row.get(3)?,
            triggers: row.get(4)?,
        })
    })?;
    rows.collect()
}

pub fn delete_codex_entry(conn: &Connection, id: &str) -> Result<()> {
    let changed = conn.execute("DELETE FROM codex_entries WHERE id = ?1", params![id])?;
    if changed == 0 {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    Ok(())
}

fn codex_belongs_to_world(conn: &Connection, id: &str, world_id: &str) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM codex_entries WHERE id = ?1 AND world_id = ?2)",
        params![id, world_id],
        |row| row.get(0),
    )
}

fn delete_codex_entry_for_world(conn: &Connection, id: &str, world_id: &str) -> Result<()> {
    let changed = conn.execute(
        "DELETE FROM codex_entries WHERE id = ?1 AND world_id = ?2",
        params![id, world_id],
    )?;
    if changed == 0 {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    Ok(())
}

pub fn update_summary(conn: &Connection, session_id: &str, summary: &str) -> Result<()> {
    conn.execute(
        "UPDATE sessions SET summary = ?1, updated_at = datetime('now') WHERE id = ?2",
        params![summary, session_id],
    )?;
    Ok(())
}

pub fn update_cover_image_path(conn: &Connection, world_id: &str, path: &str) -> Result<()> {
    let changed = conn.execute(
        "UPDATE worlds SET cover_image_path = ?1 WHERE id = ?2",
        params![path, world_id],
    )?;
    if changed == 0 {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::migrations::run(&conn).unwrap();
        conn
    }

    #[test]
    fn world_and_session_roundtrip() {
        let conn = setup();
        let world_id = create_world(
            &conn,
            &NewWorld {
                name: "Test World".into(),
                genre: "fantasy".into(),
                description: None,
                system_prompt: "You are a narrator.".into(),
                accent_color: None,
                is_nsfw: false,
                cover_image_path: None,
                source: "user".into(),
            },
        )
        .unwrap();

        let session_id = create_session(&conn, &world_id).unwrap();
        let bible = VisualBible {
            style: "cinematic anime".into(),
            palette: "indigo and amber".into(),
            character_anchors: "Mara wears a red scarf".into(),
            location_anchors: "The station has a cracked clock".into(),
            negative_prompt: "text, watermark".into(),
        };
        set_visual_bible(&conn, &world_id, &bible).unwrap();
        assert_eq!(
            get_visual_bible(&conn, &world_id).unwrap().style,
            "cinematic anime"
        );
        append_message(&conn, &session_id, "user", "I open the door.").unwrap();
        append_message(&conn, &session_id, "narrator", "The door creaks open.").unwrap();
        append_message(&conn, &session_id, "user", "I step inside.").unwrap();

        let messages = list_messages(&conn, &session_id).unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].content, "I open the door.");
        assert_eq!(messages[2].content, "I step inside.");

        let worlds = list_worlds(&conn).unwrap();
        assert_eq!(worlds.len(), 1);
        assert_eq!(worlds[0].id, world_id);
    }

    #[test]
    fn world_save_commits_editor_data_atomically() {
        let conn = setup();
        let world_id = save_world(
            &conn,
            None,
            &NewWorld {
                name: "Atomic World".into(),
                genre: "fantasy".into(),
                description: None,
                system_prompt: "Narrate atomically.".into(),
                accent_color: None,
                is_nsfw: false,
                cover_image_path: None,
                source: "user".into(),
            },
            &VisualBible {
                style: "ink".into(),
                ..VisualBible::default()
            },
            &[CodexSaveEntry {
                id: None,
                title: "The bell".into(),
                content: "It rings at midnight.".into(),
                triggers: r#"["bell"]"#.into(),
            }],
            &[],
            "local_only",
        )
        .unwrap();

        assert_eq!(list_codex_for_world(&conn, &world_id).unwrap().len(), 1);
        assert_eq!(get_visual_bible(&conn, &world_id).unwrap().style, "ink");
        assert_eq!(
            get_world(&conn, &world_id).unwrap().privacy_mode,
            "local_only"
        );

        save_world(
            &conn,
            Some(&world_id),
            &NewWorld {
                name: "Atomic World".into(),
                genre: "fantasy".into(),
                description: None,
                system_prompt: "Narrate atomically.".into(),
                accent_color: None,
                is_nsfw: false,
                cover_image_path: None,
                source: "user".into(),
            },
            &VisualBible::default(),
            &[],
            &[],
            "cloud_allowed",
        )
        .unwrap();
        assert_eq!(
            get_world(&conn, &world_id).unwrap().privacy_mode,
            "cloud_allowed"
        );
    }

    #[test]
    fn undo_removes_last_message_restores_state_and_pops_snapshot() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::migrations::run(&conn).unwrap();
        let world_id = create_world(
            &conn,
            &NewWorld {
                name: "w".into(),
                genre: "fantasy".into(),
                description: None,
                system_prompt: "".into(),
                accent_color: None,
                is_nsfw: false,
                cover_image_path: None,
                source: "user".into(),
            },
        )
        .unwrap();
        let session_id = create_session(&conn, &world_id).unwrap();
        append_message(&conn, &session_id, "user", "first").unwrap();
        append_message(&conn, &session_id, "narrator", "reply one").unwrap();
        append_message(&conn, &session_id, "user", "second").unwrap();
        append_message(&conn, &session_id, "narrator", "reply two").unwrap();

        // Two state snapshots: before turn 1 and before turn 2.
        let state1 = StoryState {
            turn: 1,
            ..Default::default()
        };
        set_story_state(&conn, &session_id, &state1).unwrap();
        let state2 = StoryState {
            turn: 2,
            ..Default::default()
        };
        set_story_state(&conn, &session_id, &state2).unwrap();

        // Undo pops "reply two", restores turn 1's state.
        assert!(undo_last_turn(&conn, &session_id).unwrap().is_some());
        let messages: Vec<(String, String)> = conn
            .prepare("SELECT role, content FROM messages WHERE session_id = ?1 ORDER BY rowid")
            .unwrap()
            .query_map([session_id.as_str()], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[2], ("user".to_string(), "second".to_string()));
        assert_eq!(get_story_state(&conn, &session_id).unwrap().turn, 1);

        // Undo again pops "second" (a user message) and restores no-state.
        assert!(undo_last_turn(&conn, &session_id).unwrap().is_some());
        assert_eq!(get_story_state(&conn, &session_id).unwrap().turn, 0);
    }

    #[test]
    fn story_state_history_stays_bounded_and_keeps_the_newest_snapshots() {
        let conn = setup();
        let world_id = create_world(
            &conn,
            &NewWorld {
                name: "Bound".into(),
                genre: "fantasy".into(),
                description: None,
                system_prompt: "You are a narrator.".into(),
                accent_color: None,
                is_nsfw: false,
                cover_image_path: None,
                source: "user".into(),
            },
        )
        .unwrap();
        let session_id = create_session(&conn, &world_id).unwrap();

        // One snapshot per turn, well past the cap.
        for turn in 0..(STORY_STATE_HISTORY_DEPTH + 50) {
            let state = StoryState {
                active_goals: vec![format!("turn-{turn}")],
                ..StoryState::default()
            };
            set_story_state(&conn, &session_id, &state).unwrap();
        }

        let rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM fpv_story_state_history WHERE session_id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            rows, STORY_STATE_HISTORY_DEPTH,
            "history grew past the cap — this table rides along in every backup"
        );

        // The cap must drop the OLDEST, so undo still rewinds one real turn.
        let newest: String = conn
            .query_row(
                "SELECT state_json FROM fpv_story_state_history WHERE session_id = ?1
                 ORDER BY saved_at DESC, rowid DESC LIMIT 1",
                params![session_id],
                |row| row.get(0),
            )
            .unwrap();
        let expected = STORY_STATE_HISTORY_DEPTH + 48;
        assert!(
            newest.contains(&format!("turn-{expected}")),
            "the newest snapshot was pruned instead of the oldest: {newest}"
        );
    }

    #[test]
    fn undo_on_empty_session_is_a_noop() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::migrations::run(&conn).unwrap();
        let world_id = create_world(
            &conn,
            &NewWorld {
                name: "w".into(),
                genre: "fantasy".into(),
                description: None,
                system_prompt: "".into(),
                accent_color: None,
                is_nsfw: false,
                cover_image_path: None,
                source: "user".into(),
            },
        )
        .unwrap();
        let session_id = create_session(&conn, &world_id).unwrap();
        assert_eq!(undo_last_turn(&conn, &session_id).unwrap(), None);
    }

    #[test]
    fn world_save_rolls_back_on_cross_world_codex_id() {
        let conn = setup();
        let first = create_world(
            &conn,
            &NewWorld {
                name: "Original".into(),
                genre: "fantasy".into(),
                description: None,
                system_prompt: "Narrate.".into(),
                accent_color: None,
                is_nsfw: false,
                cover_image_path: None,
                source: "user".into(),
            },
        )
        .unwrap();
        let second = create_world(
            &conn,
            &NewWorld {
                name: "Other".into(),
                genre: "horror".into(),
                description: None,
                system_prompt: "Narrate.".into(),
                accent_color: None,
                is_nsfw: false,
                cover_image_path: None,
                source: "user".into(),
            },
        )
        .unwrap();
        let foreign_id =
            create_codex_entry(&conn, &second, "Foreign", "Do not move me.", "[]").unwrap();

        let result = save_world(
            &conn,
            Some(&first),
            &NewWorld {
                name: "Should Roll Back".into(),
                genre: "fantasy".into(),
                description: None,
                system_prompt: "Changed.".into(),
                accent_color: None,
                is_nsfw: false,
                cover_image_path: None,
                source: "user".into(),
            },
            &VisualBible {
                style: "should not persist".into(),
                ..VisualBible::default()
            },
            &[CodexSaveEntry {
                id: Some(foreign_id),
                title: "Invalid move".into(),
                content: "This must fail.".into(),
                triggers: "[]".into(),
            }],
            &[],
            "local_only",
        );

        assert!(result.is_err());
        assert_eq!(get_world(&conn, &first).unwrap().name, "Original");
        assert_eq!(
            get_visual_bible(&conn, &first).unwrap(),
            VisualBible::default()
        );
        assert!(list_codex_for_world(&conn, &first).unwrap().is_empty());
        assert_eq!(list_codex_for_world(&conn, &second).unwrap().len(), 1);
    }

    #[test]
    fn story_lifecycle_supports_codex_state_branch_and_cleanup() {
        let conn = setup();
        let world_id = create_world(
            &conn,
            &NewWorld {
                name: "Lifecycle World".into(),
                genre: "horror".into(),
                description: Some("A village with one locked church.".into()),
                system_prompt: "Write atmospheric second-person horror.".into(),
                accent_color: None,
                is_nsfw: false,
                cover_image_path: None,
                source: "user".into(),
            },
        )
        .unwrap();
        let session_id = create_session(&conn, &world_id).unwrap();

        let codex_id = upsert_codex_entry(
            &conn,
            None,
            &world_id,
            "The locked church",
            "The bell rings when nobody is inside.",
            r#"["church", "bell"]"#,
        )
        .unwrap();
        assert_eq!(list_codex_for_world(&conn, &world_id).unwrap().len(), 1);

        append_message(&conn, &session_id, "user", "I enter the church.").unwrap();
        append_message(&conn, &session_id, "narrator", "The bell rings above you.").unwrap();
        let state = StoryState {
            location: "church".into(),
            facts: vec!["The bell rings without a hand.".into()],
            ..StoryState::default()
        };
        set_story_state(&conn, &session_id, &state).unwrap();
        update_summary(&conn, &session_id, "The player entered the church.").unwrap();

        let branch_id = branch_session(&conn, &session_id, "Open the crypt").unwrap();
        assert_ne!(branch_id, session_id);
        assert_eq!(list_messages(&conn, &branch_id).unwrap().len(), 2);
        assert_eq!(
            get_story_state(&conn, &branch_id).unwrap().location,
            "church"
        );
        assert_eq!(
            get_session_summary(&conn, &branch_id).unwrap(),
            "The player entered the church."
        );

        let canon_id =
            add_pinned_canon(&conn, &session_id, "The bell cannot be silenced.").unwrap();
        let checkpoint_id = checkpoint_session(&conn, &session_id, "Before the crypt").unwrap();
        assert_eq!(
            list_pinned_canon(&conn, &checkpoint_id).unwrap()[0].content,
            "The bell cannot be silenced."
        );
        let sessions = list_sessions_for_world(&conn, &world_id).unwrap();
        let checkpoint = sessions
            .iter()
            .find(|session| session.id == checkpoint_id)
            .unwrap();
        assert!(checkpoint.is_checkpoint);
        assert_eq!(
            checkpoint.parent_session_id.as_deref(),
            Some(session_id.as_str())
        );
        assert_eq!(checkpoint.branch_label, "Before the crypt");
        delete_pinned_canon(&conn, &canon_id).unwrap();

        delete_codex_entry(&conn, &codex_id).unwrap();
        assert!(list_codex_for_world(&conn, &world_id).unwrap().is_empty());
        delete_session(&conn, &branch_id).unwrap();
        assert!(list_messages(&conn, &branch_id).unwrap().is_empty());
        delete_world(&conn, &world_id).unwrap();
        assert!(list_worlds(&conn).unwrap().is_empty());
    }

    #[test]
    fn replaces_narrator_content_without_changing_message_count() {
        let conn = setup();
        let world_id = create_world(
            &conn,
            &NewWorld {
                name: "Replace World".into(),
                genre: "fantasy".into(),
                description: None,
                system_prompt: "Narrate.".into(),
                accent_color: None,
                is_nsfw: false,
                cover_image_path: None,
                source: "user".into(),
            },
        )
        .unwrap();
        let session_id = create_session(&conn, &world_id).unwrap();
        append_message(&conn, &session_id, "user", "Open the door.").unwrap();
        let narrator_id =
            append_message(&conn, &session_id, "narrator", "The door opens.").unwrap();

        replace_message_content(
            &conn,
            &session_id,
            &narrator_id,
            "The door opens onto darkness.",
        )
        .unwrap();

        let messages = list_messages(&conn, &session_id).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].id, narrator_id);
        assert_eq!(messages[1].content, "The door opens onto darkness.");
    }

    #[test]
    fn regeneration_clears_only_derived_session_data() {
        let conn = setup();
        let world_id = create_world(
            &conn,
            &NewWorld {
                name: "Regenerate World".into(),
                genre: "fantasy".into(),
                description: None,
                system_prompt: "Narrate.".into(),
                accent_color: None,
                is_nsfw: false,
                cover_image_path: None,
                source: "user".into(),
            },
        )
        .unwrap();
        let session_id = create_session(&conn, &world_id).unwrap();
        append_message(&conn, &session_id, "user", "Open the door.").unwrap();
        let narrator_id =
            append_message(&conn, &session_id, "narrator", "The door opens.").unwrap();
        set_story_state(
            &conn,
            &session_id,
            &StoryState {
                location: "hall".into(),
                ..Default::default()
            },
        )
        .unwrap();
        update_summary(&conn, &session_id, "The player opened the door.").unwrap();
        conn.execute(
            "INSERT INTO story_memory_records (id, session_id, message_id, content, embedding) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![Uuid::new_v4().to_string(), session_id, narrator_id, "old scene", vec![0u8; 4]],
        ).unwrap();

        clear_regeneration_derived_data(&conn, &session_id, &narrator_id).unwrap();

        assert_eq!(list_messages(&conn, &session_id).unwrap().len(), 2);
        assert_eq!(get_story_state(&conn, &session_id).unwrap().location, "");
        assert_eq!(get_session_summary(&conn, &session_id).unwrap(), "");
        let memory_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM story_memory_records WHERE session_id = ?1 AND message_id = ?2",
            params![session_id, narrator_id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(memory_count, 0);
    }

    #[test]
    fn legacy_replace_rejects_messages_from_different_sessions() {
        let conn = setup();
        let world_id = create_world(
            &conn,
            &NewWorld {
                name: "Replace World".into(),
                genre: "fantasy".into(),
                description: None,
                system_prompt: "Narrate.".into(),
                accent_color: None,
                is_nsfw: false,
                cover_image_path: None,
                source: "user".into(),
            },
        )
        .unwrap();
        let first = create_session(&conn, &world_id).unwrap();
        let second = create_session(&conn, &world_id).unwrap();
        let old_id = append_message(&conn, &first, "narrator", "Old.").unwrap();
        let new_id = append_message(&conn, &second, "narrator", "New.").unwrap();

        assert!(replace_message(&conn, &old_id, &new_id, "Replacement.").is_err());
        assert_eq!(list_messages(&conn, &first).unwrap()[0].content, "Old.");
        assert_eq!(list_messages(&conn, &second).unwrap()[0].content, "New.");
    }
}
