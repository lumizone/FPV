use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Serialize)]
pub struct GenerationMetric<'a> {
    pub timestamp: u64,
    pub stage: &'a str,
    pub provider: &'a str,
    pub model: &'a str,
    pub input_tokens_estimate: usize,
    pub output_tokens_estimate: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens_actual: Option<u64>,
    pub duration_ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_token_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_duration_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_per_second: Option<f64>,
    pub streaming: bool,
}

pub fn record_generation(metric: GenerationMetric<'_>) {
    let Some(path) = metrics_path() else { return };
    let Some(dir) = path.parent() else { return };
    if let Ok(metadata) = std::fs::metadata(&path) {
        if metadata.len() > 2 * 1024 * 1024 {
            let _ = std::fs::rename(&path, dir.join("generation-metrics.jsonl.old"));
        }
    }
    let Ok(line) = serde_json::to_string(&metric) else {
        tracing::debug!("metrics: serialize failed");
        return;
    };
    use std::io::Write;
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        Ok(mut file) => {
            if let Err(err) = writeln!(file, "{line}") {
                tracing::debug!(?err, "metrics: append failed");
            }
        }
        Err(err) => tracing::debug!(?err, "metrics: open failed"),
    }
}

pub fn metrics_path() -> Option<std::path::PathBuf> {
    crate::storage::app_data_dir()
        .ok()
        .map(|dir| dir.join("generation-metrics.jsonl"))
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}
