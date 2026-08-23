#[cfg(not(windows))]
use std::collections::{HashMap, HashSet};
use std::process::Command;
use std::time::Duration;

use tauri::{AppHandle, Manager};
#[cfg(not(windows))]
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
#[cfg(not(windows))]
use tauri_plugin_shell::ShellExt;
use tokio::time::sleep;
#[cfg(not(windows))]
use tracing::trace;
use tracing::{info, warn};

use crate::db;
use crate::error::{AppError, AppResult};
use crate::inference::OllamaClient;
use crate::state::AppState;

/// Handle to the running Ollama child process.
///
/// - macOS (and any other non-Windows target): a Tauri shell-plugin
///   sidecar (`CommandChild`), launched via `.sidecar("ollama")`.
/// - Windows: a plain `std::process::Child` launched directly from the
///   bundled, self-contained Ollama runtime folder (see
///   `spawn_bundled_windows`) so the GPU-runner payload sitting beside
///   `ollama.exe` is discovered exactly like a normal Ollama install.
enum SidecarChild {
    #[cfg(not(windows))]
    Tauri(CommandChild),
    #[cfg(windows)]
    Std(std::process::Child),
}

pub struct OllamaSidecar {
    child: SidecarChild,
}

impl OllamaSidecar {
    pub async fn kill(self, client: &OllamaClient) -> AppResult<()> {
        let parent_pid = match &self.child {
            #[cfg(not(windows))]
            SidecarChild::Tauri(c) => c.pid(),
            #[cfg(windows)]
            SidecarChild::Std(c) => c.id(),
        };
        match tokio::time::timeout(Duration::from_secs(2), client.unload_all()).await {
            Ok(Ok(())) => {}
            Ok(Err(err)) => warn!(?err, "could not unload Ollama models before shutdown"),
            Err(_) => warn!("timed out unloading Ollama models before shutdown"),
        }

        // Killing only `ollama serve` can leave its llama-server children
        // re-parented to launchd. Snapshot and terminate the complete tree.
        #[cfg(not(windows))]
        {
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
        }

        // Windows has no /bin/ps + /bin/kill. `taskkill /T /F` terminates
        // the whole process tree rooted at ollama.exe (llama-server runners
        // included) in one call — they cannot survive re-parented the way
        // they can on macOS. The Job Object additionally reaps anything
        // left after an abnormal app exit.
        #[cfg(windows)]
        {
            let _ = Command::new("taskkill")
                .args(["/PID", &parent_pid.to_string(), "/T", "/F"])
                .status();
        }

        // Consume the child handle. ESRCH is expected when SIGTERM already
        // won (macOS path); on Windows `Child::kill` + `wait` reap directly.
        match self.child {
            #[cfg(not(windows))]
            SidecarChild::Tauri(c) => {
                let _ = c.kill();
            }
            #[cfg(windows)]
            SidecarChild::Std(mut c) => {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
#[cfg(not(windows))]
struct ProcessRow {
    pid: u32,
    ppid: u32,
    command: String,
}

#[cfg(not(windows))]
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

#[cfg(not(windows))]
fn descendant_pids(parent: u32) -> Vec<u32> {
    descendant_pids_from_rows(parent, process_rows())
}

#[cfg(not(windows))]
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

#[cfg(not(windows))]
fn is_orphaned_fpv_process(row: &ProcessRow) -> bool {
    row.ppid == 1
        && (row.command.contains("/FPV.app/Contents/MacOS/llama-server")
            || (row.command.contains("/FPV.app/Contents/MacOS/ollama")
                && row.command.ends_with(" serve")))
}

#[cfg(not(windows))]
fn signal_processes(pids: impl IntoIterator<Item = u32>, signal: &str) {
    let unique: HashSet<u32> = pids.into_iter().collect();
    for pid in unique {
        let _ = Command::new("/bin/kill")
            .args([signal, &pid.to_string()])
            .status();
    }
}

#[cfg(not(windows))]
fn process_alive(pid: u32) -> bool {
    Command::new("/bin/kill")
        .args(["-0", &pid.to_string()])
        .status()
        .is_ok_and(|status| status.success())
}

/// Remove only orphaned daemons/runners from FPV bundles. A user's system
/// Ollama and processes attached to another live FPV tree are untouched.
#[cfg(not(windows))]
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

#[cfg(all(test, not(windows)))]
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

    // Windows cannot use Tauri's `.sidecar("ollama")` externalBin mechanism
    // the way macOS does — instead the whole Ollama runtime directory
    // (exe + GPU-runner libraries) is bundled as a Tauri resource and
    // launched directly. See `spawn_bundled_windows` for the full
    // rationale (CREATE_NO_WINDOW, pinned OLLAMA_HOST, etc).
    #[cfg(windows)]
    {
        // Dev fallback: `tauri dev` does NOT assemble bundled resources, so
        // the self-contained runtime (`resource_dir()/ollama-runtime`) only
        // exists in a built installer. When it's absent, try a
        // system-installed Ollama instead so local Windows development
        // still has a working chat path. Production installers always
        // carry the bundled runtime, so this branch is a no-op in shipped
        // builds.
        if !bundled_runtime_present(app) {
            if let Some(sys) = find_system_ollama() {
                use std::os::windows::process::CommandExt;
                info!(path = %sys, "bundled ollama runtime absent (dev); starting system Ollama");
                let spawned = std::process::Command::new(&sys)
                    .arg("serve")
                    .env("OLLAMA_HOST", "127.0.0.1:11434")
                    .creation_flags(CREATE_NO_WINDOW)
                    .spawn();
                match spawned {
                    Ok(child) => {
                        // Same as the production `spawn_bundled_windows` path:
                        // assign to the process-wide Job Object so a crashed
                        // `cargo tauri dev` session doesn't leave this system
                        // Ollama instance orphaned. Dropping the `Child` handle
                        // right after (this fn returns `Ok(None)` on success,
                        // owning nothing) is safe — on Windows dropping `Child`
                        // only closes our handle, it does not kill the process;
                        // the Job Object is what ensures cleanup on app exit.
                        {
                            use std::os::windows::io::AsRawHandle;
                            crate::sidecars::job_object::adopt(
                                child.as_raw_handle(),
                                "ollama-dev-system",
                            );
                        }
                        if wait_for_ready(client, Duration::from_secs(20))
                            .await
                            .is_ok()
                        {
                            info!("system ollama ready, using it instead of the bundled runtime");
                            return Ok(None);
                        }
                        warn!(
                            "system ollama did not become ready; falling through to bundled runtime"
                        );
                    }
                    Err(error) => {
                        warn!(
                            ?error,
                            "failed to spawn system ollama; falling through to bundled runtime"
                        );
                    }
                }
            }
        }

        let prefs = read_runtime_prefs(app).await;
        let sidecar = spawn_bundled_windows(app, &prefs).await?;
        // First boot may need to load GPU runners before the HTTP server
        // answers, so give it a generous window.
        if let Err(error) = wait_for_ready(client, Duration::from_secs(90)).await {
            if let Err(kill_error) = sidecar.kill(client).await {
                warn!(?kill_error, "could not kill unready Ollama sidecar");
            }
            return Err(error);
        }
        Ok(Some(sidecar))
    }

    #[cfg(not(windows))]
    {
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
///
/// macOS/Unix-only: checks Homebrew paths and a bare `ollama` on `PATH`,
/// neither of which is how a system Ollama presents on Windows (that's
/// `find_system_ollama`, used by `migrate_legacy_models` there instead).
#[cfg(not(windows))]
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
    // Don't touch a user-owned Ollama's model store. `system_ollama_present`
    // is the macOS/Unix detector (checks Homebrew paths + a bare `ollama`
    // on PATH); on Windows the binary is `ollama.exe` and the install
    // locations differ, so `find_system_ollama` (already used by the dev
    // fallback in `ensure()`) is the correct check there.
    #[cfg(windows)]
    let has_system_ollama = find_system_ollama().is_some();
    #[cfg(not(windows))]
    let has_system_ollama = system_ollama_present();
    if has_system_ollama {
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

#[cfg(not(windows))]
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

    Ok(OllamaSidecar {
        child: SidecarChild::Tauri(child),
    })
}

/// Pure builder for the env vars `spawn_bundled_windows` sets on the
/// bundled `ollama.exe` process. Extracted so the "OLLAMA_HOST is always
/// pinned to loopback, never inherited from the user's environment" and
/// "OLLAMA_LIBRARY_PATH points at runtime_dir/lib/ollama" invariants are
/// testable without a real spawn (Windows-only spawning can't run here on
/// this macOS host, but the values that WOULD be passed to `Command` can
/// be verified on any host).
///
/// Mirrors `spawn_bundled` (macOS)'s tiering exactly — `OLLAMA_NUM_GPU` /
/// `OLLAMA_KEEP_ALIVE` / `OLLAMA_MAX_LOADED_MODELS` / `OLLAMA_NUM_PARALLEL`
/// were missing here entirely until this was added: `spawn_bundled_windows`
/// never read `RuntimePrefs` at all, so the Settings → Performance
/// Device/Low Power/Semantic Memory toggles had ZERO effect on the actual
/// Windows Ollama process — every user's choice was silently ignored.
/// `OLLAMA_MODELS` was missing too, so the bundled runtime fell back to
/// Ollama's own default store (`%USERPROFILE%\.ollama\models`) instead of
/// the app-scoped, uninstall-cleaned directory `models_dir` points at —
/// the exact problem the doc comment on `storage::ollama_models_dir` says
/// this mechanism exists to prevent. Local Waifu's own Windows fork sets
/// all of these in its equivalent spawn; this was a real behavioral gap
/// versus that reference, not just a missing feature.
#[cfg(windows)]
fn ollama_env_vars(
    runtime_dir: &std::path::Path,
    prefs: &RuntimePrefs,
    models_dir: &std::path::Path,
) -> Vec<(&'static str, std::ffi::OsString)> {
    // OLLAMA_NUM_GPU=0 forces CPU-only inference. Anything else (or unset)
    // lets Ollama discover CUDA/ROCm automatically.
    let num_gpu = if prefs.device == "cpu" { "0" } else { "999" };
    let keep_alive = keep_alive_for(prefs.low_power, prefs.ram_gb);
    // Single-user app — one chat in flight at a time, same as macOS.
    let num_parallel = "1";
    let max_loaded = if prefs.semantic_memory && !prefs.low_power && prefs.ram_gb > 16 {
        "2"
    } else {
        "1"
    };
    vec![
        (
            "OLLAMA_HOST",
            std::ffi::OsString::from("127.0.0.1:11434"),
        ),
        (
            "OLLAMA_LIBRARY_PATH",
            runtime_dir.join("lib").join("ollama").into_os_string(),
        ),
        ("OLLAMA_MODELS", models_dir.as_os_str().to_owned()),
        ("OLLAMA_KEEP_ALIVE", std::ffi::OsString::from(keep_alive)),
        (
            "OLLAMA_MAX_LOADED_MODELS",
            std::ffi::OsString::from(max_loaded),
        ),
        (
            "OLLAMA_NUM_PARALLEL",
            std::ffi::OsString::from(num_parallel),
        ),
        ("OLLAMA_NUM_GPU", std::ffi::OsString::from(num_gpu)),
        // Parity with the macOS spawn_bundled path: no cloud telemetry,
        // flash attention, and a q8_0 KV cache.
        ("OLLAMA_NO_CLOUD", std::ffi::OsString::from("1")),
        ("OLLAMA_FLASH_ATTENTION", std::ffi::OsString::from("1")),
        ("OLLAMA_KV_CACHE_TYPE", std::ffi::OsString::from("q8_0")),
    ]
}

/// CREATE_NO_WINDOW process-creation flag. Suppresses the console window
/// Windows would otherwise attach to a spawned console binary. Shared by
/// every raw `std::process::Command` spawn in this file.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Whether the self-contained bundled Ollama runtime resource is present.
/// True in installed builds; false under `tauri dev`, which does not
/// assemble bundled resources — see the dev-fallback comment in `ensure`.
#[cfg(windows)]
fn bundled_runtime_present(app: &AppHandle) -> bool {
    app.path()
        .resource_dir()
        .map(|r| r.join("ollama-runtime").join("ollama.exe").exists())
        .unwrap_or(false)
}

/// Locate a system-installed Ollama for the dev fallback. Returns the
/// command/path to invoke, or `None` if nothing is found.
#[cfg(windows)]
fn find_system_ollama() -> Option<String> {
    use std::os::windows::process::CommandExt;
    use std::path::PathBuf;

    // 1. On PATH (CREATE_NO_WINDOW so the probe doesn't flash a console).
    if std::process::Command::new("where.exe")
        .arg("ollama.exe")
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return Some("ollama".to_string());
    }

    // 2. Common install locations (official Ollama installer).
    let home = dirs::home_dir()?;
    let candidates = [
        home.join("AppData")
            .join("Local")
            .join("Programs")
            .join("Ollama")
            .join("ollama.exe"),
        PathBuf::from(r"C:\Program Files\Ollama\ollama.exe"),
    ];
    for c in &candidates {
        if c.exists() {
            return Some(c.to_string_lossy().to_string());
        }
    }
    None
}

/// Windows: run the bundled, self-contained Ollama runtime.
///
/// `scripts/fetch-binaries-win.ps1` drops the entire Ollama payload
/// (`ollama.exe` + `lib/ollama/<GPU runners>`) into
/// `src-tauri/ollama-runtime/`, and `tauri.windows.conf.json` bundles
/// that folder verbatim as a resource. We launch `ollama.exe` directly
/// from the resolved resource folder with the working directory set
/// there, so Ollama finds its `lib/ollama` runner payload exactly like a
/// normal install — deterministically, with no separate download, no
/// PATH lookup, and no dependency on where Tauri would otherwise place a
/// sidecar exe.
///
/// `OLLAMA_HOST` is pinned to `127.0.0.1:11434` explicitly rather than
/// left to inherit from the environment: a user who runs their own
/// Ollama for LAN access commonly exports `OLLAMA_HOST=0.0.0.0` (or a
/// non-default port) at the user level, which would otherwise either
/// bind our private server to all interfaces (surprise firewall prompt +
/// the model API exposed on the LAN) or bind a port `wait_for_ready`
/// never checks (permanently "Offline" chat).
#[cfg(windows)]
async fn spawn_bundled_windows(app: &AppHandle, prefs: &RuntimePrefs) -> AppResult<OllamaSidecar> {
    use std::os::windows::process::CommandExt;

    // Keep the bundled Ollama's model store inside the app's data dir, same
    // as macOS — see `storage::ollama_models_dir`'s doc comment. Migrate any
    // pre-existing legacy store (either from a build before this fix, which
    // fell back to Ollama's own default location, or a plain `~/.ollama`
    // left by a prior system install) before starting the server so the
    // moved blobs are in place when it scans the dir.
    migrate_legacy_models();
    let models_dir = crate::storage::ollama_models_dir()
        .map_err(|e| AppError::Sidecar(format!("ollama models dir: {e}")))?;

    // Prefer a fully provisioned, vendor-matched GPU runtime (fetched by
    // `sidecars::ollama_gpu_runtime` on first launch or a user-triggered
    // retry) over the bundled one. It is a COMPLETE Ollama runtime — not
    // just extra runner DLLs — so using it wholesale as both the exe and
    // `OLLAMA_LIBRARY_PATH` source keeps the launch self-consistent, the
    // same shape `image::sdcpp::sd_cli_path()` already follows for the
    // image-generation GPU build. Falls through to the bundled resource
    // dir when nothing has been provisioned, or provisioning is
    // incomplete/stale — never worse than the bundled runtime.
    let runtime_dir = match crate::sidecars::ollama_gpu_runtime::provisioned_runtime_dir() {
        Some(dir) => dir,
        None => {
            let resource_dir = app
                .path()
                .resource_dir()
                .map_err(|e| AppError::Sidecar(format!("resolve resource_dir: {e}")))?;
            resource_dir.join("ollama-runtime")
        }
    };
    let exe = runtime_dir.join("ollama.exe");
    if !exe.exists() {
        return Err(AppError::Sidecar(format!(
            "ollama runtime missing at {}",
            exe.display()
        )));
    }

    let mut cmd = Command::new(&exe);
    cmd.arg("serve").current_dir(&runtime_dir);
    for (key, value) in ollama_env_vars(&runtime_dir, prefs, &models_dir) {
        cmd.env(key, value);
    }
    // CREATE_NO_WINDOW: no flashing/lingering console window when a GUI
    // app spawns a console binary.
    cmd.creation_flags(CREATE_NO_WINDOW);

    let child = cmd
        .spawn()
        .map_err(|e| AppError::Sidecar(format!("spawn bundled ollama: {e}")))?;

    // Assign the freshly-spawned process to the process-wide Job Object
    // so Windows kills it (and any GPU-runner children it spawns) if we
    // die without running our own shutdown — see sidecars/job_object.rs.
    // Best-effort: a failure here just means we're back to relying on
    // SidecarManager::shutdown_all for graceful exits.
    {
        use std::os::windows::io::AsRawHandle;
        crate::sidecars::job_object::adopt(child.as_raw_handle(), "ollama");
    }

    info!(dir = %runtime_dir.display(), "spawned bundled ollama runtime");
    Ok(OllamaSidecar {
        child: SidecarChild::Std(child),
    })
}

/// Restart the app-owned Ollama sidecar so it picks up a freshly
/// provisioned GPU runtime immediately, instead of only on the next
/// launch. Called by `sidecars::ollama_gpu_runtime` after a successful
/// download/extract, and by its `reprovision` fast path when the runtime
/// was already on disk but never got applied.
///
/// Fails (rather than silently no-op'ing) when there is no app-owned
/// sidecar to restart — i.e. the user is on an externally managed Ollama
/// instance reused via `ensure()`'s ping check. That is not an error in
/// GPU provisioning itself (the download/extract already succeeded and
/// the flag is already stamped); the caller distinguishes "nothing to
/// restart" from a real failure and logs accordingly rather than
/// reporting "GPU setup failed" to a user running their own Ollama.
#[cfg(windows)]
pub async fn restart_for_gpu(app: &AppHandle, client: &OllamaClient) -> AppResult<()> {
    let Some(state) = app.try_state::<AppState>() else {
        return Err(AppError::Sidecar(
            "app state not ready for ollama restart".into(),
        ));
    };

    // Drop the sidecar we own (if any) so the port frees up. A reused
    // externally-managed Ollama (Ok(None) at boot) leaves nothing here —
    // in that case we can't safely restart someone else's process.
    let owned = state.sidecars.ollama.lock().await.take();
    let Some(old) = owned else {
        return Err(AppError::Sidecar(
            "no app-owned ollama sidecar to restart (external instance in use)".into(),
        ));
    };
    let _ = old.kill(client).await;

    // Wait up to 5 s for the HTTP port to stop answering so the respawn
    // binds cleanly instead of re-attaching to the exiting process.
    for _ in 0..20 {
        if !client.ping().await.unwrap_or(false) {
            break;
        }
        sleep(Duration::from_millis(250)).await;
    }

    match ensure(app, client).await {
        Ok(Some(sc)) => {
            state.sidecars.ollama.lock().await.replace(sc);
            info!("ollama restarted on provisioned GPU runtime");
            Ok(())
        }
        // `ensure()` reused a now-ready instance (unlikely right after a
        // kill, but not impossible) — nothing to store, runtime is up.
        Ok(None) => Ok(()),
        Err(e) => Err(e),
    }
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

// `ollama_env_vars` itself is `#[cfg(windows)]` (it builds a
// `Vec<(&str, OsString)>` that only spawn_bundled_windows consumes), so
// this test module is windows-only too — it never runs on the macOS CI
// host or this dev machine, only on Windows CI (Task 12). It is still
// worth having: it is a REAL assertion against the function that builds
// the env vars, not a documentation placeholder, so a future edit that
// drops the OLLAMA_HOST pin or points OLLAMA_LIBRARY_PATH at the wrong
// directory fails this test the moment it runs on Windows CI.
#[cfg(all(test, windows))]
mod windows_tests {
    use super::{ollama_env_vars, RuntimePrefs};
    use std::path::Path;

    fn find(vars: &[(&'static str, std::ffi::OsString)], key: &str) -> Option<String> {
        vars.iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.to_string_lossy().into_owned())
    }

    fn default_prefs() -> RuntimePrefs {
        RuntimePrefs {
            device: "auto".into(),
            low_power: false,
            semantic_memory: false,
            ram_gb: 32,
        }
    }

    #[test]
    fn ollama_cloud_flash_and_kv_cache_match_macos_defaults() {
        // Parity guard: the macOS spawn_bundled sets these three; the
        // Windows bundled spawn must set the same values so behavior
        // does not drift between platforms.
        let vars = ollama_env_vars(Path::new("C:/rt"), &default_prefs(), Path::new("C:/m"));
        assert_eq!(find(&vars, "OLLAMA_NO_CLOUD").as_deref(), Some("1"));
        assert_eq!(find(&vars, "OLLAMA_FLASH_ATTENTION").as_deref(), Some("1"));
        assert_eq!(find(&vars, "OLLAMA_KV_CACHE_TYPE").as_deref(), Some("q8_0"));
    }

    #[test]
    fn ollama_host_is_pinned_to_loopback_not_inherited() {
        // Regression guard for hard rule (LW #77): a user's own
        // OLLAMA_HOST env var must never leak into the bundled spawn —
        // ollama_env_vars always returns the pinned value, it never
        // reads std::env.
        let vars = ollama_env_vars(
            Path::new(r"C:\fake\ollama-runtime"),
            &default_prefs(),
            Path::new(r"C:\fake\app-data\ollama\models"),
        );
        assert_eq!(find(&vars, "OLLAMA_HOST").as_deref(), Some("127.0.0.1:11434"));
    }

    #[test]
    fn library_path_points_at_runtime_dir_lib_ollama() {
        let runtime_dir = Path::new(r"C:\fake\ollama-runtime");
        let vars = ollama_env_vars(
            runtime_dir,
            &default_prefs(),
            Path::new(r"C:\fake\app-data\ollama\models"),
        );
        assert_eq!(
            find(&vars, "OLLAMA_LIBRARY_PATH").as_deref(),
            Some(r"C:\fake\ollama-runtime\lib\ollama")
        );
    }

    #[test]
    fn models_dir_is_passed_through_verbatim() {
        // Regression guard for the gap this test module was extended to
        // catch: `spawn_bundled_windows` never read `OLLAMA_MODELS` at
        // all before this, so the bundled runtime silently fell back to
        // Ollama's own default store instead of the app-scoped,
        // uninstall-cleaned directory.
        let vars = ollama_env_vars(
            Path::new(r"C:\fake\ollama-runtime"),
            &default_prefs(),
            Path::new(r"C:\fake\app-data\ollama\models"),
        );
        assert_eq!(
            find(&vars, "OLLAMA_MODELS").as_deref(),
            Some(r"C:\fake\app-data\ollama\models")
        );
    }

    #[test]
    fn device_cpu_forces_num_gpu_zero() {
        // Regression guard: the Settings -> Performance "CPU only" toggle
        // had zero effect on Windows before this — spawn_bundled_windows
        // never read RuntimePrefs.device at all.
        let mut prefs = default_prefs();
        prefs.device = "cpu".into();
        let vars = ollama_env_vars(
            Path::new(r"C:\fake\ollama-runtime"),
            &prefs,
            Path::new(r"C:\fake\models"),
        );
        assert_eq!(find(&vars, "OLLAMA_NUM_GPU").as_deref(), Some("0"));

        let vars = ollama_env_vars(
            Path::new(r"C:\fake\ollama-runtime"),
            &default_prefs(),
            Path::new(r"C:\fake\models"),
        );
        assert_eq!(find(&vars, "OLLAMA_NUM_GPU").as_deref(), Some("999"));
    }

    #[test]
    fn low_power_forces_short_keep_alive() {
        let mut prefs = default_prefs();
        prefs.low_power = true;
        let vars = ollama_env_vars(
            Path::new(r"C:\fake\ollama-runtime"),
            &prefs,
            Path::new(r"C:\fake\models"),
        );
        assert_eq!(find(&vars, "OLLAMA_KEEP_ALIVE").as_deref(), Some("1m"));
    }

    #[test]
    fn max_loaded_models_matches_semantic_memory_tiering() {
        // Mirrors spawn_bundled (macOS)'s identical rule: 2 resident
        // models only when semantic memory is on, not low-power, and RAM
        // has headroom (> 16 GB) — otherwise 1, so chat/embed thrash on
        // every turn instead of risking memory pressure.
        let mut prefs = default_prefs();
        prefs.semantic_memory = true;
        prefs.ram_gb = 32;
        let vars = ollama_env_vars(
            Path::new(r"C:\fake\ollama-runtime"),
            &prefs,
            Path::new(r"C:\fake\models"),
        );
        assert_eq!(find(&vars, "OLLAMA_MAX_LOADED_MODELS").as_deref(), Some("2"));

        prefs.ram_gb = 16;
        let vars = ollama_env_vars(
            Path::new(r"C:\fake\ollama-runtime"),
            &prefs,
            Path::new(r"C:\fake\models"),
        );
        assert_eq!(find(&vars, "OLLAMA_MAX_LOADED_MODELS").as_deref(), Some("1"));
    }
}
