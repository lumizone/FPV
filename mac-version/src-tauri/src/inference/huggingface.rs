//! Hugging Face model browser — lets the user search GGUF chat models on
//! HuggingFace and install them through the bundled Ollama (`hf.co/{repo}:
//! {quant}`), exactly the flow Local Waifu shipped. Read-only against the
//! public HF API; no auth needed.

use crate::error::{AppError, AppResult};
use serde::{Deserialize, Serialize};

const HF_API: &str = "https://huggingface.co/api";

#[derive(Debug, Serialize, Deserialize)]
pub struct HfModel {
    pub id: String,
    #[serde(default)]
    pub downloads: i64,
    #[serde(default)]
    pub likes: i64,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct HfQuant {
    /// e.g. `Q4_K_M`
    pub quant: String,
    /// filename, e.g. `model-Q4_K_M.gguf`
    pub filename: String,
    /// size in bytes (from the tree API), 0 when unknown
    pub size: i64,
}

/// Search GGUF models on HF, ordered by downloads.
pub async fn search(query: &str) -> AppResult<Vec<HfModel>> {
    let client = reqwest::Client::new();
    let url = format!(
        "{HF_API}/models?search={}&filter=gguf&sort=downloads&direction=-1&limit=20",
        urlencode_query(query)
    );
    let resp = client.get(&url).send().await.map_err(AppError::Http)?;
    if !resp.status().is_success() {
        return Err(AppError::Other(format!(
            "HuggingFace search failed: {}",
            resp.status()
        )));
    }
    resp.json::<Vec<HfModel>>()
        .await
        .map_err(|e| AppError::Other(format!("HuggingFace parse: {e}")))
}

/// List the GGUF quants available for a repo (from the model file tree).
pub async fn quants(repo: &str) -> AppResult<Vec<HfQuant>> {
    let client = reqwest::Client::new();
    let url = format!(
        "{HF_API}/models/{}/tree/main?expand=false&recursive=true",
        urlencode_path(repo)
    );
    let resp = client.get(&url).send().await.map_err(AppError::Http)?;
    if !resp.status().is_success() {
        return Err(AppError::Other(format!(
            "HuggingFace tree failed for {repo}: {}",
            resp.status()
        )));
    }
    #[derive(Deserialize)]
    struct TreeEntry {
        #[serde(default)]
        path: String,
        #[serde(default)]
        size: i64,
        #[serde(default)]
        r#type: String,
    }
    let entries: Vec<TreeEntry> = resp
        .json()
        .await
        .map_err(|e| AppError::Other(format!("HuggingFace tree parse: {e}")))?;
    let mut quants: Vec<HfQuant> = entries
        .into_iter()
        .filter(|e| e.r#type == "file" && e.path.ends_with(".gguf"))
        .map(|e| {
            let quant = quant_from_filename(&e.path).to_string();
            HfQuant {
                quant,
                filename: e.path.clone(),
                size: e.size,
            }
        })
        .collect();
    // Sort big→small so the user sees the highest-quality option first.
    quants.sort_by_key(|quant| std::cmp::Reverse(quant.size));
    Ok(quants)
}

/// Derive a short quant label from a GGUF filename:
/// `model-Q4_K_M.gguf` → `Q4_K_M`, `qwen-7b-f16.gguf` → `f16`.
fn quant_from_filename(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    let stem = name.strip_suffix(".gguf").unwrap_or(name);
    // Common patterns: `-Q4_K_M.gguf`, `-q4_k_m.gguf`, `-f16.gguf`, `-fp16.gguf`
    for marker in ["-Q", "-q", "-F", "-f", "-I", "-i", "_Q", "_q"] {
        if let Some(idx) = stem.rfind(marker) {
            let candidate = &stem[idx + 1..];
            if !candidate.is_empty() && candidate.len() <= 20 {
                return candidate;
            }
        }
    }
    stem
}

/// Percent-encode a QUERY value (RFC 3986): everything except unreserved
/// characters is encoded — & = % + must not survive, or a crafted search
/// term could inject extra HF API parameters.
fn urlencode_query(s: &str) -> String {
    percent_encode(s, false)
}

/// Percent-encode a PATH segment: same as query, but '/' is kept literal —
/// the owner/repo separator in `/models/{owner}/{repo}/tree` must not be
/// encoded away.
fn urlencode_path(s: &str) -> String {
    percent_encode(s, true)
}

fn percent_encode(s: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b'/' if keep_slash => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
