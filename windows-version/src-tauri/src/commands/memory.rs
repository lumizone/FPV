use serde::Serialize;
use tauri::{AppHandle, State};
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct StoryMemoryRecall {
    pub message_id: String,
    pub content: String,
    pub score: f32,
}

#[derive(Debug, Serialize)]
pub struct StoryMemoryRecord {
    pub message_id: String,
    pub content: String,
    pub created_at: String,
}

fn vector_from_bytes(bytes: &[u8]) -> Option<Vec<f32>> {
    if bytes.len() % 4 != 0 {
        return None;
    }
    Some(
        bytes
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
            .collect(),
    )
}

fn vector_to_bytes(vector: &[f32]) -> Vec<u8> {
    vector
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    if left.len() != right.len() || left.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0;
    let mut left_norm = 0.0;
    let mut right_norm = 0.0;
    for (a, b) in left.iter().zip(right) {
        dot += a * b;
        left_norm += a * a;
        right_norm += b * b;
    }
    if left_norm == 0.0 || right_norm == 0.0 {
        0.0
    } else {
        dot / (left_norm.sqrt() * right_norm.sqrt())
    }
}

fn fts_query(text: &str) -> String {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|term| term.chars().count() >= 3)
        .take(12)
        .map(|term| format!("\"{}\"", term.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" OR ")
}

async fn embedding_model(state: &AppState) -> Option<String> {
    let configured = {
        let conn = state.db.lock().await;
        crate::db::meta_get(&conn, "user_embed_model")
            .ok()
            .flatten()
    };
    let model = configured.unwrap_or_else(|| "embeddinggemma".into());
    let installed = state.ollama.list_models().await.ok()?;
    installed
        .iter()
        .find(|entry| entry.name == model || entry.name.starts_with(&(model.clone() + ":")))
        .map(|entry| entry.name.clone())
}

async fn memory_enabled(state: &AppState) -> bool {
    let conn = state.db.lock().await;
    crate::db::meta_get(&conn, "semanticMemoryEnabled")
        .ok()
        .flatten()
        .is_some_and(|value| value == "true")
}

#[tauri::command(rename_all = "snake_case")]
pub async fn story_memory_index(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    message_id: String,
    content: String,
) -> AppResult<()> {
    crate::storage::fs_safe_id(&session_id)?;
    crate::storage::fs_safe_id(&message_id)?;
    if content.trim().is_empty() || content.chars().count() > 20_000 {
        return Err(AppError::Config(
            "story memory content must contain 1-20000 characters".into(),
        ));
    }
    if !memory_enabled(&state).await {
        return Ok(());
    }
    {
        let conn = state.db.lock().await;
        let owns_message: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM messages m JOIN sessions s ON s.id = m.session_id WHERE m.id = ?1 AND m.session_id = ?2)",
            rusqlite::params![message_id, session_id],
            |row| row.get(0),
        )?;
        if !owns_message {
            return Err(AppError::Config(
                "memory message does not belong to session".into(),
            ));
        }
    }
    state.ensure_ollama(&app).await?;
    let Some(model) = embedding_model(&state).await else {
        return Ok(());
    };
    let embedding = state.ollama.embed_document(&model, &content).await?;
    let conn = state.db.lock().await;
    conn.execute(
        "INSERT INTO story_memory_records (id, session_id, message_id, content, embedding) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(session_id, message_id) DO UPDATE SET content = excluded.content, embedding = excluded.embedding, created_at = datetime('now')",
        rusqlite::params![Uuid::new_v4().to_string(), session_id, message_id, content, vector_to_bytes(&embedding)],
    )?;
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn story_memory_recall(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    query: String,
    limit: usize,
) -> AppResult<Vec<StoryMemoryRecall>> {
    crate::storage::fs_safe_id(&session_id)?;
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    if !memory_enabled(&state).await {
        return Ok(Vec::new());
    }
    state.ensure_ollama(&app).await?;
    let Some(model) = embedding_model(&state).await else {
        return Ok(Vec::new());
    };
    let candidates = {
        let conn = state.db.lock().await;
        let record_count: usize = conn.query_row(
            "SELECT COUNT(*) FROM story_memory_records WHERE session_id = ?1",
            rusqlite::params![session_id],
            |row| row.get(0),
        )?;
        if record_count == 0 {
            return Ok(Vec::new());
        }
        let lexical = fts_query(&query);
        let (sql, match_query) = if record_count > 64 && !lexical.is_empty() {
            (
                "SELECT r.message_id, r.content, r.embedding
                 FROM story_memory_fts
                 JOIN story_memory_records r
                   ON r.message_id = story_memory_fts.message_id AND r.session_id = story_memory_fts.session_id
                 WHERE story_memory_fts.session_id = ?1 AND story_memory_fts MATCH ?2
                 ORDER BY bm25(story_memory_fts) LIMIT 50",
                Some(lexical),
            )
        } else {
            (
                "SELECT message_id, content, embedding FROM story_memory_records WHERE session_id = ?1",
                None,
            )
        };
        let mut statement = conn.prepare(sql)?;
        let mut candidates: Vec<(String, String, Vec<u8>)> = Vec::new();
        let mut rows = if let Some(ref match_query) = match_query {
            statement.query(rusqlite::params![session_id, match_query])?
        } else {
            statement.query(rusqlite::params![session_id])?
        };
        while let Some(row) = rows.next()? {
            candidates.push((row.get(0)?, row.get(1)?, row.get(2)?));
        }
        drop(rows);
        drop(statement);
        if candidates.is_empty() && match_query.is_some() {
            let mut fallback = conn.prepare(
                "SELECT message_id, content, embedding FROM story_memory_records
                 WHERE session_id = ?1 ORDER BY created_at DESC LIMIT 50",
            )?;
            let mut rows = fallback.query(rusqlite::params![session_id])?;
            while let Some(row) = rows.next()? {
                candidates.push((row.get(0)?, row.get(1)?, row.get(2)?));
            }
        }
        candidates
    };
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let query_embedding = state.ollama.embed_query(&model, &query).await?;
    let mut matches: Vec<StoryMemoryRecall> = candidates
        .into_iter()
        .filter_map(|(message_id, content, bytes)| {
            vector_from_bytes(&bytes).map(|vector| StoryMemoryRecall {
                message_id,
                content,
                score: cosine_similarity(&query_embedding, &vector),
            })
        })
        .filter(|item| item.score >= 0.35)
        .collect();
    matches.sort_by(|left, right| right.score.total_cmp(&left.score));
    matches.truncate(limit.clamp(1, 8));
    Ok(matches)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn story_memory_list(
    state: State<'_, AppState>,
    session_id: String,
    limit: usize,
) -> AppResult<Vec<StoryMemoryRecord>> {
    crate::storage::fs_safe_id(&session_id)?;
    let conn = state.db.lock().await;
    let mut statement = conn.prepare(
        "SELECT message_id, content, created_at FROM story_memory_records
         WHERE session_id = ?1 ORDER BY created_at DESC LIMIT ?2",
    )?;
    let records = statement
        .query_map(rusqlite::params![session_id, limit.clamp(1, 20)], |row| {
            Ok(StoryMemoryRecord {
                message_id: row.get(0)?,
                content: row.get(1)?,
                created_at: row.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::{cosine_similarity, fts_query};

    #[test]
    fn cosine_similarity_ranks_matching_vectors() {
        assert!(
            cosine_similarity(&[1.0, 0.0], &[1.0, 0.0])
                > cosine_similarity(&[1.0, 0.0], &[0.0, 1.0])
        );
    }

    #[test]
    fn fts_query_keeps_useful_terms_and_drops_noise() {
        assert_eq!(
            fts_query("I ask Mara about the brass key"),
            "\"ask\" OR \"Mara\" OR \"about\" OR \"the\" OR \"brass\" OR \"key\""
        );
    }
}
