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

/// The directories whose contents are ours: the directory the app binary
/// runs from (`Contents/MacOS` in a packaged `.app`, `target/<profile>`
/// under `tauri dev`) and the app-data dir. Nothing else on the machine
/// launches binaries out of either, which is what makes a path match
/// proof of ownership.
///
/// Matching the DIRECTORY rather than a hardcoded `/FPV.app/` plus a
/// list of binary names is what LW's `sidecars::reaper` does, and the
/// reason is not tidiness: the old name whitelist covered `ollama` and
/// `llama-server` only, so an `sd-cli` orphaned mid-render was never
/// swept and kept a GPU busy until the user noticed it in Activity
/// Monitor. The literal `/FPV.app/` also stopped matching the moment a
/// user renamed the app, and never matched at all in a dev build.
fn owned_roots() -> Vec<std::path::PathBuf> {
    let mut roots = Vec::new();
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf))
    {
        roots.push(dir);
    }
    if let Ok(dir) = crate::storage::app_data_dir() {
        roots.push(dir);
    }
    roots
}

/// Is this command line running something out of `root`?
fn is_under(command: &str, root: &std::path::Path) -> bool {
    let Some(root) = root.to_str() else {
        return false;
    };
    command.starts_with(&format!("{root}/"))
}

/// Is `exe` the binary this command line runs? Compares the whole path
/// rather than a bare prefix so a neighbour whose name merely starts
/// with ours is not mistaken for it.
fn is_exe(command: &str, exe: &std::path::Path) -> bool {
    let Some(path) = exe.to_str() else {
        return false;
    };
    command == path || command.starts_with(&format!("{path} "))
}

/// A process left behind by a previous run of this app.
///
/// Two rules keep it safe. `ppid == 1` means anything belonging to a
/// *running* instance (which has that instance, or its server, as a
/// parent) is never a candidate — only something already reparented to
/// launchd is. And `self_exe` is excluded because the app binary lives
/// in the swept directory and a normally-launched instance also has
/// `ppid == 1`: without that, the sweep would kill a second running
/// copy of the app, and on the way there, itself.
fn is_orphaned_owned_process(
    row: &ProcessRow,
    roots: &[std::path::PathBuf],
    self_exe: Option<&std::path::Path>,
) -> bool {
    row.ppid == 1
        && roots.iter().any(|root| is_under(&row.command, root))
        && !self_exe.is_some_and(|exe| is_exe(&row.command, exe))
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

/// Every process of ours left behind by a previous run, plus everything
/// they in turn spawned.
///
/// Descendants are included because an orphaned Ollama *server* still
/// owns live runners: killing it alone would reparent those to launchd,
/// which is the exact bug this sweep exists to stop.
fn orphan_pids(
    rows: &[ProcessRow],
    roots: &[std::path::PathBuf],
    self_exe: Option<&std::path::Path>,
) -> Vec<u32> {
    let mut found: Vec<u32> = rows
        .iter()
        .filter(|row| is_orphaned_owned_process(row, roots, self_exe))
        .map(|row| row.pid)
        .collect();

    let mut cursor = 0;
    while cursor < found.len() {
        let parent = found[cursor];
        cursor += 1;
        for row in rows.iter().filter(|row| row.ppid == parent) {
            if !found.contains(&row.pid) {
                found.push(row.pid);
            }
        }
    }
    found
}

/// Remove only orphaned processes launched out of this app's own
/// directories — the Ollama daemon, its `llama-server` runners, and an
/// `sd-cli` left grinding through a render nobody will collect. A
/// user's system Ollama and processes attached to another live FPV tree
/// are untouched.
pub fn cleanup_orphaned_runners() {
    let roots = owned_roots();
    if roots.is_empty() {
        return;
    }
    let self_exe = std::env::current_exe().ok();
    let stale = orphan_pids(&process_rows(), &roots, self_exe.as_deref());
    if !stale.is_empty() {
        warn!(
            count = stale.len(),
            "terminating orphaned FPV sidecar processes"
        );
        signal_processes(stale.iter().copied(), "-TERM");
        std::thread::sleep(Duration::from_millis(300));
        signal_processes(stale.into_iter().filter(|pid| process_alive(*pid)), "-KILL");
    }
}

#[cfg(test)]
mod process_tests {
    use super::{descendant_pids_from_rows, orphan_pids, ProcessRow};
    use std::path::{Path, PathBuf};

    const BUNDLE: &str = "/Applications/FPV.app/Contents/MacOS";

    fn roots() -> Vec<PathBuf> {
        vec![PathBuf::from(BUNDLE)]
    }

    fn app_exe() -> PathBuf {
        PathBuf::from(BUNDLE).join("fpv-desktop")
    }

    fn orphans(rows: Vec<ProcessRow>) -> Vec<u32> {
        let exe = app_exe();
        let mut found = orphan_pids(&rows, &roots(), Some(exe.as_path() as &Path));
        found.sort_unstable();
        found
    }

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
    fn orphan_cleanup_is_restricted_to_processes_from_our_own_directories() {
        assert_eq!(
            orphans(vec![
                row(20, 1, "/Applications/FPV.app/Contents/MacOS/ollama serve"),
                row(
                    21,
                    1,
                    "/Applications/FPV.app/Contents/MacOS/llama-server --port 50000"
                ),
                row(22, 1, "/opt/homebrew/bin/ollama serve"),
                row(23, 7, "/Applications/FPV.app/Contents/MacOS/llama-server"),
            ]),
            vec![20, 21],
            "a user's own Ollama and a runner attached to a live tree must survive"
        );
    }

    #[test]
    fn an_sd_cli_left_mid_render_is_swept() {
        // The whole reason this matcher works on directories instead of a
        // list of binary names: sd-cli holds the GPU and nothing else was
        // ever going to kill it.
        assert_eq!(
            orphans(vec![row(
                30,
                1,
                "/Applications/FPV.app/Contents/MacOS/sd-cli --model x --steps 30"
            )]),
            vec![30]
        );
    }

    #[test]
    fn the_sweep_does_not_kill_another_running_copy_of_the_app() {
        // A normally-launched instance also has ppid == 1 and lives in the
        // swept directory. Without the self-exe exclusion this sweep would
        // take it down — and, on the way, itself.
        assert!(orphans(vec![
            row(40, 1, "/Applications/FPV.app/Contents/MacOS/fpv-desktop"),
            row(
                41,
                1,
                "/Applications/FPV.app/Contents/MacOS/fpv-desktop --flag"
            ),
        ])
        .is_empty());
    }

    #[test]
    fn a_neighbour_whose_name_starts_with_ours_is_not_mistaken_for_the_app() {
        assert_eq!(
            orphans(vec![row(
                50,
                1,
                "/Applications/FPV.app/Contents/MacOS/fpv-desktop-helper"
            )]),
            vec![50]
        );
    }

    #[test]
    fn children_of_an_orphaned_server_are_swept_with_it() {
        // Killing an orphaned `ollama serve` on its own would reparent its
        // runners to launchd — the exact bug the sweep exists to stop.
        assert_eq!(
            orphans(vec![
                row(60, 1, "/Applications/FPV.app/Contents/MacOS/ollama serve"),
                row(61, 60, "/Applications/FPV.app/Contents/MacOS/llama-server"),
                row(62, 61, "worker"),
            ]),
            vec![60, 61, 62]
        );
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
