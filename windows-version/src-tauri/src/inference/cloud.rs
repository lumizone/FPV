//! Minimal cloud provider types for BYOK key storage.
//! FPV doesn't use the full cloud dispatch pipeline — just key management.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudProvider {
    Openai,
    Anthropic,
    Deepseek,
    Google,
    OpenRouter,
    Mistral,
    Groq,
    Xai,
    Together,
    Fireworks,
    Novita,
    SiliconFlow,
    Moonshot,
    Zhipu,
    Qwen,
    Baichuan,
    Minimax,
    Stepfun,
    ModelScope,
    XiaomiMimo,
    Doubao,
    Hunyuan,
    Upstage,
    Yi,
    Plamo,
    Nvidia,
    Cohere,
    Cerebras,
    SambaNova,
    Perplexity,
    Ai21,
    Venice,
    Bfl,
    Custom,
}

impl CloudProvider {
    /// Every provider, for exhaustive iteration (e.g. clearing all BYOK keys).
    pub const ALL: [CloudProvider; 34] = [
        Self::Openai,
        Self::Anthropic,
        Self::Deepseek,
        Self::Google,
        Self::OpenRouter,
        Self::Mistral,
        Self::Groq,
        Self::Xai,
        Self::Together,
        Self::Fireworks,
        Self::Novita,
        Self::SiliconFlow,
        Self::Moonshot,
        Self::Zhipu,
        Self::Qwen,
        Self::Baichuan,
        Self::Minimax,
        Self::Stepfun,
        Self::ModelScope,
        Self::XiaomiMimo,
        Self::Doubao,
        Self::Hunyuan,
        Self::Upstage,
        Self::Yi,
        Self::Plamo,
        Self::Nvidia,
        Self::Cohere,
        Self::Cerebras,
        Self::SambaNova,
        Self::Perplexity,
        Self::Ai21,
        Self::Venice,
        Self::Bfl,
        Self::Custom,
    ];

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "openai" => Some(Self::Openai),
            "anthropic" => Some(Self::Anthropic),
            "deepseek" => Some(Self::Deepseek),
            "google" => Some(Self::Google),
            "openrouter" => Some(Self::OpenRouter),
            "mistral" => Some(Self::Mistral),
            "groq" => Some(Self::Groq),
            "xai" | "grok" => Some(Self::Xai),
            "together" => Some(Self::Together),
            "fireworks" => Some(Self::Fireworks),
            "novita" => Some(Self::Novita),
            "siliconflow" => Some(Self::SiliconFlow),
            "moonshot" => Some(Self::Moonshot),
            "zhipu" => Some(Self::Zhipu),
            "qwen" => Some(Self::Qwen),
            "baichuan" => Some(Self::Baichuan),
            "minimax" => Some(Self::Minimax),
            "stepfun" => Some(Self::Stepfun),
            "modelscope" => Some(Self::ModelScope),
            "xiaomimimo" => Some(Self::XiaomiMimo),
            "doubao" => Some(Self::Doubao),
            "hunyuan" => Some(Self::Hunyuan),
            "upstage" => Some(Self::Upstage),
            "yi" => Some(Self::Yi),
            "plamo" => Some(Self::Plamo),
            "nvidia" => Some(Self::Nvidia),
            "cohere" => Some(Self::Cohere),
            "cerebras" => Some(Self::Cerebras),
            "sambanova" => Some(Self::SambaNova),
            "perplexity" => Some(Self::Perplexity),
            "ai21" => Some(Self::Ai21),
            "venice" => Some(Self::Venice),
            "bfl" => Some(Self::Bfl),
            "custom" => Some(Self::Custom),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Openai => "openai",
            Self::Anthropic => "anthropic",
            Self::Deepseek => "deepseek",
            Self::Google => "google",
            Self::OpenRouter => "openrouter",
            Self::Mistral => "mistral",
            Self::Groq => "groq",
            Self::Xai => "xai",
            Self::Together => "together",
            Self::Fireworks => "fireworks",
            Self::Novita => "novita",
            Self::SiliconFlow => "siliconflow",
            Self::Moonshot => "moonshot",
            Self::Zhipu => "zhipu",
            Self::Qwen => "qwen",
            Self::Baichuan => "baichuan",
            Self::Minimax => "minimax",
            Self::Stepfun => "stepfun",
            Self::ModelScope => "modelscope",
            Self::XiaomiMimo => "xiaomimimo",
            Self::Doubao => "doubao",
            Self::Hunyuan => "hunyuan",
            Self::Upstage => "upstage",
            Self::Yi => "yi",
            Self::Plamo => "plamo",
            Self::Nvidia => "nvidia",
            Self::Cohere => "cohere",
            Self::Cerebras => "cerebras",
            Self::SambaNova => "sambanova",
            Self::Perplexity => "perplexity",
            Self::Ai21 => "ai21",
            Self::Venice => "venice",
            Self::Bfl => "bfl",
            Self::Custom => "custom",
        }
    }
}

/// Sanitize error text by stripping anything that looks like a token/secret.
/// Used by license client and other callers to prevent key leakage.
pub fn sanitize_error_body(text: &str) -> String {
    // Simple redaction of common patterns without a regex dependency.
    const PATTERNS: [&str; 13] = [
        "sk-",
        "api_key=",
        "apikey=",
        "api-key=",
        "bearer ",
        "key=",
        "secret=",
        "token=",
        "authorization: ",
        "\"apikey\":\"",
        "\"api_key\":\"",
        "\"key\":\"",
        "x-goog-api-key: ",
    ];
    const MAX_LEN: usize = 200;

    let mut result = text.to_string();
    for pattern in PATTERNS {
        // Match case-insensitively against a lowercase view of the current text.
        // to_ascii_lowercase() only rewrites ASCII bytes in place, so byte offsets
        // found in the lowercase view stay valid against `result`.
        while let Some(start) = result.to_ascii_lowercase().find(pattern) {
            let value_start = start + pattern.len();
            let end = result[value_start..]
                .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ',' | '&' | '}'))
                .map_or(result.len(), |offset| value_start + offset);
            result.replace_range(start..end, "[redacted]");
        }
    }

    if result.len() > MAX_LEN {
        let cut = (0..=MAX_LEN)
            .rev()
            .find(|&i| result.is_char_boundary(i))
            .unwrap_or(0);
        format!("{}... (truncated)", &result[..cut])
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::sanitize_error_body;

    #[test]
    fn redacts_json_and_query_style_keys() {
        let result = sanitize_error_body(r#"{"apiKey":"secret-value","key":"other","x":"ok"}"#);
        assert!(!result.contains("secret-value"));
        assert!(!result.contains("other"));
        assert!(result.contains("[redacted]"));
    }
}
