//! Minimal, local bridge to an already authenticated Codex CLI.
//!
//! FPV never reads Codex credentials. The user signs in with `codex login`,
//! then this module speaks the documented app-server JSON-RPC protocol over
//! stdio for a single isolated narration turn.

use std::process::Stdio;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use crate::error::{AppError, AppResult};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);
// Windows: suppress the console window a spawned console binary would
// otherwise attach (status probe, login, and the long-running app-server).
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const TURN_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_MESSAGE_BYTES: usize = 2 * 1024 * 1024;

pub fn is_codex_model(model: &str) -> bool {
    model
        .strip_prefix("codex:")
        .is_some_and(|name| !name.trim().is_empty())
}

fn executable() -> String {
    // GUI apps often have a minimal PATH. Prefer standard Homebrew locations,
    // then fall back to PATH. No renderer-supplied path is ever executed.
    ["/opt/homebrew/bin/codex", "/usr/local/bin/codex"]
        .iter()
        .find(|path| std::path::Path::new(path).is_file())
        .map(|path| (*path).to_string())
        .unwrap_or_else(|| "codex".into())
}

pub async fn status() -> bool {
    tokio::time::timeout(STARTUP_TIMEOUT, {
        let mut cmd = Command::new(executable());
        cmd.args(["login", "status"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        cmd.status()
    })
    .await
    .ok()
    .and_then(Result::ok)
    .is_some_and(|status| status.success())
}

/// Start Codex's own browser-based ChatGPT sign-in. Credentials remain owned
/// by Codex; this process is deliberately detached and never exposes output
/// or tokens to FPV.
pub fn start_login() -> AppResult<()> {
    let mut cmd = Command::new(executable());
    cmd.arg("login")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().map_err(|err| {
        AppError::Config(format!(
            "Codex CLI is unavailable; install Codex before signing in: {err}"
        ))
    })?;
    tokio::spawn(async move {
        let _ = child.wait().await;
    });
    Ok(())
}

async fn send(input: &mut tokio::process::ChildStdin, value: Value) -> AppResult<()> {
    let line = serde_json::to_string(&value)?;
    input.write_all(line.as_bytes()).await?;
    input.write_all(b"\n").await?;
    input.flush().await?;
    Ok(())
}

async fn next_message(
    lines: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
) -> AppResult<Value> {
    let line = lines
        .next_line()
        .await?
        .ok_or_else(|| AppError::Other("Codex app-server closed unexpectedly".into()))?;
    if line.len() > MAX_MESSAGE_BYTES {
        return Err(AppError::Other(
            "Codex app-server sent an oversized message".into(),
        ));
    }
    serde_json::from_str(&line).map_err(AppError::from)
}

async fn response_for(
    lines: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    id: u64,
) -> AppResult<Value> {
    loop {
        let message = next_message(lines).await?;
        if message.get("id").and_then(Value::as_u64) != Some(id) {
            continue;
        }
        if let Some(error) = message.get("error") {
            return Err(AppError::Other(format!(
                "Codex app-server error: {}",
                error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("request failed")
            )));
        }
        return message.get("result").cloned().ok_or_else(|| {
            AppError::Other("Codex app-server returned an invalid response".into())
        });
    }
}

/// Generate text in an isolated Codex thread. `cancel` is request-scoped and
/// terminates the owned process after asking the app-server to interrupt.
pub async fn narrate<F>(
    model: &str,
    prompt: &str,
    cancel: &mut tokio::sync::broadcast::Receiver<String>,
    request_id: &str,
    mut on_delta: F,
) -> AppResult<String>
where
    F: FnMut(&str),
{
    let model = model
        .strip_prefix("codex:")
        .ok_or_else(|| AppError::Config("malformed Codex model spec".into()))?;
    let mut cmd = Command::new(executable());
    cmd.arg("app-server")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().map_err(|err| {
        AppError::Config(format!(
            "Codex CLI is unavailable; install it and run `codex login`: {err}"
        ))
    })?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| AppError::Other("Codex stdin unavailable".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AppError::Other("Codex stdout unavailable".into()))?;
    let mut lines = BufReader::new(stdout).lines();

    let result = async {
        send(&mut stdin, json!({"method":"initialize","id":1,"params":{"clientInfo":{"name":"fpv_desktop","title":"FPV","version":env!("CARGO_PKG_VERSION")}}})).await?;
        response_for(&mut lines, 1).await?;
        send(&mut stdin, json!({"method":"initialized","params":{}})).await?;
        send(
            &mut stdin,
            // Do not let a narration request inherit FPV's working directory.
            json!({"method":"thread/start","id":2,"params":{"model":model,"cwd":std::env::temp_dir()}}),
        )
        .await?;
        let thread = response_for(&mut lines, 2).await?;
        let thread_id = thread
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Other("Codex app-server did not return a thread id".into()))?
            .to_owned();
        send(&mut stdin, json!({"method":"turn/start","id":3,"params":{"threadId":thread_id,"input":[{"type":"text","text":prompt}]}})).await?;
        let turn = response_for(&mut lines, 3).await?;
        let turn_id = turn
            .pointer("/turn/id")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Other("Codex app-server did not return a turn id".into()))?
            .to_owned();
        let mut output = String::new();
        loop {
            tokio::select! {
                message = next_message(&mut lines) => {
                    let message = message?;
                    match message.get("method").and_then(Value::as_str) {
                        Some("item/agentMessage/delta") => {
                            if let Some(delta) = message.pointer("/params/delta").and_then(Value::as_str) {
                                output.push_str(delta);
                                on_delta(delta);
                            }
                        }
                        Some("turn/completed") => {
                            let completed_id = message.pointer("/params/turn/id").and_then(Value::as_str);
                            if completed_id == Some(turn_id.as_str()) {
                                let status = message.pointer("/params/turn/status").and_then(Value::as_str).unwrap_or("failed");
                                if status != "completed" { return Err(AppError::Other(format!("Codex turn {status}"))); }
                                if output.trim().is_empty() { return Err(AppError::Other("Codex returned an empty response".into())); }
                                return Ok(output);
                            }
                        }
                        _ => {}
                    }
                }
                cancelled = cancel.recv() => {
                    if cancelled.ok().as_deref() == Some(request_id) {
                        let _ = send(&mut stdin, json!({"method":"turn/interrupt","id":4,"params":{"threadId":thread_id,"turnId":turn_id}})).await;
                        return Err(AppError::Other("Narration cancelled".into()));
                    }
                }
            }
        }
    };
    let result = tokio::time::timeout(TURN_TIMEOUT, result)
        .await
        .map_err(|_| AppError::Other("Codex narration timed out".into()))?;
    let _ = child.kill().await;
    result
}

#[cfg(test)]
mod tests {
    use super::is_codex_model;

    #[test]
    fn accepts_only_nonempty_codex_specs() {
        assert!(is_codex_model("codex:gpt-5.6-terra"));
        assert!(!is_codex_model("codex:"));
        assert!(!is_codex_model("openai:gpt-5.6-terra"));
    }
}
