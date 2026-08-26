use std::time::Duration;

use crate::error::{AppError, AppResult};
use serde::{Deserialize, Serialize};

const DEFAULT_BASE_URL: &str = "http://127.0.0.1:11434";

#[derive(Clone)]
pub struct OllamaClient {
    base_url: String,
    http: reqwest::Client,
}

impl OllamaClient {
    pub fn new() -> crate::error::AppResult<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|e| crate::error::AppError::Other(format!("build reqwest client: {e}")))?;
        Ok(Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            http,
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub async fn ping(&self) -> AppResult<bool> {
        let url = format!("{}/api/tags", self.base_url);
        let resp = self
            .http
            .get(&url)
            .timeout(Duration::from_millis(800))
            .send()
            .await;
        Ok(matches!(resp, Ok(r) if r.status().is_success()))
    }

    pub async fn list_models(&self) -> AppResult<Vec<TagsModel>> {
        let url = format!("{}/api/tags", self.base_url);
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let body: TagsResponse = resp.json().await?;
        Ok(body.models)
    }

    pub async fn chat(&self, req: ChatRequest) -> AppResult<ChatResponse> {
        // Admission control: a local render holds several GB and so does
        // the model this call is about to make resident. See
        // `crate::resource_gate`.
        let _slot = crate::resource_gate::narration_slot().await;
        let url = format!("{}/api/chat", self.base_url);
        let resp = self
            .http
            .post(&url)
            .json(&req)
            .timeout(std::time::Duration::from_secs(600))
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AppError::Ollama(format!("{status}: {text}")));
        }
        let body: ChatResponse = resp.json().await?;
        Ok(body)
    }

    /// Ask Ollama to LOAD a model without generating anything:
    /// `/api/generate` with an empty prompt returns once the weights are
    /// resident (documented Ollama "load" semantics); the server-side
    /// keep_alive then applies. Used by the boot pre-warm so the first
    /// chat message doesn't pay the cold weight load.
    ///
    /// `num_ctx` MUST match what the chat path will send (the user's
    /// contextSize setting): Ollama treats context length as part of the
    /// runner config, so a pre-warm at the default ctx followed by a chat
    /// at 32K would tear the model down and reload it — making the
    /// pre-warm worse than useless.
    pub async fn load_model(&self, model: &str, num_ctx: Option<i64>) -> AppResult<()> {
        let url = format!("{}/api/generate", self.base_url);
        let mut body = serde_json::json!({ "model": model, "prompt": "" });
        if let Some(n) = num_ctx {
            body["options"] = serde_json::json!({ "num_ctx": n });
        }
        // Big checkpoints take tens of seconds to page in (18 GB ≈ 20 s
        // measured) — go well past the client's 120 s default.
        let resp = self
            .http
            .post(&url)
            .json(&body)
            .timeout(Duration::from_secs(300))
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AppError::Ollama(format!("load {status}: {text}")));
        }
        Ok(())
    }

    /// Unload model runners before terminating the bundled daemon. Without
    /// this, Ollama's llama-server children can outlive their parent process.
    pub async fn unload_all(&self) -> AppResult<()> {
        let url = format!("{}/api/ps", self.base_url);
        let response = self
            .http
            .get(&url)
            .timeout(Duration::from_secs(2))
            .send()
            .await?
            .error_for_status()?;
        let running: RunningModelsResponse = response.json().await?;

        for model in running.models {
            let url = format!("{}/api/generate", self.base_url);
            self.http
                .post(&url)
                .json(&serde_json::json!({
                    "model": model.name,
                    "prompt": "",
                    "keep_alive": 0,
                    "stream": false
                }))
                .timeout(Duration::from_secs(10))
                .send()
                .await?
                .error_for_status()?;
        }
        Ok(())
    }

    /// Chat with `stream: true`. Each NDJSON chunk is parsed and the
    /// content delta passed to `on_token`. Returns the accumulated full
    /// text + counters from the final `done` chunk.
    pub async fn chat_stream<F>(
        &self,
        req: ChatRequest,
        mut on_token: F,
    ) -> AppResult<ChatStreamSummary>
    where
        F: FnMut(&str) + Send,
    {
        use futures_util::StreamExt;
        // Admission control: a local render holds several GB and so does
        // the model this call is about to make resident. See
        // `crate::resource_gate`.
        let _slot = crate::resource_gate::narration_slot().await;
        let url = format!("{}/api/chat", self.base_url);
        let body = stream_body(&req);
        // Long generations on big models can run past the client's 120 s
        // default. Override per-request — the stream itself signals end.
        let resp = self
            .http
            .post(&url)
            .json(&body)
            .timeout(Duration::from_secs(10 * 60))
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AppError::Ollama(format!("stream {status}: {text}")));
        }

        let mut stream = resp.bytes_stream();
        let mut buf = Vec::new();
        let mut full = String::new();
        let mut model_returned = String::new();
        let mut eval_count: u64 = 0;
        let mut total_duration: u64 = 0;
        let mut saw_done = false;

        // Hard ceiling on the unparsed NDJSON buffer. A misbehaving
        // proxy / model that sends a giant chunk without newlines
        // would otherwise grow `buf` to the size of the entire
        // response before draining — potential OOM on a 27B model
        // streaming 50K tokens. 4 MiB is comfortable for any
        // legitimate single NDJSON line (typical < 4 KB).
        const MAX_BUF_BYTES: usize = 4 * 1024 * 1024;

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            buf.extend_from_slice(&chunk);
            if buf.len() > MAX_BUF_BYTES {
                return Err(AppError::Ollama(format!(
                    "stream: buffer exceeded {MAX_BUF_BYTES} bytes without newline (proxy/server issue)"
                )));
            }
            while let Some(idx) = buf.iter().position(|b| *b == b'\n') {
                let line = buf.drain(..=idx).collect::<Vec<u8>>();
                let line = &line[..line.len().saturating_sub(1)];
                if line.is_empty() {
                    continue;
                }
                apply_chat_line(
                    line,
                    &mut on_token,
                    &mut full,
                    &mut model_returned,
                    &mut saw_done,
                    &mut eval_count,
                    &mut total_duration,
                )?;
            }
        }

        if !buf.is_empty() {
            apply_chat_line(
                &buf,
                &mut on_token,
                &mut full,
                &mut model_returned,
                &mut saw_done,
                &mut eval_count,
                &mut total_duration,
            )?;
        }

        if !saw_done {
            return Err(AppError::Ollama(
                "stream ended before Ollama sent a done chunk".into(),
            ));
        }

        Ok(ChatStreamSummary {
            text: full,
            model: model_returned,
            eval_count,
            total_duration,
        })
    }

    /// POST /api/pull — streams progress lines (`{"status": "...", "completed": .., "total": ..}`)
    /// to the supplied callback. Returns when the stream closes.
    ///
    /// Override the client's default 120 s timeout: a 5 GB model on a
    /// typical home connection takes 10–20 min to download, well past
    /// the default. We give the whole pull an hour and let the stream
    /// itself decide when it's done.
    pub async fn pull<F>(&self, model: &str, mut on_progress: F) -> AppResult<()>
    where
        F: FnMut(PullProgress) + Send,
    {
        use futures_util::StreamExt;
        let url = format!("{}/api/pull", self.base_url);
        let body = serde_json::json!({ "name": model, "stream": true });
        let resp = self
            .http
            .post(&url)
            .json(&body)
            .timeout(Duration::from_secs(60 * 60))
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AppError::Ollama(format!("pull {status}: {text}")));
        }
        let mut stream = resp.bytes_stream();
        let mut buf = Vec::new();
        const MAX_PULL_LINE_BYTES: usize = 4 * 1024 * 1024;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            buf.extend_from_slice(&chunk);
            if buf.len() > MAX_PULL_LINE_BYTES {
                return Err(AppError::Ollama(
                    "pull NDJSON line exceeded size limit".into(),
                ));
            }
            while let Some(idx) = buf.iter().position(|b| *b == b'\n') {
                let line = buf.drain(..=idx).collect::<Vec<u8>>();
                let line = &line[..line.len().saturating_sub(1)];
                if line.is_empty() {
                    continue;
                }
                match parse_pull_line(line) {
                    PullLine::Progress(pp) => on_progress(pp),
                    // Ollama reports mid-pull failures (missing manifest,
                    // disk full, network drop, engine too old) as an
                    // `{"error": "..."}` NDJSON line — the HTTP status is
                    // already 200 by then. Without this arm the error line
                    // deserialized as nothing, the stream ended, and pull()
                    // returned Ok(()) — the UI showed a successful download
                    // that never installed anything.
                    PullLine::Error(msg) => {
                        return Err(AppError::Ollama(format!("pull failed: {msg}")))
                    }
                    PullLine::Other => {}
                }
            }
        }
        if !buf.is_empty() {
            match parse_pull_line(&buf) {
                PullLine::Progress(pp) => on_progress(pp),
                PullLine::Error(msg) => {
                    return Err(AppError::Ollama(format!("pull failed: {msg}")))
                }
                PullLine::Other => {
                    return Err(AppError::Ollama("pull returned malformed NDJSON".into()))
                }
            }
        }
        Ok(())
    }

    /// DELETE /api/delete — remove a pulled model.
    pub async fn delete_model(&self, model: &str) -> AppResult<()> {
        let url = format!("{}/api/delete", self.base_url);
        let resp = self
            .http
            .delete(&url)
            .json(&serde_json::json!({ "name": model }))
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AppError::Ollama(format!("delete {status}: {text}")));
        }
        Ok(())
    }

    /// POST /api/embeddings — returns a single embedding vector for `prompt`.
    /// Failure modes (model not pulled, network, etc.) are surfaced verbatim
    /// so callers can decide whether to skip memory recall or hard-fail.
    ///
    /// Per hard rule 19: the client default timeout is 120 s, but
    /// embedding can exceed that on a cold Ollama startup when the
    /// embed model is being loaded into GPU RAM for the first
    /// time. A timed-out embed makes `user_embedding = None` in the
    /// chat pipeline, which silently disables recall, surprisal
    /// recording, and memory persistence for that turn — the user
    /// sees a reply with no memory context and no error. Use the same
    /// 600 s per-request override that `chat()` and `chat_stream()`
    /// already apply.
    pub async fn embed(&self, model: &str, prompt: &str) -> AppResult<Vec<f32>> {
        // Admission control: a local render holds several GB and so does
        // the model this call is about to make resident. See
        // `crate::resource_gate`.
        let _slot = crate::resource_gate::narration_slot().await;
        let url = format!("{}/api/embeddings", self.base_url);
        let body = serde_json::json!({ "model": model, "prompt": prompt });
        let resp = self
            .http
            .post(&url)
            .json(&body)
            .timeout(std::time::Duration::from_secs(600))
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AppError::Ollama(format!("embed {status}: {text}")));
        }
        let parsed: EmbedResponse = resp.json().await?;
        Ok(parsed.embedding)
    }

    /// Embed a search query (the user's current message). For
    /// EmbeddingGemma this applies the official query prefix; for other
    /// models the text is embedded raw. See [`format_embed_input`].
    pub async fn embed_query(&self, model: &str, text: &str) -> AppResult<Vec<f32>> {
        self.embed(model, &format_embed_input(model, EmbedTask::Query, text))
            .await
    }

    /// Embed a stored document (memory content, session summary,
    /// self-insight, consolidated entry). For EmbeddingGemma this
    /// applies the official document prefix; for other models the text
    /// is embedded raw. See [`format_embed_input`].
    pub async fn embed_document(&self, model: &str, text: &str) -> AppResult<Vec<f32>> {
        self.embed(model, &format_embed_input(model, EmbedTask::Document, text))
            .await
    }
}

fn apply_chat_line<F>(
    line: &[u8],
    on_token: &mut F,
    full: &mut String,
    model_returned: &mut String,
    saw_done: &mut bool,
    eval_count: &mut u64,
    total_duration: &mut u64,
) -> AppResult<()>
where
    F: FnMut(&str),
{
    let value: serde_json::Value = serde_json::from_slice(line)
        .map_err(|e| AppError::Ollama(format!("stream returned malformed NDJSON: {e}")))?;
    let object = value
        .as_object()
        .ok_or_else(|| AppError::Ollama("stream returned a non-object NDJSON chunk".into()))?;
    if !["model", "message", "done", "total_duration", "eval_count"]
        .iter()
        .any(|key| object.contains_key(*key))
    {
        return Err(AppError::Ollama(
            "stream returned an unknown NDJSON chunk".into(),
        ));
    }
    let part: ChatResponse = serde_json::from_value(value)
        .map_err(|e| AppError::Ollama(format!("stream returned invalid chat chunk: {e}")))?;
    if !part.model.is_empty() {
        *model_returned = part.model;
    }
    if let Some(msg) = part.message {
        if !msg.content.is_empty() {
            on_token(&msg.content);
            full.push_str(&msg.content);
        }
    }
    if part.done {
        *saw_done = true;
        *eval_count = part.eval_count;
        *total_duration = part.total_duration;
    }
    Ok(())
}

/// Parse the `capabilities` array from an `/api/show` JSON body. Recent
/// Ollama reports e.g. `{"capabilities":["completion","vision"], ...}`.
/// Returns an empty vec when the field is absent (older Ollama / older
/// model) or the body isn't valid JSON — the caller then falls back to
/// substring-based detection.
#[cfg(test)]
pub fn parse_show_capabilities(body: &str) -> Vec<String> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            v.get("capabilities").and_then(|c| c.as_array()).map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
        })
        .unwrap_or_default()
}

/// True when a capability list declares vision/multimodal support.
#[cfg(test)]
pub fn caps_include_vision(caps: &[String]) -> bool {
    caps.iter().any(|c| c.eq_ignore_ascii_case("vision"))
}

/// Embedding task type. EmbeddingGemma is trained with task-specific
/// prefixes; applying the right one materially improves retrieval
/// quality, and it's asymmetric — the search query and the stored
/// document use different prefixes. Models that don't use prefixes
/// (e.g. the legacy `nomic-embed-text`) receive the raw text unchanged,
/// so this is safe to apply unconditionally at every call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedTask {
    /// A search query — the user's current message driving recall.
    Query,
    /// A stored document — memory content, summary, insight, entity.
    Document,
}

/// Whether `model` is an EmbeddingGemma variant that wants task
/// prefixes. Matches the bare `embeddinggemma` tag, version/quant
/// variants (`embeddinggemma:300m`, …), and namespaced mirrors
/// (`dhiltgen/embeddinggemma`).
fn wants_gemma_prefix(model: &str) -> bool {
    let m = model.to_ascii_lowercase();
    m.starts_with("embeddinggemma") || m.contains("/embeddinggemma")
}

/// Format `text` for `model` + `task`. For EmbeddingGemma this prepends
/// the official prompt prefixes from Google's model card:
///   query    → `task: search result | query: {text}`
///   document → `title: none | text: {text}`
/// For any other model the text is returned unchanged.
pub fn format_embed_input(model: &str, task: EmbedTask, text: &str) -> String {
    if wants_gemma_prefix(model) {
        match task {
            EmbedTask::Query => format!("task: search result | query: {text}"),
            EmbedTask::Document => format!("title: none | text: {text}"),
        }
    } else {
        text.to_string()
    }
}

#[derive(Debug, Deserialize)]
struct EmbedResponse {
    embedding: Vec<f32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PullProgress {
    pub status: String,
    #[serde(default)]
    pub digest: Option<String>,
    #[serde(default)]
    pub total: Option<u64>,
    #[serde(default)]
    pub completed: Option<u64>,
}

/// One decoded NDJSON line of an `/api/pull` stream.
#[derive(Debug)]
enum PullLine {
    Progress(PullProgress),
    Error(String),
    Other,
}

/// Ollama's mid-stream error line: `{"error": "..."}` with no `status`
/// field, so it never deserializes as `PullProgress`.
#[derive(Debug, Deserialize)]
struct PullErrorLine {
    error: String,
}

fn parse_pull_line(line: &[u8]) -> PullLine {
    if let Ok(pp) = serde_json::from_slice::<PullProgress>(line) {
        return PullLine::Progress(pp);
    }
    if let Ok(e) = serde_json::from_slice::<PullErrorLine>(line) {
        return PullLine::Error(e.error);
    }
    PullLine::Other
}

impl Default for OllamaClient {
    fn default() -> Self {
        Self::new().expect("build OllamaClient")
    }
}

#[derive(Debug, Deserialize)]
pub struct TagsResponse {
    pub models: Vec<TagsModel>,
}

#[derive(Debug, Deserialize)]
struct RunningModelsResponse {
    #[serde(default)]
    models: Vec<RunningModel>,
}

#[derive(Debug, Deserialize)]
struct RunningModel {
    name: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TagsModel {
    pub name: String,
    #[serde(default)]
    pub size: u64,
}

/// Request body for the streaming chat call: the SAME `ChatRequest`
/// struct `chat()` serializes (so every field — notably `think` — reaches
/// the wire), with `stream` forced to `true`. Never hand-build this body
/// field-by-field: the v1.4.2 hand-built version silently dropped `think`,
/// and reasoning-by-default local models (gemma4) then burned minutes of
/// hidden thinking tokens on every streamed desktop turn while the UI
/// showed nothing (thinking deltas stream as `message.thinking`, which the
/// parser deliberately ignores).
fn stream_body(req: &ChatRequest) -> serde_json::Value {
    let mut v = serde_json::to_value(req).expect("ChatRequest is always serializable");
    v["stream"] = serde_json::Value::Bool(true);
    v
}

#[derive(Debug, Serialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<serde_json::Value>,
    /// Native reasoning ("thinking") control for capable models.
    /// `Some(false)` disables it (fastest, reply-only — avoids the hidden
    /// reasoning tokens that both slow generation and, under a length cap,
    /// starved the visible reply). `Some(true)` lets the model reason when it
    /// wants. `None` omits the field entirely (model default). Note: gemma4
    /// only accepts a boolean here — string effort levels ("low"/"high")
    /// error, so we never send them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub think: Option<bool>,
    /// Structured-output mode: `"json"`, or a JSON Schema object.
    ///
    /// **This has to be TOP-LEVEL.** Ollama reads `format` off the
    /// request root; anything under `options` is silently dropped —
    /// unknown option keys are not an error, they're just ignored.
    /// Every background extractor that "enabled JSON mode" by putting
    /// `"format": "json"` in its options blob (session summary, digest
    /// consolidation, profile drift, self-reflection) was therefore
    /// running in plain text mode with no sign of it. Proven live
    /// 2026-07-25 against gemma4:e4b: identical prompt, `format` in
    /// `options` → prose; `format` at the root → a JSON object.
    /// `background_chat` lifts the key out of the options blob so those
    /// call sites now get what they asked for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    /// Base64 image payloads (no `data:` prefix) for vision turns.
    /// Skipped from the wire when empty so text turns are unchanged.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub images: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct ChatResponse {
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub message: Option<ChatMessage>,
    #[serde(default)]
    pub done: bool,
    #[serde(default)]
    pub total_duration: u64,
    #[serde(default)]
    pub eval_count: u64,
}

#[derive(Debug, Clone)]
pub struct ChatStreamSummary {
    pub text: String,
    pub model: String,
    pub eval_count: u64,
    pub total_duration: u64,
}

#[cfg(test)]
mod pull_line_tests {
    use super::*;

    #[test]
    fn pull_error_line_is_detected_not_swallowed() {
        // Real shape Ollama emits when a pull fails mid-stream (HTTP is
        // already 200): the error MUST surface, not parse as a no-op.
        match parse_pull_line(br#"{"error":"pull model manifest: file does not exist"}"#) {
            PullLine::Error(msg) => {
                assert_eq!(msg, "pull model manifest: file does not exist")
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn pull_progress_line_still_parses() {
        match parse_pull_line(
            br#"{"status":"downloading","digest":"sha256:abc","total":10,"completed":5}"#,
        ) {
            PullLine::Progress(pp) => {
                assert_eq!(pp.status, "downloading");
                assert_eq!(pp.completed, Some(5));
            }
            other => panic!("expected Progress, got {other:?}"),
        }
    }

    #[test]
    fn pull_unknown_line_is_other() {
        assert!(matches!(parse_pull_line(b"not json"), PullLine::Other));
        assert!(matches!(
            parse_pull_line(br#"{"weird":1}"#),
            PullLine::Other
        ));
    }
}

#[cfg(test)]
mod chat_stream_line_tests {
    use super::*;

    #[test]
    fn malformed_ndjson_is_rejected() {
        let mut text = String::new();
        let mut model = String::new();
        let mut done = false;
        let mut eval = 0;
        let mut duration = 0;
        let result = apply_chat_line(
            br#"{"message":{"content":"unterminated"}"#,
            &mut |_| {},
            &mut text,
            &mut model,
            &mut done,
            &mut eval,
            &mut duration,
        );
        assert!(result.is_err());
    }

    #[test]
    fn final_done_line_is_accepted_without_newline() {
        let mut text = String::new();
        let mut model = String::new();
        let mut done = false;
        let mut eval = 0;
        let mut duration = 0;
        apply_chat_line(
            br#"{"model":"qwen","done":true,"eval_count":3,"total_duration":9}"#,
            &mut |_| {},
            &mut text,
            &mut model,
            &mut done,
            &mut eval,
            &mut duration,
        )
        .unwrap();
        assert!(done);
        assert_eq!(model, "qwen");
        assert_eq!(eval, 3);
    }
}

#[cfg(test)]
mod stream_body_tests {
    use super::*;

    fn req(think: Option<bool>) -> ChatRequest {
        ChatRequest {
            model: "gemma4:12b".into(),
            messages: vec![ChatMessage {
                role: "user".into(),
                content: "hi".into(),
                images: None,
            }],
            stream: false,
            options: Some(serde_json::json!({ "num_ctx": 16384 })),
            think,
            format: None,
        }
    }

    #[test]
    fn stream_body_forces_stream_true() {
        let b = stream_body(&req(Some(false)));
        assert_eq!(b["stream"], serde_json::json!(true));
    }

    #[test]
    fn stream_body_carries_think_when_set() {
        // The v1.4.2 regression: the hand-built stream body dropped
        // `think`, so reasoning-by-default models (gemma4) burned hidden
        // thinking tokens on every streamed desktop turn — measured 776
        // tokens / 63 s vs 69 tokens / 6 s for the same message. The
        // struct is the wire contract; the stream body must carry every
        // field `chat()` would send.
        let b = stream_body(&req(Some(false)));
        assert_eq!(b["think"], serde_json::json!(false));
        let b = stream_body(&req(Some(true)));
        assert_eq!(b["think"], serde_json::json!(true));
    }

    #[test]
    fn stream_body_omits_think_when_none() {
        let b = stream_body(&req(None));
        assert!(
            b.get("think").is_none(),
            "None must omit the field (model default)"
        );
    }

    #[test]
    fn stream_body_keeps_model_messages_options() {
        let b = stream_body(&req(Some(false)));
        assert_eq!(b["model"], serde_json::json!("gemma4:12b"));
        assert_eq!(b["messages"][0]["content"], serde_json::json!("hi"));
        assert_eq!(b["options"]["num_ctx"], serde_json::json!(16384));
    }
}

#[cfg(test)]
mod chat_message_tests {
    use super::*;

    #[test]
    fn chat_message_serializes_images_when_present() {
        let m = ChatMessage {
            role: "user".into(),
            content: "hi".into(),
            images: Some(vec!["BASE64".into()]),
        };
        let j = serde_json::to_string(&m).unwrap();
        assert!(j.contains("\"images\""));
        let m2 = ChatMessage {
            role: "user".into(),
            content: "hi".into(),
            images: None,
        };
        assert!(!serde_json::to_string(&m2).unwrap().contains("images"));
    }
}

#[cfg(test)]
mod embed_prefix_tests {
    use super::*;

    #[test]
    fn gemma_query_and_document_prefixes() {
        assert_eq!(
            format_embed_input("embeddinggemma", EmbedTask::Query, "kocham koty"),
            "task: search result | query: kocham koty"
        );
        assert_eq!(
            format_embed_input("embeddinggemma:300m", EmbedTask::Document, "kocham koty"),
            "title: none | text: kocham koty"
        );
        // Namespaced mirror still matches.
        assert_eq!(
            format_embed_input("dhiltgen/embeddinggemma", EmbedTask::Query, "x"),
            "task: search result | query: x"
        );
    }

    #[test]
    fn non_gemma_models_pass_text_through_unchanged() {
        assert_eq!(
            format_embed_input("nomic-embed-text", EmbedTask::Query, "hello"),
            "hello"
        );
        assert_eq!(
            format_embed_input("nomic-embed-text", EmbedTask::Document, "hello"),
            "hello"
        );
    }

    #[test]
    fn parses_capabilities_array_from_show_body() {
        let body = r#"{"license":"x","capabilities":["completion","vision"],"modelfile":"..."}"#;
        let caps = parse_show_capabilities(body);
        assert_eq!(caps, vec!["completion".to_string(), "vision".to_string()]);
        assert!(caps_include_vision(&caps));
    }

    #[test]
    fn missing_or_invalid_capabilities_is_empty_and_not_vision() {
        assert!(parse_show_capabilities(r#"{"license":"x"}"#).is_empty());
        assert!(parse_show_capabilities("not json").is_empty());
        assert!(!caps_include_vision(&parse_show_capabilities(
            r#"{"capabilities":["completion"]}"#
        )));
    }

    #[test]
    fn caps_vision_is_case_insensitive() {
        assert!(caps_include_vision(&["Vision".to_string()]));
        assert!(caps_include_vision(&[
            "completion".to_string(),
            "VISION".to_string()
        ]));
        assert!(!caps_include_vision(&["completion".to_string()]));
    }
}
