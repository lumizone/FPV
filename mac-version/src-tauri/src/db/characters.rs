use rusqlite::{params, Connection, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewCharacter {
    pub world_id: String,
    pub name: String,
    pub role: Option<String>,
    pub backstory: Option<String>,
    pub traits: Option<Vec<String>>,
    pub avatar_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Character {
    pub id: String,
    pub world_id: String,
    pub name: String,
    pub role: String,
    pub backstory: Option<String>,
    pub traits: String,
    pub avatar_path: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

pub fn create(conn: &Connection, c: &NewCharacter) -> Result<String> {
    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp();
    let role = c.role.as_deref().unwrap_or("npc");
    let traits_json = c
        .traits
        .as_ref()
        .map(|t| serde_json::to_string(t).unwrap_or_else(|_| "[]".into()))
        .unwrap_or_else(|| "[]".into());

    conn.execute(
        "INSERT INTO fpv_characters (id, world_id, name, role, backstory, traits, avatar_path, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![id, c.world_id, c.name, role, c.backstory, traits_json, c.avatar_path, now, now],
    )?;
    Ok(id)
}

pub fn list_for_world(conn: &Connection, world_id: &str) -> Result<Vec<Character>> {
    let mut stmt = conn.prepare(
        "SELECT id, world_id, name, role, backstory, traits, avatar_path, created_at, updated_at \
         FROM fpv_characters WHERE world_id = ?1 ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map(params![world_id], |row| {
        Ok(Character {
            id: row.get(0)?,
            world_id: row.get(1)?,
            name: row.get(2)?,
            role: row.get(3)?,
            backstory: row.get(4)?,
            traits: row.get(5)?,
            avatar_path: row.get(6)?,
            created_at: row.get(7)?,
            updated_at: row.get(8)?,
        })
    })?;
    rows.collect()
}

pub fn get(conn: &Connection, id: &str) -> Result<Character> {
    conn.query_row(
        "SELECT id, world_id, name, role, backstory, traits, avatar_path, created_at, updated_at \
         FROM fpv_characters WHERE id = ?1",
        params![id],
        |row| {
            Ok(Character {
                id: row.get(0)?,
                world_id: row.get(1)?,
                name: row.get(2)?,
                role: row.get(3)?,
                backstory: row.get(4)?,
                traits: row.get(5)?,
                avatar_path: row.get(6)?,
                created_at: row.get(7)?,
                updated_at: row.get(8)?,
            })
        },
    )
}

/// Update a character's optional fields. Only non-None fields are applied.
pub fn update(
    conn: &Connection,
    id: &str,
    name: Option<&str>,
    role: Option<&str>,
    backstory: Option<&str>,
    traits: Option<&[String]>,
    avatar_path: Option<&str>,
) -> Result<()> {
    let now = chrono::Utc::now().timestamp();

    // Apply every provided field in ONE transaction so a mid-way failure
    // cannot leave a half-updated character on disk.
    let tx = conn.unchecked_transaction()?;
    // Build SET clauses dynamically for each provided field.
    if let Some(n) = name {
        conn.execute(
            "UPDATE fpv_characters SET name = ?1, updated_at = ?2 WHERE id = ?3",
            params![n, now, id],
        )?;
    }
    if let Some(r) = role {
        conn.execute(
            "UPDATE fpv_characters SET role = ?1, updated_at = ?2 WHERE id = ?3",
            params![r, now, id],
        )?;
    }
    if let Some(b) = backstory {
        conn.execute(
            "UPDATE fpv_characters SET backstory = ?1, updated_at = ?2 WHERE id = ?3",
            params![b, now, id],
        )?;
    }
    if let Some(t) = traits {
        let traits_json = serde_json::to_string(t).unwrap_or_else(|_| "[]".into());
        conn.execute(
            "UPDATE fpv_characters SET traits = ?1, updated_at = ?2 WHERE id = ?3",
            params![traits_json, now, id],
        )?;
    }
    if let Some(a) = avatar_path {
        conn.execute(
            "UPDATE fpv_characters SET avatar_path = ?1, updated_at = ?2 WHERE id = ?3",
            params![a, now, id],
        )?;
    }
    tx.commit()?;
    Ok(())
}

pub fn delete(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("DELETE FROM fpv_characters WHERE id = ?1", params![id])?;
    Ok(())
}
