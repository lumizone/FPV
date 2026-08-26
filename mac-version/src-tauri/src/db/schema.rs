// V4: per-character language for prompt building. Backfills from
// existing soul-file `language_primary` aren't possible without the
// master key (which the migration runner doesn't have), so existing
// rows get the 'en' default. The user-facing fix is the
// `character_set_language` Tauri command — Settings → General also
// calls it whenever the user toggles UI language.
pub const V4: &str = r#"
ALTER TABLE characters ADD COLUMN language_primary TEXT NOT NULL DEFAULT 'en';
"#;

// V5: indexes that flatten the SQL hot path identified in the SQL/perf
// audit. `idx_memories_char_time` lets `list_recent` use the index for
// its ORDER BY rather than sorting in memory (matters for character
// export at limit=1000). `idx_relations_unique` lets `upsert_relation`
// switch from SELECT-then-INSERT/UPDATE to a single ON CONFLICT.
pub const V5: &str = r#"
CREATE INDEX IF NOT EXISTS idx_memories_char_time
    ON memories(character_id, created_at DESC);

CREATE UNIQUE INDEX IF NOT EXISTS idx_relations_unique
    ON relations(from_entity_id, to_entity_id, relation_kind);
"#;

// V6: per-character portrait. NULL = no portrait set, fall back to the
// initials gradient in the UI. Stored as a path under
// `<data_dir>/portraits/<character_id>.<ext>` rather than embedding the
// bytes in SQLite — a typical portrait is 200-800 KB and would bloat
// the DB / WAL / backup window with no upside (we'd never query it).
// Frontend reads it via Tauri's `convertFileSrc` and renders it through
// `<img src>`.
pub const V6: &str = r#"
ALTER TABLE characters ADD COLUMN portrait_path TEXT;
"#;

// V7: user-editable persona override (Hermes SOUL.md pattern). Lets
// the user nudge tone / mannerisms / quirks without restarting
// onboarding. Stored on `characters` rather than as a soul-file slot
// because it's plain text the user types in the settings UI — no
// need to re-key the cocoon encryption every time they tweak a line.
// Capped at 2,000 chars by the Tauri setter, mirroring the bounded-
// digest discipline we already apply elsewhere in the prompt.
pub const V7: &str = r#"
ALTER TABLE characters ADD COLUMN persona_override TEXT;
"#;

// V8: memory provenance. `source_message_ids` stores the chat message
// IDs the entry was derived from as a JSON array (`["id1","id2"]`),
// so a future digest renderer can cite ("Last Tuesday you mentioned
// you don't drink coffee") instead of asserting facts in a vacuum.
// NULL means "no provenance recorded" — true for everything written
// before v0.1.13 plus anything that's a free-floating import / manual
// edit. Used as a JSON blob rather than a join table because the
// pointer is informational, not relational (we don't need referential
// integrity — a deleted message shouldn't invalidate the inference
// the LLM made from it).
pub const V8: &str = r#"
ALTER TABLE memories ADD COLUMN source_message_ids TEXT;
"#;

// V9: inner thoughts (AIRI-style). Optional JSON array of the
// assistant's internal narration extracted from `<thought>…</thought>`
// blocks in the model output. The post-stream parser strips the blocks
// from what the user sees and stores the joined text here. NULL means
// the model didn't emit any thoughts that turn — most replies will be
// NULL on cheap local models, but bigger models lean into the
// affordance once it's offered in the system prompt.
pub const V9: &str = r#"
ALTER TABLE chat_messages ADD COLUMN thoughts_json TEXT;
"#;

// V10: open conversation threads. A "thread" is a topic the character
// is mid-conversation about and hasn't resolved — "your job interview
// on Friday", "the wedding planning row with your mum". Drives both
// in-context callbacks ("how did Friday go?") and idle-trigger
// openers (background/triggers picks the oldest still-open thread).
// `status` is one of "open" / "resolved" / "stale"; resolved + stale
// stick around for context but aren't surfaced for callbacks. The
// memory tool's `[[THREAD ...]]` markers write here.
pub const V10: &str = r#"
CREATE TABLE IF NOT EXISTS open_threads (
    id TEXT PRIMARY KEY,
    character_id TEXT NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    topic TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'open',
    opened_at INTEGER NOT NULL,
    last_mentioned_at INTEGER NOT NULL,
    resolved_at INTEGER
);
CREATE INDEX IF NOT EXISTS idx_open_threads_char_status
    ON open_threads(character_id, status, last_mentioned_at DESC);
"#;

// V11: Letta-style multi-block memory. Each row is a labelled block
// of plain-text working memory the character maintains across turns —
// `persona` (her own self-image / current state of mind), `user`
// (compressed user-facing notes, separate from the digest), `session`
// (what's on her mind right now), `scratch` (free-form). The memory
// tool's `[[BLOCK set|append|clear ...]]` markers operate on these.
// Each block has a soft char cap (`capacity`) the system prompt
// surfaces so the model knows how much room is left — when full it
// must compact its own block before adding more. Default rows are
// inserted on first character creation by the chat pipeline.
pub const V11: &str = r#"
CREATE TABLE IF NOT EXISTS memory_blocks (
    id TEXT PRIMARY KEY,
    character_id TEXT NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    label TEXT NOT NULL,
    content TEXT NOT NULL DEFAULT '',
    capacity INTEGER NOT NULL DEFAULT 1000,
    description TEXT,
    updated_at INTEGER NOT NULL,
    UNIQUE(character_id, label)
);
CREATE INDEX IF NOT EXISTS idx_memory_blocks_char
    ON memory_blocks(character_id);
"#;

// V12: SillyTavern-inspired lorebook. Per-character pool of optional
// backstory snippets the user pre-writes; each entry has a list of
// trigger keywords and only injects into the system prompt when the
// user's current message (or recent chat tail) mentions one of them.
// Lets the user pour 5,000 words of lore into a character without
// paying the token cost every turn — only the entries whose keywords
// are live this turn show up.
//
// Columns:
// - `keywords_json` — JSON array of lowercase strings to substring-
//   match against the user message. Empty array = always active
//   (degenerate case, mirrors `backstory`).
// - `content` — the lore snippet text the model sees when triggered.
// - `priority` — sort key when multiple entries fire (higher = first).
// - `enabled` — soft-disable without deleting the row.
pub const V12: &str = r#"
CREATE TABLE IF NOT EXISTS lorebook_entries (
    id TEXT PRIMARY KEY,
    character_id TEXT NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    title TEXT NOT NULL DEFAULT '',
    keywords_json TEXT NOT NULL DEFAULT '[]',
    content TEXT NOT NULL,
    priority INTEGER NOT NULL DEFAULT 0,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_lorebook_char
    ON lorebook_entries(character_id, enabled, priority DESC);
"#;

// V13: Letta-style sleep-time anticipations. A background task runs
// after long idle gaps and asks a fast model "looking at the recent
// chat + the open threads, what would she naturally bring up next
// time?" The 1-3 short anticipations land here, and the spontaneous-
// topic trigger picks the freshest one over a bare open-thread topic.
// Single-row-per-character isn't enforced (we might keep a small
// rolling history); `consumed_at` flips when the trigger fires the
// entry so we don't repeat it.
pub const V13: &str = r#"
CREATE TABLE IF NOT EXISTS anticipations (
    id TEXT PRIMARY KEY,
    character_id TEXT NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    topic TEXT NOT NULL,
    rationale TEXT,
    created_at INTEGER NOT NULL,
    consumed_at INTEGER
);
CREATE INDEX IF NOT EXISTS idx_anticipations_char_fresh
    ON anticipations(character_id, consumed_at, created_at DESC);
"#;

// V14: behavior_history + surprisal_stats (Hermes-deep learning loop).
//
// `behavior_history` — rolling snapshots of the BehaviorProfile, one
//   row per re-extraction batch. Lets the daily drift pass diff against
//   prior snapshots ("user used to write short messages, now writes
//   longer ones") and lets the frontend chart style evolution. Pruned
//   to ~52 rows per character (weekly cadence × 1 year horizon) by the
//   background daily pass.
//
// `surprisal_stats` — per-character rolling mean of top-recall cosine
//   used by the adaptive surprisal gate. Replaces the static 0.78
//   threshold so the KG/behavior extractor fires only when the user
//   says something genuinely novel for THIS user — fresh users see
//   everything as novel under a static threshold, heavy-history users
//   see nothing as novel, both wrong.
pub const V14: &str = r#"
CREATE TABLE IF NOT EXISTS behavior_history (
    id TEXT PRIMARY KEY,
    character_id TEXT NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    snapshot_at INTEGER NOT NULL,
    message_length TEXT,
    emoji_use TEXT,
    language_register TEXT,
    tone_trend TEXT,
    vocabulary_note TEXT,
    total_messages_seen INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_behavior_history_char_time
    ON behavior_history(character_id, snapshot_at DESC);

CREATE TABLE IF NOT EXISTS surprisal_stats (
    character_id TEXT PRIMARY KEY REFERENCES characters(id) ON DELETE CASCADE,
    mean_top_cosine REAL NOT NULL DEFAULT 0.50,
    samples_seen INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL
);
"#;

pub const V3: &str = r#"
CREATE TABLE IF NOT EXISTS relationship (
    character_id TEXT PRIMARY KEY REFERENCES characters(id) ON DELETE CASCADE,
    xp INTEGER NOT NULL DEFAULT 0,
    level INTEGER NOT NULL DEFAULT 1,
    total_messages INTEGER NOT NULL DEFAULT 0,
    total_voice_seconds INTEGER NOT NULL DEFAULT 0,
    love_taps INTEGER NOT NULL DEFAULT 0,
    hard_slaps INTEGER NOT NULL DEFAULT 0,
    started_at INTEGER NOT NULL,
    last_interaction_at INTEGER NOT NULL,
    last_level_up_at INTEGER
);

CREATE TABLE IF NOT EXISTS relationship_milestones (
    id TEXT PRIMARY KEY,
    character_id TEXT NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    detail TEXT,
    happened_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_milestones_char_time
    ON relationship_milestones(character_id, happened_at DESC);

CREATE TABLE IF NOT EXISTS proactive_log (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    payload_json TEXT,
    fired_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_proactive_time
    ON proactive_log(fired_at DESC);
"#;

pub const V2: &str = r#"
CREATE TABLE IF NOT EXISTS memories (
    id TEXT PRIMARY KEY,
    character_id TEXT NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    content TEXT NOT NULL,
    importance REAL NOT NULL DEFAULT 0.5,
    emotional_weight REAL NOT NULL DEFAULT 0.0,
    embedding BLOB,
    created_at INTEGER NOT NULL,
    last_referenced_at INTEGER,
    ref_count INTEGER NOT NULL DEFAULT 0,
    user_confirmed INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_memories_char_importance
    ON memories(character_id, importance DESC);

CREATE TABLE IF NOT EXISTS entities (
    id TEXT PRIMARY KEY,
    character_id TEXT NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    attributes_json TEXT NOT NULL DEFAULT '{}',
    embedding BLOB,
    confidence REAL NOT NULL DEFAULT 0.5,
    ref_count INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    UNIQUE(character_id, kind, name)
);
CREATE INDEX IF NOT EXISTS idx_entities_char_kind
    ON entities(character_id, kind);

CREATE TABLE IF NOT EXISTS relations (
    id TEXT PRIMARY KEY,
    from_entity_id TEXT NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
    to_entity_id TEXT NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
    relation_kind TEXT NOT NULL,
    strength REAL NOT NULL DEFAULT 0.5,
    valence REAL NOT NULL DEFAULT 0.0,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_relations_from ON relations(from_entity_id);
CREATE INDEX IF NOT EXISTS idx_relations_to ON relations(to_entity_id);

CREATE TABLE IF NOT EXISTS feedback_events (
    id TEXT PRIMARY KEY,
    message_id TEXT NOT NULL REFERENCES chat_messages(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    reasons_json TEXT,
    comment TEXT,
    created_at INTEGER NOT NULL,
    consolidated INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_feedback_unconsolidated
    ON feedback_events(consolidated, created_at);
"#;

pub const V1: &str = r#"
CREATE TABLE IF NOT EXISTS app_meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS characters (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    kind TEXT NOT NULL DEFAULT 'narrator',
    created_at INTEGER NOT NULL,
    last_active_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS chat_messages (
    id TEXT PRIMARY KEY,
    character_id TEXT NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    source TEXT NOT NULL DEFAULT 'app',
    created_at INTEGER NOT NULL,
    feedback TEXT
);
CREATE INDEX IF NOT EXISTS idx_chat_messages_char_time
    ON chat_messages(character_id, created_at DESC);

CREATE TABLE IF NOT EXISTS mood_state (
    character_id TEXT PRIMARY KEY REFERENCES characters(id) ON DELETE CASCADE,
    happiness REAL NOT NULL DEFAULT 0.6,
    affection REAL NOT NULL DEFAULT 0.6,
    energy REAL NOT NULL DEFAULT 0.6,
    irritation REAL NOT NULL DEFAULT 0.0,
    excitement REAL NOT NULL DEFAULT 0.5,
    sadness REAL NOT NULL DEFAULT 0.0,
    updated_at INTEGER NOT NULL
);
"#;

// V16: per-message image reference. Selfies the character generates and
// photos the user attaches are saved as files under the app data dir
// (images/<uuid>.png); this column points at them. Nullable — text-only
// messages leave it NULL.
pub const V16: &str = r#"
ALTER TABLE chat_messages ADD COLUMN image_path TEXT;
"#;

// V15: nomic-embed-text → embeddinggemma migration. The two models
// produce vectors in incompatible spaces, so every stored embedding is
// nulled here — leaving stale nomic vectors in place would mean cosine
// recall compares EmbeddingGemma query vectors against nomic document
// vectors, which is meaningless. The one-time background re-embed pass
// (`background::reembed`) recomputes them with the new model once it's
// pulled; recall degrades gracefully (NULL embeddings are skipped) and
// self-heals as the pass progresses. Also migrates the saved
// embed-model preference so the chat pipeline and auto-installer agree
// on the new default. Empty tables (fresh install) make this a no-op.
pub const V15: &str = r#"
UPDATE memories SET embedding = NULL WHERE embedding IS NOT NULL;
UPDATE entities SET embedding = NULL WHERE embedding IS NOT NULL;
UPDATE app_meta SET value = 'embeddinggemma'
    WHERE key = 'user_embed_model' AND value = 'nomic-embed-text';
"#;

// V17: multi-chat. A "conversation" partitions message history for one
// character; the memory / mood / relationship engine stays character-
// scoped (the LLM never sees a raw history tail — see chat_pipeline.rs).
// `title` starts '' and the UI renders "New chat" until the background
// auto-titler fills it. `updated_at` is the sidebar sort key. Backfill
// gives every existing character one "Chat" conversation and adopts all
// its prior messages (exactly one conversation per character at this
// point, so the correlated UPDATE is unambiguous). `created_at` is unix
// seconds elsewhere in the schema, so strftime('%s','now') matches.
//
// The schema-shape half (CREATE TABLE / ALTER TABLE / CREATE INDEX) is
// a one-shot operation — SQLite has no `ADD COLUMN IF NOT EXISTS`, so
// it must never run twice against the same connection. The backfill
// half (`V17_BACKFILL`) is split out separately because it's naturally
// idempotent (INSERT only fires for characters with zero existing
// conversations; UPDATE only touches messages still missing a
// `conversation_id`) and safe to re-run — e.g. by a test seeding rows
// after the initial migration pass, or by a future repair tool.
// `V17` (the registered migration body) is both halves concatenated;
// `run()` never re-executes an applied migration body anyway
// (bookkeeping in `schema_migrations` short-circuits it).
#[cfg(test)]
pub const V17_BACKFILL: &str = r#"
INSERT INTO conversations (id, character_id, title, created_at, updated_at)
SELECT lower(hex(randomblob(16))), c.id, 'Chat',
       strftime('%s','now'), strftime('%s','now')
FROM characters c
WHERE NOT EXISTS (
    SELECT 1 FROM conversations conv WHERE conv.character_id = c.id
);

UPDATE chat_messages
SET conversation_id = (
    SELECT c.id FROM conversations c
    WHERE c.character_id = chat_messages.character_id
)
WHERE conversation_id IS NULL;
"#;

pub const V17: &str = concat!(
    r#"
CREATE TABLE IF NOT EXISTS conversations (
    id TEXT PRIMARY KEY,
    character_id TEXT NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    title TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_conversations_char
    ON conversations(character_id, updated_at DESC);

ALTER TABLE chat_messages ADD COLUMN conversation_id TEXT
    REFERENCES conversations(id) ON DELETE CASCADE;
CREATE INDEX IF NOT EXISTS idx_chat_messages_conv_time
    ON chat_messages(conversation_id, created_at DESC);
"#,
    r#"
INSERT INTO conversations (id, character_id, title, created_at, updated_at)
SELECT lower(hex(randomblob(16))), c.id, 'Chat',
       strftime('%s','now'), strftime('%s','now')
FROM characters c
WHERE NOT EXISTS (
    SELECT 1 FROM conversations conv WHERE conv.character_id = c.id
);

UPDATE chat_messages
SET conversation_id = (
    SELECT c.id FROM conversations c
    WHERE c.character_id = chat_messages.character_id
)
WHERE conversation_id IS NULL;
"#,
);

/// V19: characters for FPV worlds — player, NPC, and narrator definitions
/// with backstory, traits, and avatar support.
pub const V19: &str = r#"
CREATE TABLE IF NOT EXISTS fpv_characters (
    id TEXT PRIMARY KEY,
    world_id TEXT NOT NULL REFERENCES worlds(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    role TEXT NOT NULL DEFAULT 'npc' CHECK (role IN ('player', 'npc', 'narrator')),
    backstory TEXT,
    traits TEXT NOT NULL DEFAULT '[]',
    avatar_path TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_fpv_characters_world ON fpv_characters(world_id);
"#;

/// FPV narrative data model — worlds (story settings), codex (lore),
/// sessions, and messages. All local SQLite, no Supabase dependency.
pub const V18: &str = r#"
CREATE TABLE IF NOT EXISTS worlds (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    genre TEXT NOT NULL,
    description TEXT,
    system_prompt TEXT NOT NULL,
    accent_color TEXT,
    is_nsfw INTEGER NOT NULL DEFAULT 0,
    cover_image_path TEXT,
    source TEXT NOT NULL CHECK (source IN ('seed','user')),
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS codex_entries (
    id TEXT PRIMARY KEY,
    world_id TEXT NOT NULL REFERENCES worlds(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    content TEXT NOT NULL,
    triggers TEXT NOT NULL DEFAULT '[]'
);

CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    world_id TEXT NOT NULL REFERENCES worlds(id) ON DELETE CASCADE,
    summary TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS messages (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('narrator','user')),
    content TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_codex_world ON codex_entries(world_id);
CREATE INDEX IF NOT EXISTS idx_sessions_world ON sessions(world_id);
CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id);
"#;

/// V20: structured per-session state for continuity. The JSON payload stays
/// flexible while the session id and update timestamp remain queryable.
pub const V20: &str = r#"
CREATE TABLE IF NOT EXISTS fpv_story_state (
    session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
    state_json TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
"#;

/// V21: lightweight save points. A branch is a full local copy of a session,
/// so the original timeline remains immutable and can be revisited.
pub const V21: &str = r#"
ALTER TABLE sessions ADD COLUMN parent_session_id TEXT REFERENCES sessions(id) ON DELETE SET NULL;
ALTER TABLE sessions ADD COLUMN branch_label TEXT NOT NULL DEFAULT '';
CREATE INDEX IF NOT EXISTS idx_sessions_parent ON sessions(parent_session_id);
"#;

/// V22: stable per-session ordering independent of timestamp precision.
pub const V22: &str = r#"
ALTER TABLE messages ADD COLUMN sequence INTEGER NOT NULL DEFAULT 0;
UPDATE messages
SET sequence = (
    SELECT COUNT(*) FROM messages older
    WHERE older.session_id = messages.session_id
      AND (older.created_at < messages.created_at
           OR (older.created_at = messages.created_at AND older.id <= messages.id))
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_messages_session_sequence
    ON messages(session_id, sequence);
"#;

/// V23: semantic recall index for FPV story sessions. Embeddings are kept
/// separate from messages so the narrative history remains human-readable and
/// can be rebuilt when the embedding model changes.
pub const V23: &str = r#"
CREATE TABLE IF NOT EXISTS story_memory_records (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    message_id TEXT NOT NULL,
    content TEXT NOT NULL,
    embedding BLOB NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(session_id, message_id)
);
CREATE INDEX IF NOT EXISTS idx_story_memory_session
    ON story_memory_records(session_id, created_at DESC);
"#;

/// V24: lexical shortlist for hybrid semantic recall. Triggers keep the FTS
/// index synchronized with the authoritative embedding records.
pub const V24: &str = r#"
CREATE VIRTUAL TABLE IF NOT EXISTS story_memory_fts USING fts5(
    message_id UNINDEXED,
    session_id UNINDEXED,
    content,
    tokenize = 'unicode61'
);
INSERT INTO story_memory_fts(message_id, session_id, content)
SELECT message_id, session_id, content FROM story_memory_records;
CREATE TRIGGER IF NOT EXISTS story_memory_fts_insert AFTER INSERT ON story_memory_records BEGIN
    INSERT INTO story_memory_fts(message_id, session_id, content)
    VALUES (new.message_id, new.session_id, new.content);
END;
CREATE TRIGGER IF NOT EXISTS story_memory_fts_update AFTER UPDATE ON story_memory_records BEGIN
    DELETE FROM story_memory_fts WHERE message_id = old.message_id AND session_id = old.session_id;
    INSERT INTO story_memory_fts(message_id, session_id, content)
    VALUES (new.message_id, new.session_id, new.content);
END;
CREATE TRIGGER IF NOT EXISTS story_memory_fts_delete AFTER DELETE ON story_memory_records BEGIN
    DELETE FROM story_memory_fts WHERE message_id = old.message_id AND session_id = old.session_id;
END;
"#;

/// V25: named, durable story snapshots and per-session facts explicitly
/// pinned by the player. Both are copied with a branch so timelines remain
/// isolated after they diverge.
pub const V25: &str = r#"
ALTER TABLE sessions ADD COLUMN is_checkpoint INTEGER NOT NULL DEFAULT 0;
CREATE TABLE IF NOT EXISTS story_pinned_canon (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    content TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_story_pinned_canon_session
    ON story_pinned_canon(session_id, created_at ASC);
"#;

/// V26: per-world visual continuity instructions used by scene and cover
/// generation. Kept separate from world prose so image direction can evolve
/// without rewriting the narrative prompt.
pub const V26: &str = r#"
CREATE TABLE IF NOT EXISTS world_visual_bibles (
    world_id TEXT PRIMARY KEY REFERENCES worlds(id) ON DELETE CASCADE,
    style TEXT NOT NULL DEFAULT '',
    palette TEXT NOT NULL DEFAULT '',
    character_anchors TEXT NOT NULL DEFAULT '',
    location_anchors TEXT NOT NULL DEFAULT '',
    negative_prompt TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
"#;

/// V27: explicit per-story privacy policy for narrator routing.
pub const V27: &str = r#"
ALTER TABLE worlds ADD COLUMN privacy_mode TEXT NOT NULL DEFAULT 'local_only'
    CHECK (privacy_mode IN ('local_only', 'cloud_allowed'));
"#;

/// V28: durable cloud spend reservations. Unlike the rotating metrics file,
/// this ledger survives resets, restarts, and ambiguous cancelled requests.
pub const V28: &str = r#"
CREATE TABLE IF NOT EXISTS cloud_daily_spend_ledger (
    request_id TEXT PRIMARY KEY,
    day_start INTEGER NOT NULL,
    reserved_microusd INTEGER NOT NULL CHECK (reserved_microusd >= 0),
    charged_microusd INTEGER NOT NULL CHECK (charged_microusd >= 0),
    status TEXT NOT NULL CHECK (status IN ('reserved', 'completed', 'cancelled', 'failed')),
    created_at INTEGER NOT NULL,
    settled_at INTEGER
);
CREATE INDEX IF NOT EXISTS idx_cloud_daily_spend_ledger_day
    ON cloud_daily_spend_ledger(day_start, status);
"#;

/// V29: per-turn story-state history. `fpv_story_state` is a single
/// overwrite-per-session row with no history, so "undo last turn" could
/// never restore the state from before the undone turn. Every
/// `set_story_state` snapshots the previous state here first; `session_undo`
/// pops the most recent snapshot.
pub const V29: &str = r#"
CREATE TABLE IF NOT EXISTS fpv_story_state_history (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    state_json TEXT NOT NULL,
    saved_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_story_state_history_session
    ON fpv_story_state_history(session_id, saved_at);
"#;

/// V30: drop the Local Waifu tables FPV never used.
///
/// FPV was forked from Local Waifu, and migrations v1–v17 are LW's — they
/// build a companion-AI schema (characters, chat, the memory knowledge
/// graph, relationship state, lorebook, anticipations) before FPV's own
/// schema starts at v18. Not one of these seventeen tables is read or
/// written anywhere outside `schema.rs` and `migrations.rs`; every fresh
/// install created them empty and carried them forever.
///
/// The confusing part was `characters`: FPV has its own `fpv_characters`
/// (v19) and `db::characters` writes exclusively to that one, so the DB
/// held two character tables of which only the prefixed one was live.
/// Same story with `open_threads` — the TABLE is LW's and dead, while
/// `StoryState::open_threads` in `story/db.rs` is a live JSON field with
/// no relation to it.
///
/// v1–v17 still run on a fresh database (rewriting applied migrations is
/// how you corrupt somebody's install); this drops the result immediately
/// afterwards, so new and existing databases converge on the same shape.
///
/// Order matters: `foreign_keys` is ON, so each table is dropped before
/// the ones it references — everything pointing at `chat_messages`,
/// `conversations` and `entities` goes first, and `characters`, which all
/// of them ultimately hang off, goes last.
pub const V30: &str = r#"
DROP TABLE IF EXISTS relations;
DROP TABLE IF EXISTS entities;
DROP TABLE IF EXISTS feedback_events;
DROP TABLE IF EXISTS memories;
DROP TABLE IF EXISTS memory_blocks;
DROP TABLE IF EXISTS anticipations;
DROP TABLE IF EXISTS behavior_history;
DROP TABLE IF EXISTS lorebook_entries;
DROP TABLE IF EXISTS mood_state;
DROP TABLE IF EXISTS open_threads;
DROP TABLE IF EXISTS proactive_log;
DROP TABLE IF EXISTS relationship_milestones;
DROP TABLE IF EXISTS relationship;
DROP TABLE IF EXISTS surprisal_stats;
DROP TABLE IF EXISTS chat_messages;
DROP TABLE IF EXISTS conversations;
DROP TABLE IF EXISTS characters;
"#;
