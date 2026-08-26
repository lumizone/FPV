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
    ("v30_drop_local_waifu_tables", super::schema::V30),
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

    /// The dead Local Waifu tables are gone, the live FPV ones are not,
    /// and the drops succeed with `foreign_keys` ON — which the in-memory
    /// chain test above does not exercise, since enforcement is switched
    /// on in `db::open_connection`, not by `run`. Get the drop order wrong
    /// and this is what fails.
    #[test]
    fn v30_drops_every_local_waifu_table_with_foreign_keys_enforced() {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        run(&conn).unwrap();

        let exists = |table: &str| -> bool {
            conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
                [table],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
                == 1
        };

        for dead in [
            "anticipations",
            "behavior_history",
            "chat_messages",
            "characters",
            "conversations",
            "entities",
            "feedback_events",
            "lorebook_entries",
            "memories",
            "memory_blocks",
            "mood_state",
            "open_threads",
            "proactive_log",
            "relations",
            "relationship",
            "relationship_milestones",
            "surprisal_stats",
        ] {
            assert!(
                !exists(dead),
                "`{dead}` is Local Waifu's and must be dropped"
            );
        }

        for live in [
            "app_meta",
            "worlds",
            "sessions",
            "messages",
            "fpv_characters",
            "fpv_story_state",
            "fpv_story_state_history",
            "codex_entries",
            "story_memory_records",
            "story_memory_fts",
            "story_pinned_canon",
            "world_visual_bibles",
            "cloud_daily_spend_ledger",
        ] {
            assert!(exists(live), "`{live}` is FPV's own and must survive");
        }
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

    // Local Waifu's own v15 / v17 backfill tests lived here. They seeded
    // `characters`, `memories` and `chat_messages` and asserted on the
    // result — tables v30 now drops at the end of the chain, so the
    // assertions had nothing left to read. The backfills themselves still
    // run on a fresh database and are covered by
    // `full_chain_runs_and_is_idempotent`; what they produce is dropped
    // immediately afterwards and is no longer observable in FPV.

    #[test]
    fn v17_registered_body_contains_the_backfill_half() {
        assert!(
            super::super::schema::V17.contains(super::super::schema::V17_BACKFILL),
            "V17 must embed V17_BACKFILL verbatim so the two can't drift apart"
        );
    }
}
