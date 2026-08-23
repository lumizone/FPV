use std::collections::{HashMap, HashSet};
use std::process::Command;
use std::time::Duration;

use tauri::{AppHandle, Manager};
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;
use tokio::time::sleep;
use tracing::{info, trace, warn};

use crate::db;
use crate::error::{AppError, AppResult};
use crate::inference::OllamaClient;
use crate::state::AppState;

pub struct OllamaSidecar {
    child: CommandChild,
}

impl OllamaSidecar {
    pub async fn kill(self, client: &OllamaClient) -> AppResult<()> {
        let parent_pid = self.child.pid();
        match tokio::time::timeout(Duration::from_secs(2), client.unload_all()).await {
            Ok(Ok(())) => {}
            Ok(Err(err)) => warn!(?err, "could not unload Ollama models before shutdown"),
            Err(_) => warn!("timed out unloading Ollama models before shutdown"),
        }

        // Killing only `ollama serve` can leave its llama-server children
        // re-parented to launchd. Snapshot and terminate the complete tree.
        let descendants = descendant_pids(parent_pid);
        signal_processes(
            descendants
                .iter()
                .copied()
                .chain(std::iter::once(parent_pid)),
            "-TERM",
        );
        sleep(Duration::from_millis(800)).await;

        let survivors = descendants
            .into_iter()
            .chain(std::iter::once(parent_pid))
            .filter(|pid| process_alive(*pid));
        signal_processes(survivors, "-KILL");

        // Consume CommandChild. ESRCH is expected when SIGTERM already won.
        let _ = self.child.kill();
        Ok(())
    }
}

#[derive(Debug)]
struct ProcessRow {
    pid: u32,
    ppid: u32,
    command: String,
}

fn process_rows() -> Vec<ProcessRow> {
    let Ok(output) = Command::new("/bin/ps")
        .args(["-axo", "pid=,ppid=,command="])
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            Some(ProcessRow {
                pid: fields.next()?.parse().ok()?,
                ppid: fields.next()?.parse().ok()?,
                command: fields.collect::<Vec<_>>().join(" "),
            })
        })
        .collect()
}

fn descendant_pids(parent: u32) -> Vec<u32> {
    descendant_pids_from_rows(parent, process_rows())
}

fn descendant_pids_from_rows(parent: u32, rows: Vec<ProcessRow>) -> Vec<u32> {
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for row in rows {
        children.entry(row.ppid).or_default().push(row.pid);
    }
    let mut pending = vec![parent];
    let mut found = Vec::new();
    while let Some(pid) = pending.pop() {
        if let Some(direct) = children.get(&pid) {
            for child in direct {
                found.push(*child);
                pending.push(*child);
            }
        }
    }
    found
}

fn is_orphaned_fpv_process(row: &ProcessRow) -> bool {
    row.ppid == 1
        && (row.command.contains("/FPV.app/Contents/MacOS/llama-server")
            || (row.command.contains("/FPV.app/Contents/MacOS/ollama")
                && row.command.ends_with(" serve")))
}

fn signal_processes(pids: impl IntoIterator<Item = u32>, signal: &str) {
    let unique: HashSet<u32> = pids.into_iter().collect();
    for pid in unique {
        let _ = Command::new("/bin/kill")
            .args([signal, &pid.to_string()])
            .status();
    }
}

fn process_alive(pid: u32) -> bool {
    Command::new("/bin/kill")
        .args(["-0", &pid.to_string()])
        .status()
        .is_ok_and(|status| status.success())
}

/// Remove only orphaned daemons/runners from FPV bundles. A user's system
/// Ollama and processes attached to another live FPV tree are untouched.
pub fn cleanup_orphaned_runners() {
    let stale: Vec<u32> = process_rows()
        .into_iter()
        .filter(is_orphaned_fpv_process)
        .map(|row| row.pid)
        .collect();
    if !stale.is_empty() {
        warn!(
            count = stale.len(),
            "terminating orphaned FPV model runners"
        );
        signal_processes(stale.iter().copied(), "-TERM");
        std::thread::sleep(Duration::from_millis(300));
        signal_processes(stale.into_iter().filter(|pid| process_alive(*pid)), "-KILL");
    }
}

#[cfg(test)]
mod process_tests {
    use super::{descendant_pids_from_rows, is_orphaned_fpv_process, ProcessRow};

    fn row(pid: u32, ppid: u32, command: &str) -> ProcessRow {
        ProcessRow {
            pid,
            ppid,
            command: command.into(),
        }
    }

    #[test]
    fn finds_complete_process_tree_without_unrelated_processes() {
        let mut found = descendant_pids_from_rows(
            10,
            vec![
                row(11, 10, "ollama serve"),
                row(12, 11, "llama-server"),
                row(13, 12, "worker"),
                row(99, 1, "unrelated"),
            ],
        );
        found.sort_unstable();
        assert_eq!(found, vec![11, 12, 13]);
    }

    #[test]
    fn orphan_cleanup_is_restricted_to_fpv_bundle_processes() {
        assert!(is_orphaned_fpv_process(&row(
            20,
            1,
            "/Applications/FPV.app/Contents/MacOS/ollama serve"
        )));
        assert!(is_orphaned_fpv_process(&row(
            21,
            1,
            "/Applications/FPV.app/Contents/MacOS/llama-server --port 50000"
        )));
        assert!(!is_orphaned_fpv_process(&row(
            22,
            1,
            "/opt/homebrew/bin/ollama serve"
        )));
        assert!(!is_orphaned_fpv_process(&row(
            23,
            7,
            "/Applications/FPV.app/Contents/MacOS/llama-server"
        )));
    }
}

/// Try to use a running Ollama (system-installed or already spawned).
/// If none is reachable, spawn the bundled sidecar.
pub async fn ensure(app: &AppHandle, client: &OllamaClient) -> AppResult<Option<OllamaSidecar>> {
    if client.ping().await.unwrap_or(false) {
        info!(
            url = client.base_url(),
            "ollama already running, using existing instance"
        );
        return Ok(None);
    }

    info!("ollama not reachable, spawning bundled sidecar");
    let prefs = read_runtime_prefs(app).await;
    let sidecar = spawn_bundled(app, &prefs).await?;
    if let Err(error) = wait_for_ready(client, Duration::from_secs(30)).await {
        if let Err(kill_error) = sidecar.kill(client).await {
            warn!(?kill_error, "could not kill unready Ollama sidecar");
        }
        return Err(error);
    }
    Ok(Some(sidecar))
}

#[derive(Debug, Default)]
struct RuntimePrefs {
    /// `auto` (default) | `cpu` | `metal` | `neural`. Controls
    /// `OLLAMA_NUM_GPU` — 0 forces CPU, anything else lets Ollama
    /// pick (which is Metal on Apple Silicon).
    device: String,
    /// When true, drop Ollama keep_alive from 5 min to 1 min and
    /// disable parallelism. Battery-aware throttling.
    low_power: bool,
    semantic_memory: bool,
    /// Physical RAM (GB) from hardware detection. Decides whether we let
    /// Ollama keep two models resident (chat + embed) — see
    /// `OLLAMA_MAX_LOADED_MODELS` below.
    ram_gb: u64,
}

async fn read_runtime_prefs(app: &AppHandle) -> RuntimePrefs {
    // Best-effort — if state isn't installed yet (very early boot)
    // or DB lookup fails, defaults are fine.
    let Some(state) = app.try_state::<AppState>() else {
        return RuntimePrefs::default();
    };
    let conn = state.db.lock().await;
    let device = db::meta_get(&conn, "device")
        .ok()
        .flatten()
        .unwrap_or_else(|| "auto".into());
    let low_power = db::meta_get(&conn, "lowPowerMode")
        .ok()
        .flatten()
        .map(|v| v == "true")
        .unwrap_or(false);
    let semantic_memory = db::meta_get(&conn, "semanticMemoryEnabled")
        .ok()
        .flatten()
        .map(|v| v == "true")
        .unwrap_or(false);
    RuntimePrefs {
        device,
        low_power,
        semantic_memory,
        ram_gb: state.hardware.ram_gb,
    }
}

/// True when a user-installed, system-wide Ollama appears to be present
/// (a CLI binary in a common install location or anywhere on `PATH`).
/// Our bundled sidecar is invoked by absolute path and is NOT on `PATH`,
/// so a hit here means the user runs their own Ollama — and the
/// `~/.ollama/models` store is theirs, not ours.
fn system_ollama_present() -> bool {
    // Homebrew (Apple Silicon + Intel) and the official manual installer.
    const FIXED: &[&str] = &[
        "/opt/homebrew/bin/ollama",
        "/usr/local/bin/ollama",
        "/usr/bin/ollama",
    ];
    if FIXED.iter().any(|p| std::path::Path::new(p).exists()) {
        return true;
    }
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            if dir.join("ollama").exists() {
                return true;
            }
        }
    }
    false
}

/// One-time move of a pre-existing `~/.ollama/models` store into the
/// app-owned models dir, so users who pulled models before the
/// `OLLAMA_MODELS` relocation don't have to re-download. Best-effort:
/// any failure is logged and leaves the legacy files in place (Ollama
/// would then simply re-pull on demand). Self-gating — once the app dir
/// has a `blobs` folder it never runs again.
///
/// Skipped entirely when a system-wide Ollama is detected: in that case
/// `~/.ollama/models` belongs to the user's own install and moving it
/// would empty their `ollama list`. We leave it untouched and let our
/// bundled sidecar populate its own (app-data) store on demand.
fn migrate_legacy_models() {
    let Ok(new_dir) = crate::storage::ollama_models_dir() else {
        return;
    };
    // Already populated/migrated — nothing to do.
    if new_dir.join("blobs").exists() {
        return;
    }
    let Some(home) = dirs::home_dir() else { return };
    let legacy = home.join(".ollama").join("models");
    if !legacy.join("blobs").exists() {
        return;
    }
    // Don't touch a user-owned Ollama's model store.
    if system_ollama_present() {
        info!("system-wide Ollama detected — leaving ~/.ollama/models in place (no migration)");
        return;
    }
    let mut moved = false;
    for sub in ["blobs", "manifests"] {
        let from = legacy.join(sub);
        let to = new_dir.join(sub);
        if from.exists() && !to.exists() {
            match std::fs::rename(&from, &to) {
                Ok(()) => moved = true,
                Err(e) => warn!(
                    ?e,
                    sub, "ollama model migration: rename failed; left in place"
                ),
            }
        }
    }
    if moved {
        info!("migrated legacy ~/.ollama/models into app data dir");
    }
}

async fn spawn_bundled(app: &AppHandle, prefs: &RuntimePrefs) -> AppResult<OllamaSidecar> {
    // OLLAMA_NUM_GPU=0 forces CPU-only inference. Anything else (or
    // unset) lets Ollama discover Metal automatically. Apple Silicon
    // has no separate "Neural Engine" path in Ollama, so `neural`
    // falls back to the same Metal default.
    let num_gpu = if prefs.device == "cpu" { "0" } else { "999" };
    // Keep the model resident long enough that mid-session pauses don't
    // pay the multi-second cold-reload on the next message (an 18 GB
    // gemma4:26b load is ~20 s measured). Low-power mode still unloads
    // quickly to save battery/RAM.
    let keep_alive = keep_alive_for(prefs.low_power, prefs.ram_gb);
    // Single-user app — one chat in flight at a time. Higher values
    // would let Ollama swap the model out from memory pressure mid-turn
    // on Light tier, so we cap at 1 regardless of low-power mode.
    let num_parallel = "1";

    // How many models Ollama keeps resident at once. The chat pipeline
    // touches TWO models every turn — the embed model (embeddinggemma,
    // ~0.62 GB) for recall, then the chat model. With a cap of 1, each turn evicts
    // the other model and reloads it from disk (embed → reload chat →
    // next turn reload embed …) — a multi-hundred-ms to multi-second
    // thrash on every message. Allowing 2 keeps both resident so only
    // the first turn pays the load cost. The embed model is tiny, so the
    // extra resident footprint is negligible; we still cap at 1 on
    // low-RAM (≤ 16 GB) or low-power machines where headroom is tight
    // and a large chat model + KV cache already pressures memory.
    let max_loaded = if prefs.semantic_memory && !prefs.low_power && prefs.ram_gb > 16 {
        "2"
    } else {
        "1"
    };

    // Keep the bundled Ollama's model store inside the app's data dir
    // (not the default ~/.ollama/models) so models are removed together
    // with the app and don't collide with a system-wide Ollama. Run the
    // one-time migration of any legacy ~/.ollama/models BEFORE starting
    // the server so the moved blobs are in place when it scans the dir.
    migrate_legacy_models();
    let models_dir = crate::storage::ollama_models_dir()
        .map_err(|e| AppError::Sidecar(format!("ollama models dir: {e}")))?;

    let cmd = app
        .shell()
        .sidecar("ollama")
        .map_err(|e| AppError::Sidecar(format!("resolve sidecar: {e}")))?
        .args(["serve"])
        // Do not inherit host/cloud settings from the launch environment.
        .env("OLLAMA_HOST", "127.0.0.1:11434")
        .env("OLLAMA_NO_CLOUD", "1")
        .env("OLLAMA_MODELS", &models_dir)
        .env("OLLAMA_KEEP_ALIVE", keep_alive)
        .env("OLLAMA_MAX_LOADED_MODELS", max_loaded)
        .env("OLLAMA_NUM_PARALLEL", num_parallel)
        .env("OLLAMA_NUM_GPU", num_gpu)
        .env("OLLAMA_FLASH_ATTENTION", "1")
        .env("OLLAMA_KV_CACHE_TYPE", "q8_0");

    let (mut rx, child) = cmd
        .spawn()
        .map_err(|e| AppError::Sidecar(format!("spawn ollama: {e}")))?;

    tauri::async_runtime::spawn(async move {
        while let Some(event) = rx.recv().await {
            match event {
                CommandEvent::Stdout(line) => {
                    let s = String::from_utf8_lossy(&line);
                    trace!(target: "ollama.stdout", "{}", s.trim_end());
                }
                CommandEvent::Stderr(line) => {
                    let s = String::from_utf8_lossy(&line);
                    trace!(target: "ollama.stderr", "{}", s.trim_end());
                }
                CommandEvent::Terminated(payload) => {
                    warn!(code = ?payload.code, "ollama sidecar terminated");
                    break;
                }
                _ => {}
            }
        }
    });

    Ok(OllamaSidecar { child })
}

/// How long Ollama keeps an idle model resident. Low-power unloads fast
/// (battery/RAM first). On big-RAM machines (≥ 32 GB) the model is the
/// app's whole point, but retaining multi-gigabyte weights for hours after a
/// short session creates avoidable memory pressure. Fifteen minutes covers a
/// normal pause while returning resources promptly.
fn keep_alive_for(low_power: bool, _ram_gb: u64) -> &'static str {
    if low_power {
        "1m"
    } else {
        "15m"
    }
}

async fn wait_for_ready(client: &OllamaClient, timeout: Duration) -> AppResult<()> {
    let started = std::time::Instant::now();
    while started.elapsed() < timeout {
        if client.ping().await.unwrap_or(false) {
            info!("ollama sidecar ready");
            return Ok(());
        }
        sleep(Duration::from_millis(500)).await;
    }
    Err(AppError::Sidecar(format!(
        "ollama sidecar did not become ready within {:?}",
        timeout
    )))
}

#[cfg(test)]
mod keep_alive_tests {
    use super::keep_alive_for;

    #[test]
    fn keep_alive_tiers() {
        assert_eq!(keep_alive_for(true, 64), "1m"); // low power always wins
        assert_eq!(keep_alive_for(false, 36), "15m");
        assert_eq!(keep_alive_for(false, 32), "15m");
        assert_eq!(keep_alive_for(false, 16), "15m");
        assert_eq!(keep_alive_for(false, 24), "15m");
    }
}
