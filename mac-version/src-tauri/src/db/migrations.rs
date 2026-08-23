use rusqlite::Connection;

use crate::error::AppResult;

/// SQL-quote an identifier-ish string literal. The migration IDs we
/// embed inline are compile-time constants from `MIGRATIONS`, so this
/// is defensive (no SQLi vector), but we still double single-quotes
/// to be safe against future contributors adding an apostrophe to a
/// migration name.
fn sql_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

const MIGRATIONS: &[(&str, &str)] = &[
    ("v1_initial", super::schema::V1),
    ("v2_memory_kg", super::schema::V2),
    ("v3_relationship", super::schema::V3),
    ("v4_character_language", super::schema::V4),
    ("v5_perf_indexes", super::schema::V5),
    ("v6_character_portrait", super::schema::V6),
    ("v7_persona_override", super::schema::V7),
    ("v8_memory_provenance", super::schema::V8),
    ("v9_chat_thoughts", super::schema::V9),
    ("v10_open_threads", super::schema::V10),
    ("v11_memory_blocks", super::schema::V11),
    ("v12_lorebook", super::schema::V12),
    ("v13_anticipations", super::schema::V13),
    ("v14_behavior_history_and_surprisal", super::schema::V14),
    ("v15_embeddinggemma_migration", super::schema::V15),
    ("v16_chat_image_path", super::schema::V16),
    ("v17_conversations", super::schema::V17),
    ("v18_fpv_schema", super::schema::V18),
    ("v19_fpv_characters", super::schema::V19),
    ("v20_fpv_story_state", super::schema::V20),
    ("v21_fpv_session_branches", super::schema::V21),
    ("v22_message_sequence", super::schema::V22),
    ("v23_story_memory_records", super::schema::V23),
    ("v24_story_memory_fts", super::schema::V24),
    ("v25_story_checkpoints_and_pinned_canon", super::schema::V25),
    ("v26_world_visual_bibles", super::schema::V26),
    ("v27_world_privacy_mode", super::schema::V27),
    ("v28_cloud_daily_spend_ledger", super::schema::V28),
    ("v29_story_state_history", super::schema::V29),
];

pub fn run(conn: &Connection) -> AppResult<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            id TEXT PRIMARY KEY,
            applied_at INTEGER NOT NULL
        );",
    )?;

    for (id, sql) in MIGRATIONS {
        let already: i64 = conn.query_row(
            "SELECT COUNT(*) FROM schema_migrations WHERE id = ?1",
            [id],
            |r| r.get(0),
        )?;
        if already > 0 {
            continue;
        }
        // Wrap each migration in BEGIN/COMMIT so a multi-statement batch
        // either applies fully or rolls back fully. The
        // `schema_migrations` row is inserted INSIDE the same transaction
        // so the migration body and its bookkeeping commit (or roll
        // back) atomically. Earlier the bookkeeping INSERT ran AFTER
        // the body's COMMIT — a crash between the two left the body
        // applied but unrecorded, and the next launch would re-run an
        // already-applied `ALTER TABLE ADD COLUMN` (fatal: SQLite has no
        // `IF NOT EXISTS` for that). The atomic form below closes that
        // window.
        let applied_at = chrono::Utc::now().timestamp();
        let txn_sql = format!(
            "BEGIN;\n{sql}\nINSERT INTO schema_migrations (id, applied_at) VALUES ({id_lit}, {applied_at});\nCOMMIT;",
            id_lit = sql_quote(id),
        );
        if let Err(e) = conn.execute_batch(&txn_sql) {
            // ROLLBACK is implicit when COMMIT is never reached, but be
            // defensive in case sqlite left the transaction open.
            let _ = conn.execute_batch("ROLLBACK;");
            return Err(e.into());
        }
        tracing::info!(migration = id, "applied migration");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_chain_runs_and_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        // First run applies every migration (incl. V15) cleanly.
        run(&conn).unwrap();
        let applied: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(applied as usize, MIGRATIONS.len());
        // Second run is a no-op (every id already recorded).
        run(&conn).unwrap();
        let applied_again: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(applied_again, applied);
    }

    #[test]
    fn full_chain_survives_file_database_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("migration-test.sqlite");
        {
            let conn = Connection::open(&path).unwrap();
            run(&conn).unwrap();
            let applied: i64 = conn
                .query_row("SELECT COUNT(*) FROM schema_migrations", [], |r| r.get(0))
                .unwrap();
            assert_eq!(applied as usize, MIGRATIONS.len());
        }

        let conn = Connection::open(&path).unwrap();
        run(&conn).unwrap();
        let applied: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(applied as usize, MIGRATIONS.len());
        let _: i64 = conn
            .query_row("SELECT COUNT(*) FROM worlds", [], |r| r.get(0))
            .unwrap();
    }

    #[test]
    fn v15_migrates_saved_embed_pref_and_nulls_vectors() {
        let conn = Connection::open_in_memory().unwrap();
        run(&conn).unwrap();
        // Seed a character + memory with a fake embedding + the old
        // saved embed preference, then re-apply V15's body directly to
        // confirm its effect (FK enforcement is on, so the character
        // row is required first).
        conn.execute(
            "INSERT INTO characters (id, name, created_at, last_active_at) VALUES ('c1','Yumi',0,0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memories (id, character_id, kind, content, importance, emotional_weight, embedding, created_at, ref_count)
             VALUES ('m1','c1','fact','hi',0.5,0.0,X'00',0,0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO app_meta (key, value) VALUES ('user_embed_model','nomic-embed-text')",
            [],
        )
        .unwrap();

        conn.execute_batch(super::super::schema::V15).unwrap();

        let emb: Option<Vec<u8>> = conn
            .query_row("SELECT embedding FROM memories WHERE id='m1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(emb.is_none(), "embedding should be nulled");
        let pref: String = conn
            .query_row(
                "SELECT value FROM app_meta WHERE key='user_embed_model'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(pref, "embeddinggemma");
    }

    #[test]
    fn v17_backfills_one_conversation_per_character_and_adopts_messages() {
        let conn = Connection::open_in_memory().unwrap();
        run(&conn).unwrap();
        // Two characters, each with a message, written the way the app
        // wrote them BEFORE V17 (conversation_id left NULL).
        for (cid, name) in [("c1", "Yumi"), ("c2", "Aki")] {
            conn.execute(
                "INSERT INTO characters (id, name, created_at, last_active_at) VALUES (?1, ?2, 0, 0)",
                rusqlite::params![cid, name],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO chat_messages (id, character_id, role, content, source, created_at, conversation_id)
                 VALUES (?1, ?2, 'user', 'hi', 'app', 0, NULL)",
                rusqlite::params![format!("m_{cid}"), cid],
            )
            .unwrap();
        }
        // Re-run just the (idempotent) backfill half of V17 directly to
        // exercise it against the seeded rows (the initial run() already
        // applied the one-shot schema-shape half + backfilled the empty
        // DB; V17_SCHEMA's ALTER TABLE can't be re-run, but V17_BACKFILL
        // is safe to re-run any number of times).
        conn.execute_batch(super::super::schema::V17_BACKFILL)
            .unwrap();

        let conv_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM conversations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(conv_count, 2);
        let orphans: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chat_messages WHERE conversation_id IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(orphans, 0);
        let mismatched: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chat_messages m
                 JOIN conversations c ON c.id = m.conversation_id
                 WHERE c.character_id != m.character_id",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(mismatched, 0);
    }

    // Guard against silent drift between the registered migration body
    // (`V17`) and the re-runnable backfill half (`V17_BACKFILL`). `V17`
    // is `concat!(schema_shape, backfill)` — a textual copy of the
    // backfill — so if someone edits `V17_BACKFILL` without updating
    // `V17`'s copy, the real migration and the tested/repair backfill
    // would diverge. This assertion keeps them locked together.
    #[test]
    fn v17_registered_body_contains_the_backfill_half() {
        assert!(
            super::super::schema::V17.contains(super::super::schema::V17_BACKFILL),
            "V17 must embed V17_BACKFILL verbatim so the two can't drift apart"
        );
    }
}
