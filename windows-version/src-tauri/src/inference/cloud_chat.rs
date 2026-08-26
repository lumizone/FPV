//! Minimal cloud LLM client for narration — Bring-Your-Own-Key.
//!
//! FPV is local-first; cloud models are an opt-in. This module talks to the
//! user's own provider with their own key (stored by `byok::save`), never a
//! hardcoded credential. Two wire shapes:
//!   - OpenAI-compatible `/chat/completions` — OpenAI, DeepSeek, OpenRouter,
//!     Mistral, Groq, xAI/Grok, and the generic `custom` endpoint all speak it.
//!   - Anthropic `/v1/messages`.
//!   - Google Gemini `generateContent`.

use crate::error::{AppError, AppResult};

/// Ceiling on a fully-buffered provider response.
///
/// A non-streaming completion is at most a few hundred KB, and an error
/// body gets truncated to 200 characters by `sanitize_error_body` anyway
/// — but `resp.text()` buffers the WHOLE body first, with no limit. That
/// matters most for `CloudProvider::Custom`, whose base URL the user
/// supplies: a broken or hostile endpoint could stream until the app ran
/// out of memory. Images already went through `read_response_limited`;
/// the chat paths never did.
const MAX_CLOUD_BODY_BYTES: usize = 8 * 1024 * 1024;

/// Read a provider body with a ceiling.
///
/// Returns an error rather than a placeholder string when the ceiling is
/// hit: several callers use the same buffer for both the error branch and
/// the success branch, and handing a failure message to `serde_json` there
/// would surface as "expected value" instead of what actually went wrong.
pub(crate) async fn bounded_body(response: reqwest::Response) -> AppResult<String> {
    let bytes = crate::image::read_response_limited(
        response,
        MAX_CLOUD_BODY_BYTES,
        "cloud response exceeded size limit",
    )
    .await?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

use crate::inference::cloud::CloudProvider;

const OPENAI_BASE: &str = "https://api.openai.com/v1";
const DEEPSEEK_BASE: &str = "https://api.deepseek.com/v1";
const OPENROUTER_BASE: &str = "https://openrouter.ai/api/v1";
const MISTRAL_BASE: &str = "https://api.mistral.ai/v1";
const GROQ_BASE: &str = "https://api.groq.com/openai/v1";
const XAI_BASE: &str = "https://api.x.ai/v1";
const ANTHROPIC_BASE: &str = "https://api.anthropic.com";
const GOOGLE_BASE: &str = "https://generativelanguage.googleapis.com";
const TOGETHER_BASE: &str = "https://api.together.xyz/v1";
const FIREWORKS_BASE: &str = "https://api.fireworks.ai/inference/v1";
const NOVITA_BASE: &str = "https://api.novita.ai/openai";
const SILICONFLOW_BASE: &str = "https://api.siliconflow.com/v1";
const MOONSHOT_BASE: &str = "https://api.moonshot.ai/v1";
const ZHIPU_BASE: &str = "https://open.bigmodel.cn/api/paas/v4";
const QWEN_BASE: &str = "https://dashscope-intl.aliyuncs.com/compatible-mode/v1";
const BAICHUAN_BASE: &str = "https://api.baichuan-ai.com/v1";
const MINIMAX_BASE: &str = "https://api.minimax.io/v1";
const STEPFUN_BASE: &str = "https://api.stepfun.ai/v1";
const MODELSCOPE_BASE: &str = "https://api-inference.modelscope.cn/v1";
const XIAOMIMIMO_BASE: &str = "https://api.xiaomimimo.com/v1";
const DOUBAO_BASE: &str = "https://ark.cn-beijing.volces.com/api/v3";
const HUNYUAN_BASE: &str = "https://api.hunyuan.cloud.tencent.com/v1";
const UPSTAGE_BASE: &str = "https://api.upstage.ai/v1";
// 01.AI serves its OpenAI-compatible API from lingyiwanwu.com. The old
// `api.01.ai` host has no DNS record at all (NXDOMAIN, checked
// 2026-08-26), so every Yi request failed before it left the machine.
const YI_BASE: &str = "https://api.lingyiwanwu.com/v1";
const PLAMO_BASE: &str = "https://api.platform.preferredai.jp/v1";
const NVIDIA_BASE: &str = "https://integrate.api.nvidia.com/v1";
const COHERE_BASE: &str = "https://api.cohere.com/v1";
const CEREBRAS_BASE: &str = "https://api.cerebras.ai/v1";
const SAMBANOVA_BASE: &str = "https://api.sambanova.ai/v1";
const PERPLEXITY_BASE: &str = "https://api.perplexity.ai";
const AI21_BASE: &str = "https://api.ai21.com/studio/v1";
const VENICE_BASE: &str = "https://api.venice.ai/api/v1";
// Black Forest Labs moved off the `.ml` domain. `api.bfl.ml` still
// resolves but refuses connections; `api.bfl.ai` answers (403 without
// a key). Checked 2026-08-26.
const BFL_BASE: &str = "https://api.bfl.ai/v1";

/// Every provider base in one place, so the invariants below can be
/// asserted over the whole set instead of whichever one somebody
/// remembered to add to a test.
#[cfg(test)]
const ALL_BASES: &[(&str, &str)] = &[
    ("OPENAI_BASE", OPENAI_BASE),
    ("DEEPSEEK_BASE", DEEPSEEK_BASE),
    ("OPENROUTER_BASE", OPENROUTER_BASE),
    ("MISTRAL_BASE", MISTRAL_BASE),
    ("GROQ_BASE", GROQ_BASE),
    ("XAI_BASE", XAI_BASE),
    ("ANTHROPIC_BASE", ANTHROPIC_BASE),
    ("GOOGLE_BASE", GOOGLE_BASE),
    ("TOGETHER_BASE", TOGETHER_BASE),
    ("FIREWORKS_BASE", FIREWORKS_BASE),
    ("NOVITA_BASE", NOVITA_BASE),
    ("SILICONFLOW_BASE", SILICONFLOW_BASE),
    ("MOONSHOT_BASE", MOONSHOT_BASE),
    ("ZHIPU_BASE", ZHIPU_BASE),
    ("QWEN_BASE", QWEN_BASE),
    ("BAICHUAN_BASE", BAICHUAN_BASE),
    ("MINIMAX_BASE", MINIMAX_BASE),
    ("STEPFUN_BASE", STEPFUN_BASE),
    ("MODELSCOPE_BASE", MODELSCOPE_BASE),
    ("XIAOMIMIMO_BASE", XIAOMIMIMO_BASE),
    ("DOUBAO_BASE", DOUBAO_BASE),
    ("HUNYUAN_BASE", HUNYUAN_BASE),
    ("UPSTAGE_BASE", UPSTAGE_BASE),
    ("YI_BASE", YI_BASE),
    ("PLAMO_BASE", PLAMO_BASE),
    ("NVIDIA_BASE", NVIDIA_BASE),
    ("COHERE_BASE", COHERE_BASE),
    ("CEREBRAS_BASE", CEREBRAS_BASE),
    ("SAMBANOVA_BASE", SAMBANOVA_BASE),
    ("PERPLEXITY_BASE", PERPLEXITY_BASE),
    ("AI21_BASE", AI21_BASE),
    ("VENICE_BASE", VENICE_BASE),
    ("BFL_BASE", BFL_BASE),
];
const MAX_SSE_BUFFER_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct GenerationOptions {
    pub temperature: f64,
    pub top_p: f64,
    pub max_tokens: i64,
}

fn openai_compat_body(
    model: &str,
    prompt: &str,
    options: GenerationOptions,
    stream: bool,
    provider: CloudProvider,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "temperature": options.temperature,
        "top_p": options.top_p,
    });
    // OpenAI's o-series (o1/o3/o4-mini) and the gpt-5.x reasoning family both
    // reject max_tokens and require max_completion_tokens; legacy gpt-3.5-turbo
    // and pre-4o gpt-4/gpt-4-turbo reject max_completion_tokens and require
    // max_tokens, so this can't be simplified to always send the new field.
    //
    // The 'o' prefix heuristic is scoped to hosted providers: a CUSTOM
    // OpenAI-compatible endpoint (LM Studio, vLLM, SGLang, Ollama…) frequently
    // serves open-weight models whose names start with 'o' (openhermes, olmo,
    // orca, openchat…) and would otherwise get a max_completion_tokens field
    // those servers reject — a custom narrator that never answers.
    let token_key = if provider != CloudProvider::Custom
        && (model.starts_with('o') || model.starts_with("gpt-5"))
    {
        "max_completion_tokens"
    } else {
        "max_tokens"
    };
    body[token_key] = serde_json::json!(options.max_tokens);
    if stream {
        body["stream"] = serde_json::json!(true);
    }
    body
}

/// Resolve the endpoint base URL for a provider (custom uses the saved URL).
pub fn base_url_for(provider: CloudProvider) -> AppResult<String> {
    let base = match provider {
        CloudProvider::Openai => OPENAI_BASE.to_string(),
        CloudProvider::Deepseek => DEEPSEEK_BASE.to_string(),
        CloudProvider::OpenRouter => OPENROUTER_BASE.to_string(),
        CloudProvider::Mistral => MISTRAL_BASE.to_string(),
        CloudProvider::Groq => GROQ_BASE.to_string(),
        CloudProvider::Xai => XAI_BASE.to_string(),
        CloudProvider::Anthropic => ANTHROPIC_BASE.to_string(),
        CloudProvider::Google => GOOGLE_BASE.to_string(),
        CloudProvider::Together => TOGETHER_BASE.to_string(),
        CloudProvider::Fireworks => FIREWORKS_BASE.to_string(),
        CloudProvider::Novita => NOVITA_BASE.to_string(),
        CloudProvider::SiliconFlow => SILICONFLOW_BASE.to_string(),
        CloudProvider::Moonshot => MOONSHOT_BASE.to_string(),
        CloudProvider::Zhipu => ZHIPU_BASE.to_string(),
        CloudProvider::Qwen => QWEN_BASE.to_string(),
        CloudProvider::Baichuan => BAICHUAN_BASE.to_string(),
        CloudProvider::Minimax => MINIMAX_BASE.to_string(),
        CloudProvider::Stepfun => STEPFUN_BASE.to_string(),
        CloudProvider::ModelScope => MODELSCOPE_BASE.to_string(),
        CloudProvider::XiaomiMimo => XIAOMIMIMO_BASE.to_string(),
        CloudProvider::Doubao => DOUBAO_BASE.to_string(),
        CloudProvider::Hunyuan => HUNYUAN_BASE.to_string(),
        CloudProvider::Upstage => UPSTAGE_BASE.to_string(),
        CloudProvider::Yi => YI_BASE.to_string(),
        CloudProvider::Plamo => PLAMO_BASE.to_string(),
        CloudProvider::Nvidia => NVIDIA_BASE.to_string(),
        CloudProvider::Cohere => COHERE_BASE.to_string(),
        CloudProvider::Cerebras => CEREBRAS_BASE.to_string(),
        CloudProvider::SambaNova => SAMBANOVA_BASE.to_string(),
        CloudProvider::Perplexity => PERPLEXITY_BASE.to_string(),
        CloudProvider::Ai21 => AI21_BASE.to_string(),
        CloudProvider::Venice => VENICE_BASE.to_string(),
        CloudProvider::Bfl => BFL_BASE.to_string(),
        // fal.ai has no fixed base: it's image-only (excluded from chat/model
        // discovery in commands/cloud.rs) and the endpoint the user pastes IS
        // the model, not a host to append a path to. No current caller reaches
        // this arm, but this crate builds release with `panic = "abort"`, so a
        // future caller must get a normal error here instead of taking down
        // the whole app — same guarantee as `unreachable!()`, no crash vector.
        CloudProvider::Fal => {
            return Err(AppError::Config(
                "fal has no fixed base URL: the pasted endpoint is the model".into(),
            ))
        }
        CloudProvider::Custom => crate::byok::read_base_url()?
            .ok_or_else(|| AppError::Config("custom provider has no base URL saved".into()))?,
    };
    Ok(base)
}

/// Some providers reject temperature values OpenAI itself allows (0-2) —
/// Zhipu and Moonshot cap at 1.0, DashScope's compatible-mode requires
/// strictly less than 2.0. The in-app "Unpredictability" slider goes up to
/// 2.0 for every provider (no reason to cap the feature for providers that
/// support the full range); clamp per-provider here instead so a request
/// never 400s just because the user's saved preference exceeds what THIS
/// provider accepts.
fn max_temperature_for(provider: CloudProvider) -> f64 {
    match provider {
        // Anthropic's own documented range is 0.0-1.0 (not the 0-2 OpenAI
        // convention) — this predates today's provider expansion and was
        // already reachable via the existing 0.1-2.0 slider.
        CloudProvider::Anthropic | CloudProvider::Zhipu | CloudProvider::Moonshot => 1.0,
        CloudProvider::Qwen => 1.99,
        _ => 2.0,
    }
}

/// One chat turn against a cloud provider. `prompt` is the full narration
/// prompt (already built by the frontend prompt builder).
pub async fn chat(
    provider: CloudProvider,
    model: &str,
    prompt: &str,
    api_key: &str,
    base_url: &str,
    options: GenerationOptions,
) -> AppResult<String> {
    let options = GenerationOptions {
        temperature: options.temperature.min(max_temperature_for(provider)),
        ..options
    };
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| AppError::Other(format!("http client: {e}")))?;

    match provider {
        CloudProvider::Anthropic => {
            anthropic_chat(&client, model, prompt, api_key, base_url, options).await
        }
        CloudProvider::Google => {
            google_chat(&client, model, prompt, api_key, base_url, options).await
        }
        _ => openai_compat_chat(&client, provider, model, prompt, api_key, base_url, options).await,
    }
}

pub async fn chat_stream<F>(
    provider: CloudProvider,
    model: &str,
    prompt: &str,
    api_key: &str,
    base_url: &str,
    options: GenerationOptions,
    on_token: F,
) -> AppResult<String>
where
    F: FnMut(&str) + Send,
{
    let options = GenerationOptions {
        temperature: options.temperature.min(max_temperature_for(provider)),
        ..options
    };
    match provider {
        CloudProvider::Anthropic => {
            anthropic_chat_stream(model, prompt, api_key, base_url, options, on_token).await
        }
        CloudProvider::Google => {
            google_chat_stream(model, prompt, api_key, base_url, options, on_token).await
        }
        _ => {
            openai_compat_chat_stream(
                provider, model, prompt, api_key, base_url, options, on_token,
            )
            .await
        }
    }
}

async fn openai_compat_chat(
    client: &reqwest::Client,
    provider: CloudProvider,
    model: &str,
    prompt: &str,
    api_key: &str,
    base_url: &str,
    options: GenerationOptions,
) -> AppResult<String> {
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let body = openai_compat_body(model, prompt, options, false, provider);
    let resp = client
        .post(&url)
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .map_err(|_| AppError::Other("cloud request failed".into()))?;

    let status = resp.status();
    let text = bounded_body(resp).await?;
    if !status.is_success() {
        return Err(AppError::Other(format!(
            "cloud provider {} error: {}",
            status,
            crate::inference::cloud::sanitize_error_body(&text)
        )));
    }

    let parsed: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| AppError::Other(format!("cloud response parse: {e}")))?;
    extract_message_content(&parsed)
        .ok_or_else(|| AppError::Other("cloud response had no message content".into()))
}

/// Some reasoning-mode models (Kimi K2.6+/K2.7 thinking, Qwen3 thinking, GLM
/// thinking variants) return the chain-of-thought in a separate
/// `reasoning_content` field and leave `content` null or empty once the
/// model spent its whole budget thinking without emitting a final answer.
/// Falling back avoids a hard "no message content" error in that case —
/// a visible-thinking answer beats a failed turn.
fn extract_message_content(parsed: &serde_json::Value) -> Option<String> {
    let message = parsed.pointer("/choices/0/message")?;
    let content = message.get("content").and_then(|v| v.as_str());
    match content {
        Some(s) if !s.is_empty() => Some(s.to_string()),
        _ => message
            .get("reasoning_content")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string()),
    }
}

async fn openai_compat_chat_stream<F>(
    provider: CloudProvider,
    model: &str,
    prompt: &str,
    api_key: &str,
    base_url: &str,
    options: GenerationOptions,
    mut on_token: F,
) -> AppResult<String>
where
    F: FnMut(&str) + Send,
{
    use futures_util::StreamExt;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| AppError::Other(format!("http client: {e}")))?;
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let body = openai_compat_body(model, prompt, options, true, provider);
    let response = client
        .post(&url)
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .map_err(|_| AppError::Other("cloud stream request failed".into()))?;
    let status = response.status();
    if !status.is_success() {
        let text = bounded_body(response).await?;
        return Err(AppError::Other(format!(
            "cloud provider {} error: {}",
            status,
            crate::inference::cloud::sanitize_error_body(&text)
        )));
    }

    let mut stream = response.bytes_stream();
    // Buffer raw BYTES, not decoded strings: decoding each HTTP chunk with
    // String::from_utf8_lossy corrupts any multi-byte UTF-8 character split
    // across a chunk boundary (replaced with U+FFFD) — Polish diacritics
    // ą/ę/ś/ł/ż break on every long response. Complete SSE lines are always
    // valid UTF-8, so we only decode at line boundaries.
    let mut buffer: Vec<u8> = Vec::new();
    let mut full = String::new();
    let mut saw_done = false;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| AppError::Other("cloud stream interrupted".into()))?;
        buffer.extend_from_slice(&chunk);
        if buffer.len() > MAX_SSE_BUFFER_BYTES {
            return Err(AppError::Other("cloud stream event exceeded 4 MiB".into()));
        }
        while let Some(newline) = buffer.iter().position(|b| *b == b'\n') {
            let line = String::from_utf8_lossy(&buffer[..newline])
                .trim()
                .to_string();
            buffer.drain(..=newline);
            process_openai_sse_line(&line, &mut on_token, &mut full, &mut saw_done)?;
        }
    }
    // Providers may close a chunk exactly after the final event, without a
    // trailing newline. Parse it before deciding the stream is incomplete.
    if !buffer.is_empty() {
        let line = String::from_utf8_lossy(&buffer).trim().to_string();
        if !line.is_empty() {
            process_openai_sse_line(&line, &mut on_token, &mut full, &mut saw_done)?;
        }
    }
    if !saw_done {
        return Err(AppError::Other("cloud stream ended before [DONE]".into()));
    }
    // A reasoning-mode model can spend its whole turn on chain-of-thought
    // (streamed as `delta.reasoning_content`, which this parser doesn't
    // forward — mixing thinking into narration text would corrupt the
    // story, unlike the non-streaming path where content vs. reasoning is
    // known only once the message is complete) and never emit a
    // `delta.content` token. `[DONE]` still arrives, so this would
    // otherwise return a silent, empty narration turn instead of a clear,
    // retryable error.
    if full.is_empty() {
        return Err(AppError::Other("cloud stream returned no text".into()));
    }
    Ok(full)
}

fn process_openai_sse_line<F>(
    line: &str,
    on_token: &mut F,
    full: &mut String,
    saw_done: &mut bool,
) -> AppResult<()>
where
    F: FnMut(&str),
{
    let Some(data) = line.strip_prefix("data:").map(str::trim) else {
        return Ok(());
    };
    if data == "[DONE]" {
        *saw_done = true;
        return Ok(());
    }
    let payload: serde_json::Value = serde_json::from_str(data)
        .map_err(|_| AppError::Other("cloud stream returned invalid JSON".into()))?;
    if let Some(token) = payload
        .pointer("/choices/0/delta/content")
        .and_then(|v| v.as_str())
    {
        on_token(token);
        full.push_str(token);
    }
    Ok(())
}

async fn anthropic_chat(
    client: &reqwest::Client,
    model: &str,
    prompt: &str,
    api_key: &str,
    base_url: &str,
    options: GenerationOptions,
) -> AppResult<String> {
    let url = format!("{}/v1/messages", base_url.trim_end_matches('/'));
    let body = serde_json::json!({
        "model": model,
        "max_tokens": options.max_tokens,
        "temperature": options.temperature,
        "messages": [{"role": "user", "content": prompt}],
    });
    let resp = client
        .post(&url)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .map_err(|_| AppError::Other("cloud request failed".into()))?;

    let status = resp.status();
    let text = bounded_body(resp).await?;
    if !status.is_success() {
        return Err(AppError::Other(format!(
            "anthropic error: {}",
            crate::inference::cloud::sanitize_error_body(&text)
        )));
    }
    let parsed: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| AppError::Other(format!("anthropic response parse: {e}")))?;
    let content = parsed
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|b| b.get("text"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| AppError::Other("anthropic response had no text content".into()))?;
    Ok(content)
}

async fn anthropic_chat_stream<F>(
    model: &str,
    prompt: &str,
    api_key: &str,
    base_url: &str,
    options: GenerationOptions,
    on_token: F,
) -> AppResult<String>
where
    F: FnMut(&str) + Send,
{
    let url = format!("{}/v1/messages", base_url.trim_end_matches('/'));
    let body = serde_json::json!({
        "model": model,
        "max_tokens": options.max_tokens,
        "temperature": options.temperature,
        "messages": [{"role": "user", "content": prompt}],
        "stream": true,
    });
    sse_text_stream(
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| AppError::Other(format!("http client: {e}")))?
            .post(url)
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body),
        "/delta/text",
        SseCompletion::Anthropic,
        on_token,
    )
    .await
}

async fn google_chat(
    client: &reqwest::Client,
    model: &str,
    prompt: &str,
    api_key: &str,
    _base_url: &str,
    options: GenerationOptions,
) -> AppResult<String> {
    let url = format!("{GOOGLE_BASE}/v1beta/models/{model}:generateContent");
    let body = serde_json::json!({
        "contents": [{"role": "user", "parts": [{"text": prompt}]}],
        "generationConfig": {
            "maxOutputTokens": options.max_tokens,
            "temperature": options.temperature,
            "topP": options.top_p
        }
    });
    let resp = client
        .post(&url)
        .header("x-goog-api-key", api_key)
        .json(&body)
        .send()
        .await
        .map_err(|_| AppError::Other("cloud request failed".into()))?;

    let status = resp.status();
    let text = bounded_body(resp).await?;
    if !status.is_success() {
        return Err(AppError::Other(format!(
            "google error: {}",
            crate::inference::cloud::sanitize_error_body(&text)
        )));
    }
    let parsed: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| AppError::Other(format!("google response parse: {e}")))?;
    parsed
        .pointer("/candidates/0/content/parts/0/text")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| AppError::Other("google response had no text content".into()))
}

async fn google_chat_stream<F>(
    model: &str,
    prompt: &str,
    api_key: &str,
    base_url: &str,
    options: GenerationOptions,
    on_token: F,
) -> AppResult<String>
where
    F: FnMut(&str) + Send,
{
    let url = format!(
        "{}/v1beta/models/{}:streamGenerateContent?alt=sse",
        base_url.trim_end_matches('/'),
        model
    );
    let body = serde_json::json!({
        "contents": [{"role": "user", "parts": [{"text": prompt}]}],
        "generationConfig": {
            "maxOutputTokens": options.max_tokens,
            "temperature": options.temperature,
            "topP": options.top_p
        }
    });
    sse_text_stream(
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| AppError::Other(format!("http client: {e}")))?
            .post(url)
            .header("x-goog-api-key", api_key)
            .json(&body),
        "/candidates/0/content/parts/0/text",
        SseCompletion::Google,
        on_token,
    )
    .await
}

async fn sse_text_stream<F>(
    request: reqwest::RequestBuilder,
    text_pointer: &str,
    completion: SseCompletion,
    mut on_token: F,
) -> AppResult<String>
where
    F: FnMut(&str) + Send,
{
    use futures_util::StreamExt;

    let response = request
        .timeout(std::time::Duration::from_secs(300))
        .send()
        .await
        .map_err(|_| AppError::Other("cloud stream request failed".into()))?;
    let status = response.status();
    if !status.is_success() {
        let text = bounded_body(response).await?;
        return Err(AppError::Other(format!(
            "cloud provider {} error: {}",
            status,
            crate::inference::cloud::sanitize_error_body(&text)
        )));
    }
    let mut stream = response.bytes_stream();
    let mut buffer: Vec<u8> = Vec::new();
    let mut full = String::new();
    let mut completed = false;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| AppError::Other("cloud stream interrupted".into()))?;
        buffer.extend_from_slice(&chunk);
        if buffer.len() > MAX_SSE_BUFFER_BYTES {
            return Err(AppError::Other("cloud stream event exceeded 4 MiB".into()));
        }
        while let Some(newline) = buffer.iter().position(|b| *b == b'\n') {
            let line = String::from_utf8_lossy(&buffer[..newline])
                .trim()
                .to_string();
            buffer.drain(..=newline);
            let Some(data) = line.strip_prefix("data:").map(str::trim) else {
                continue;
            };
            if data.is_empty() {
                continue;
            }
            if data == "[DONE]" {
                completed = true;
                continue;
            }
            let payload: serde_json::Value = serde_json::from_str(data)
                .map_err(|_| AppError::Other("cloud stream returned invalid JSON".into()))?;
            if let Some(token) = payload
                .pointer(text_pointer)
                .and_then(|value| value.as_str())
            {
                on_token(token);
                full.push_str(token);
            }
            completed |= match completion {
                SseCompletion::Anthropic => {
                    payload.get("type").and_then(|v| v.as_str()) == Some("message_stop")
                }
                SseCompletion::Google => payload
                    .pointer("/candidates/0/finishReason")
                    .and_then(|v| v.as_str())
                    .is_some(),
            };
        }
    }
    if !buffer.is_empty() {
        let tail = String::from_utf8_lossy(&buffer);
        let data = tail
            .trim()
            .strip_prefix("data:")
            .map(str::trim)
            .ok_or_else(|| {
                AppError::Other("cloud stream ended with an incomplete SSE event".into())
            })?;
        if data.is_empty() { /* harmless final blank event */
        } else if data == "[DONE]" {
            completed = true;
        } else {
            let payload: serde_json::Value = serde_json::from_str(data)
                .map_err(|_| AppError::Other("cloud stream returned invalid JSON".into()))?;
            if let Some(token) = payload
                .pointer(text_pointer)
                .and_then(|value| value.as_str())
            {
                on_token(token);
                full.push_str(token);
            }
            completed |= match completion {
                SseCompletion::Anthropic => {
                    payload.get("type").and_then(|v| v.as_str()) == Some("message_stop")
                }
                SseCompletion::Google => payload
                    .pointer("/candidates/0/finishReason")
                    .and_then(|v| v.as_str())
                    .is_some(),
            };
        }
    }
    if !completed {
        return Err(AppError::Other(
            "cloud stream ended before provider completion".into(),
        ));
    }
    if full.is_empty() {
        return Err(AppError::Other("cloud stream returned no text".into()));
    }
    Ok(full)
}

#[derive(Debug, Clone, Copy)]
enum SseCompletion {
    Anthropic,
    Google,
}

#[cfg(test)]
mod tests {
    /// Dwa hosty w tej liście umarły w locie i nikt tego nie zauważył, bo
    /// objawem jest błąd sieci u użytkownika, nie błąd kompilacji:
    /// `api.01.ai` stracił rekord DNS, a `api.bfl.ml` przestał przyjmować
    /// połączenia po przeprowadzce Black Forest Labs na `.ai`.
    #[test]
    fn retired_provider_hosts_do_not_come_back() {
        for dead in ["api.01.ai", "api.bfl.ml"] {
            assert!(
                !YI_BASE.contains(dead) && !BFL_BASE.contains(dead),
                "{dead} is a retired host — see the comments above these constants"
            );
        }
        assert!(YI_BASE.starts_with("https://api.lingyiwanwu.com"));
        assert!(BFL_BASE.starts_with("https://api.bfl.ai"));
    }

    /// Każda baza musi być absolutnym https bez końcowego ukośnika —
    /// URL-e budujemy przez `format!("{base}/...")`, więc ukośnik dałby
    /// podwójny separator, a http byłoby cichym downgrade'em.
    #[test]
    fn every_base_url_is_https_and_has_no_trailing_slash() {
        for (name, base) in ALL_BASES {
            assert!(base.starts_with("https://"), "{name} is not https: {base}");
            assert!(!base.ends_with('/'), "{name} has a trailing slash: {base}");
        }
    }

    use super::*;

    #[test]
    fn temperature_cap_matches_each_providers_documented_range() {
        assert_eq!(max_temperature_for(CloudProvider::Anthropic), 1.0);
        assert_eq!(max_temperature_for(CloudProvider::Zhipu), 1.0);
        assert_eq!(max_temperature_for(CloudProvider::Moonshot), 1.0);
        assert_eq!(max_temperature_for(CloudProvider::Qwen), 1.99);
        assert_eq!(max_temperature_for(CloudProvider::Openai), 2.0);
        assert_eq!(max_temperature_for(CloudProvider::Together), 2.0);
    }

    #[test]
    fn base_urls_are_sensible() {
        assert_eq!(
            base_url_for(CloudProvider::Openai).unwrap(),
            "https://api.openai.com/v1"
        );
        assert_eq!(
            base_url_for(CloudProvider::Deepseek).unwrap(),
            "https://api.deepseek.com/v1"
        );
        assert_eq!(
            base_url_for(CloudProvider::Xai).unwrap(),
            "https://api.x.ai/v1"
        );
        assert_eq!(
            base_url_for(CloudProvider::Together).unwrap(),
            "https://api.together.xyz/v1"
        );
        assert_eq!(
            base_url_for(CloudProvider::Moonshot).unwrap(),
            "https://api.moonshot.ai/v1"
        );
        assert_eq!(
            base_url_for(CloudProvider::Qwen).unwrap(),
            "https://dashscope-intl.aliyuncs.com/compatible-mode/v1"
        );
        assert_eq!(
            base_url_for(CloudProvider::Zhipu).unwrap(),
            "https://open.bigmodel.cn/api/paas/v4"
        );
        assert_eq!(
            base_url_for(CloudProvider::Plamo).unwrap(),
            "https://api.platform.preferredai.jp/v1"
        );
        assert_eq!(
            base_url_for(CloudProvider::Venice).unwrap(),
            "https://api.venice.ai/api/v1"
        );
    }

    /// `base_url_for`'s `Fal` arm returns `AppError::Config` instead of
    /// `unreachable!()` specifically because this crate builds release with
    /// `panic = "abort"` — a future caller reaching this arm must get a
    /// normal error, not take down the whole app. Nothing asserted that
    /// behaviour until now.
    #[test]
    fn fal_base_url_errors_instead_of_panicking() {
        assert!(base_url_for(CloudProvider::Fal).is_err());
    }

    #[test]
    fn openai_final_buffered_event_is_processed() {
        let mut tokens = Vec::new();
        let mut full = String::new();
        let mut done = false;
        process_openai_sse_line(
            r#"data: {"choices":[{"delta":{"content":"last"}}]}"#,
            &mut |token| tokens.push(token.to_string()),
            &mut full,
            &mut done,
        )
        .unwrap();
        process_openai_sse_line("data: [DONE]", &mut |_| {}, &mut full, &mut done).unwrap();
        assert_eq!(tokens, ["last"]);
        assert_eq!(full, "last");
        assert!(done);
    }

    #[test]
    fn extract_message_content_prefers_content_over_reasoning() {
        let parsed: serde_json::Value = serde_json::from_str(
            r#"{"choices":[{"message":{"content":"the answer","reasoning_content":"thinking..."}}]}"#,
        )
        .unwrap();
        assert_eq!(
            extract_message_content(&parsed).as_deref(),
            Some("the answer")
        );
    }

    #[test]
    fn extract_message_content_falls_back_to_reasoning_when_content_is_empty() {
        let parsed: serde_json::Value = serde_json::from_str(
            r#"{"choices":[{"message":{"content":"","reasoning_content":"thinking..."}}]}"#,
        )
        .unwrap();
        assert_eq!(
            extract_message_content(&parsed).as_deref(),
            Some("thinking...")
        );

        let parsed_null: serde_json::Value = serde_json::from_str(
            r#"{"choices":[{"message":{"content":null,"reasoning_content":"thinking..."}}]}"#,
        )
        .unwrap();
        assert_eq!(
            extract_message_content(&parsed_null).as_deref(),
            Some("thinking...")
        );
    }

    #[test]
    fn extract_message_content_none_when_both_empty() {
        let parsed: serde_json::Value =
            serde_json::from_str(r#"{"choices":[{"message":{"content":""}}]}"#).unwrap();
        assert_eq!(extract_message_content(&parsed), None);
    }

    #[test]
    fn reasoning_models_use_completion_token_limit() {
        for model in ["o4-mini", "gpt-5", "gpt-5.1", "gpt-5-mini"] {
            let body = openai_compat_body(
                model,
                "prompt",
                GenerationOptions {
                    temperature: 0.7,
                    top_p: 0.9,
                    max_tokens: 100,
                },
                false,
                CloudProvider::Openai,
            );
            assert_eq!(body["max_completion_tokens"], 100, "model {model}");
            assert!(body.get("max_tokens").is_none(), "model {model}");
        }
    }

    #[test]
    fn regular_models_use_legacy_chat_token_limit() {
        let body = openai_compat_body(
            "gpt-4.1",
            "prompt",
            GenerationOptions {
                temperature: 0.7,
                top_p: 0.9,
                max_tokens: 100,
            },
            false,
            CloudProvider::Openai,
        );
        assert_eq!(body["max_tokens"], 100);
    }

    #[test]
    fn custom_models_never_use_completion_token_limit() {
        // A custom OpenAI-compatible endpoint (LM Studio / vLLM / Ollama)
        // serving open-weight models whose names start with 'o' must keep
        // getting the legacy max_tokens field — the reasoning-field heuristic
        // is for hosted providers only (see openai_compat_body).
        for model in ["openhermes-2.5", "olmo-7b", "orca-mini", "o1-like-local"] {
            let body = openai_compat_body(
                model,
                "prompt",
                GenerationOptions {
                    temperature: 0.7,
                    top_p: 0.9,
                    max_tokens: 100,
                },
                false,
                CloudProvider::Custom,
            );
            assert_eq!(body["max_tokens"], 100, "model {model}");
            assert!(body.get("max_completion_tokens").is_none(), "model {model}");
        }
    }
}
