//! Minimal model catalog for FPV — model listing and management.
//! FPV model catalog — only local models. Cloud providers were removed.

use serde::Serialize;

/// Which role a catalog model plays. `embed` models power semantic memory
/// (always local via Ollama); `chat` models are the narrator choices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogKind {
    Chat,
    Embed,
}

#[derive(Debug, Clone, Serialize)]
pub struct CatalogEntry {
    pub id: &'static str,
    pub kind: CatalogKind,
    /// True for the recommended default in its role — currently only the
    /// `embeddinggemma` embed model. Chat recommendations are hardware-tier
    /// driven instead (`hardware_chat_default` on `ModelListResult`).
    pub recommended: bool,
    pub description: &'static str,
}

/// Embedding models (with `embeddinggemma` as the recommended default) and
/// the selectable chat models. Every embedding entry is OPTIONAL — the user
/// picks the active one in Settings → Narrative Model → Embedding; semantic
/// memory falls back to `embeddinggemma` only when no choice was made
/// (see `commands/memory.rs`).
///
/// The chat entries mirror `Tier::default_chat_model` in
/// `inference/hardware_tier.rs` — one per RAM tier, same tags, same sizes.
/// They must stay in sync: `hardware_chat_default` on `ModelListResult` is
/// resolved from the tier, and the UI matches it against these ids to mark
/// the recommended row. A tag present in one and missing from the other
/// shows the user a recommendation they cannot install.
///
/// Tags last verified 2026-07-25 against the Ollama registry (see the note
/// above `default_chat_model`); re-verify with a HEAD against the manifest,
/// never by reading a blog:
///
///   curl -s -o /dev/null -w "%{http_code}" \
///     https://registry.ollama.ai/v2/library/<name>/manifests/<tag>
///
/// Only the chat entries reach the onboarding picker — `ModelSetupStep`
/// filters on `kind`. Until chat entries existed the picker rendered an
/// empty list, because the embedding model was the sole entry.
pub static CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        id: "embeddinggemma",
        kind: CatalogKind::Embed,
        recommended: true,
        description: "Recommended · embedding model for semantic search",
    },
    CatalogEntry {
        id: "nomic-embed-text",
        kind: CatalogKind::Embed,
        recommended: false,
        description: "0.27 GB · classic general-purpose open embeddings",
    },
    CatalogEntry {
        id: "mxbai-embed-large",
        kind: CatalogKind::Embed,
        recommended: false,
        description: "0.67 GB · top MTEB open embedder",
    },
    CatalogEntry {
        id: "bge-m3",
        kind: CatalogKind::Embed,
        recommended: false,
        description: "1.2 GB · multilingual (100+ languages) — great for Polish",
    },
    CatalogEntry {
        id: "all-minilm:l6-v2",
        kind: CatalogKind::Embed,
        recommended: false,
        description: "0.09 GB · tiny and fast, CPU-friendly",
    },
    CatalogEntry {
        id: "gemma4:e2b",
        kind: CatalogKind::Chat,
        recommended: false,
        description: "7.2 GB · for Macs with up to 12 GB of memory",
    },
    CatalogEntry {
        id: "gemma4:e4b",
        kind: CatalogKind::Chat,
        recommended: false,
        description: "9.6 GB · for Macs with 13–18 GB of memory",
    },
    CatalogEntry {
        id: "qwen3.5:9b",
        kind: CatalogKind::Chat,
        recommended: false,
        description: "6.6 GB · for Macs with 19–34 GB of memory",
    },
    CatalogEntry {
        id: "qwen3.6:27b",
        kind: CatalogKind::Chat,
        recommended: false,
        description: "17 GB · for Macs with 35–63 GB of memory",
    },
    CatalogEntry {
        id: "qwen3.6:35b",
        kind: CatalogKind::Chat,
        recommended: false,
        description: "24 GB · for Macs with 64 GB of memory or more",
    },
];
