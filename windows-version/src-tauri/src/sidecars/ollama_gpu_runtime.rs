//! First-launch GPU-runner provisioning for the bundled Ollama on Windows.
//!
//! WHY: the NSIS installer can only carry ~2 GB of UNCOMPRESSED payload
//! (32-bit makensis limit — see `scripts/fetch-binaries-win.ps1`), so FPV
//! bundles a small base: `ollama.exe` + the CPU runner + a single CUDA line
//! (`-KeepCuda "v12"`). That covers CPU everywhere and most current NVIDIA
//! cards (Turing→Ada), but:
//!   - Blackwell (RTX 50-series / new datacenter GPUs) needs the `cuda_v13`
//!     runner, which is pruned out of the bundle → those boxes fall back to
//!     CPU ("works, but very slowly"), and
//!   - AMD Radeon needs the ROCm runner, entirely absent from the standard
//!     zip.
//!
//! This module fixes both WITHOUT bloating the installer: on first launch it
//! detects the GPU vendor and, if needed, downloads the matching FULL Ollama
//! runtime (same pinned version as the bundled exe) into the user's app-data
//! dir and extracts it there. `sidecars::ollama::spawn_bundled_windows` then
//! prefers this complete runtime over the bundled one, launching `ollama.exe`
//! from it with `OLLAMA_LIBRARY_PATH` pointed at its `lib/ollama`.
//!
//! This is a DIFFERENT GPU-runtime concern from `sidecars::gpu_runtime`,
//! which provisions the accelerated stable-diffusion.cpp build for local
//! image generation — that module's sentinel is a file inside the
//! provisioned directory; this one's is an app-meta DB flag, matching how
//! Local Waifu's own reference module (of the same name, one directory
//! over) does it. `GpuVendor` detection is SHARED with that module
//! (`crate::sidecars::gpu_runtime::{GpuVendor, detect_vendor}`) rather than
//! duplicated — the GPU vendor cannot change at runtime, so one memoised
//! probe serves both subsystems.
//!
//! Properties:
//!   - **Fail-safe**: any failure (no network, extract error, unknown GPU)
//!     leaves the bundled CPU + cuda_v12 runners in place — the app keeps
//!     working, just without the extra GPU coverage.
//!   - **Run-once** per (vendor, Ollama version) via an app_meta flag.
//!   - **Applies immediately, not just next launch**: on success, the
//!     app-owned Ollama sidecar (if any) is restarted onto the freshly
//!     provisioned runtime — see `sidecars::ollama::restart_for_gpu`.
//!   - **Windows-only at runtime**: the whole module is `#[cfg(windows)]`
//!     in `sidecars/mod.rs`, so it is not compiled at all on macOS.
//!
//! Adapted from Local Waifu's Windows fork (`sidecars/gpu_runtime.rs`,
//! read-only reference) — that module has no character/voice/licence
//! concerns of its own, so this is close to a direct port, with FPV's own
//! module/type names and its `SidecarManager`/`AppState` shapes.

use std::path::{Path, PathBuf};

use tauri::{AppHandle, Emitter, Manager};
use tracing::{info, warn};

use crate::db::{self, DbHandle};
use crate::error::{AppError, AppResult};
use crate::sidecars::gpu_runtime::{detect_vendor, GpuVendor};

/// Pinned to the bundled Ollama version (`scripts/fetch-binaries-win.ps1`
/// `$OllamaVersion`). The downloaded runner set MUST match the bundled
/// `ollama.exe` ABI, so keep these in lockstep.
const OLLAMA_VERSION: &str = "v0.30.8";

/// app_meta flag recording which (vendor, version) has been provisioned, so
/// a launch doesn't re-download what a previous one already fetched.
const FLAG_KEY: &str = "ollama_gpu_runtime_provisioned";

/// Serializes the whole provisioning pass. The boot trigger (`spawn`) and a
/// user-visible "Retry GPU setup" button (`reprovision`) both write the SAME
/// fixed paths (`download.zip`, `_extract/`), and `reprovision`'s own
/// "runtime already on disk, just restart" fast path reads/acts on that same
/// directory. Without a shared lock, a double-click or a boot+retry overlap
/// has one pass touching files the other is mid-write/delete on.
static PROVISION_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Sub-directory of the app-data dir holding the downloaded runner set.
/// `ollama.exe` + `lib/ollama/<runners>` live directly under here — the
/// FULL runtime, not just the extra runner, so it is self-consistent and
/// can be launched in place exactly like the bundled one.
fn gpu_runtime_dir() -> AppResult<PathBuf> {
    Ok(crate::storage::app_data_dir()?.join("ollama-gpu"))
}

/// The downloaded runtime dir, but ONLY if it is complete AND actually adds
/// a GPU runner the bundle lacks (`cuda_v13` for Blackwell NVIDIA, or an AMD
/// ROCm/HIP runner). `sidecars::ollama::spawn_bundled_windows` launches
/// `ollama.exe` from THIS dir when it returns `Some`. Conservative by
/// design: a half-extracted set returns `None` and the bundled CPU +
/// cuda_v12 runtime stays in charge — never worse than shipped.
pub fn provisioned_runtime_dir() -> Option<PathBuf> {
    let dir = gpu_runtime_dir().ok()?;
    if !runtime_root_is_complete(&dir) {
        return None;
    }
    Some(dir)
}

/// Run the provisioning pass in the background. Fire and forget: spawned
/// once at app startup, logs and emits on its own.
pub fn spawn(app: AppHandle, db: DbHandle) {
    tauri::async_runtime::spawn(async move {
        if let Err(err) = run(&app, &db).await {
            warn!(
                ?err,
                "ollama GPU runtime provisioning failed (non-fatal; CPU + bundled CUDA still active)"
            );
        }
    });
}

/// Force a re-evaluation of GPU provisioning on demand — the backend half
/// of a "Retry GPU setup" UI action.
///
/// Clears the run-once flag so detection re-runs, then either:
///   - a complete GPU runtime is already on disk — just restart Ollama onto
///     it (covers "download finished but the live restart didn't take"), or
///   - otherwise runs the full detect → download → restart pass.
///
/// Fail-safe: any error leaves the bundled CPU/cuda_v12 runner serving, so a
/// retry can never leave the user worse off than before pressing it.
pub async fn reprovision(app: &AppHandle, db: &DbHandle) -> AppResult<()> {
    // Retry must honor the same provider gate as boot provisioning. A cloud
    // narration selection has no reason to download or restart Ollama.
    if !local_narration_selected(db).await {
        info!("ollama GPU runtime: retry skipped; cloud narration provider is selected");
        return Ok(());
    }
    {
        let conn = db.lock().await;
        // Empty value never equals a real "{vendor}:{version}" flag, so
        // `run()` below won't early-skip as "already provisioned".
        let _ = db::meta_set(&conn, FLAG_KEY, "");
    }

    // Same lock `run()` takes — this fast path reads/acts on the exact
    // directory `run()` writes, so it must not run concurrently with a
    // boot-time provisioning pass either.
    let _guard = PROVISION_LOCK.lock().await;

    if provisioned_runtime_dir().is_some() {
        if let Some(state) = app.try_state::<crate::state::AppState>() {
            let client = state.ollama.clone();
            match crate::sidecars::ollama::restart_for_gpu(app, &client).await {
                Ok(()) => {}
                // The user is on an externally-managed Ollama (not FPV's
                // bundled sidecar) — GPU handling is on that install, not
                // FPV, so there is nothing to restart. Not a failure of
                // this button; surfacing it as a hard error would tell an
                // external-Ollama user "GPU setup failed" for nothing.
                Err(e) => {
                    info!(
                        ?e,
                        "ollama GPU runtime: nothing to restart (external Ollama in use); runtime still applies next launch"
                    );
                }
            }
        }
        let vendor = detect_vendor();
        stamp(db, &format!("{}:{OLLAMA_VERSION}", vendor.tag())).await?;
        info!("ollama GPU runtime: re-applied existing provisioned runtime on request");
        return Ok(());
    }
    drop(_guard);

    run(app, db).await
}

/// Notify the UI of a provisioning phase so it can show a toast. Same
/// channel and phase vocabulary (`downloading` / `ready` / `error`) as
/// `sidecars::gpu_runtime`'s image-GPU provisioning — both subsystems share
/// one generic "setting up GPU acceleration" toast in the frontend.
fn emit(app: &AppHandle, phase: &str) {
    let _ = app.emit("gpu-setup", phase);
}

async fn run(app: &AppHandle, db: &DbHandle) -> AppResult<()> {
    let _guard = PROVISION_LOCK.lock().await;

    let vendor = detect_vendor();
    info!(vendor = vendor.tag(), "ollama GPU runtime: detected vendor");

    // Never provision Ollama for a cloud-only narration setup. This check is
    // inside the locked run as well as in the retry entry point because boot
    // and user-triggered paths must share identical spend semantics.
    if !local_narration_selected(db).await {
        info!("ollama GPU runtime: skipped; cloud narration provider is selected");
        return Ok(());
    }

    // Decide whether a download is actually needed:
    //   - AMD: always — the installer bundles no AMD runner at all.
    //   - NVIDIA: only when the card is too new for the bundled `cuda_v12`
    //     (covers up to Hopper / compute 9.0). Blackwell (RTX 50-series =
    //     compute 12.0, datacenter B = 10.0) needs `cuda_v13`. Turing→Ada
    //     users already get GPU accel from the bundle and must NOT sit
    //     through a ~1 GB download for nothing. Unreadable capability →
    //     fetch to be safe (covers an unknown-newer case).
    //   - Intel/virtual/none: never — no usable Windows GPU runner exists.
    let need_fetch = match vendor {
        GpuVendor::Amd => true,
        GpuVendor::Nvidia => nvidia_needs_runtime_fetch(),
        GpuVendor::Other => false,
    };
    if !need_fetch {
        info!("ollama GPU runtime: bundled runners already cover this GPU; no download needed");
        return Ok(());
    }

    let want_flag = format!("{}:{OLLAMA_VERSION}", vendor.tag());

    // A stale flag is only a hint. Validate the on-disk runtime first so a
    // crash after flagging (or a manually removed/partial install) repairs
    // itself instead of bypassing provisioning forever.
    if provisioned_runtime_dir().is_some() {
        let conn = db.lock().await;
        if db::meta_get(&conn, FLAG_KEY)?.as_deref() == Some(&want_flag) {
            return Ok(());
        }
        drop(conn);
        stamp(db, &want_flag).await?;
        info!("ollama GPU runtime: already present; stamped flag");
        return Ok(());
    }

    // Flag lost or stale while the files are incomplete: continue through
    // download/extraction rather than treating app_meta as proof of readiness.
    {
        let conn = db.lock().await;
        if db::meta_get(&conn, FLAG_KEY)?.as_deref() == Some(&want_flag) {
            info!("ollama GPU runtime: stale provisioned flag; repairing incomplete runtime");
        }
    }

    let url = download_url(vendor);
    info!(%url, "ollama GPU runtime: downloading matching Ollama runtime");
    emit(app, "downloading");

    // Every failure path inside this block — not only the download/extract
    // one — must emit "error": `emit(app, "downloading")` above already
    // told the frontend an attempt started, and without a matching failure
    // signal the toast has no way to know the attempt gave up rather than
    // succeeded silently.
    let expected_hash = expected_sha256(vendor);
    let provision_result: AppResult<()> = async {
        let dir = gpu_runtime_dir()?;
        recover_orphaned_runtime(&dir);
        let tmp = dir.join("download.zip");
        if let Some(parent) = tmp.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Download + extract, then ALWAYS remove the ~1 GB zip, including
        // on the error path — a failed attempt must not leave a partial
        // multi-hundred-MB file for the next one to inherit or ignore.
        let inner_result = async {
            download_to_file(&url, &tmp).await?;
            // Verify BEFORE extraction — the `ollama.exe` this produces
            // gets EXECUTED, so a corrupted or tampered download must
            // never reach that point.
            let tmp_for_hash = tmp.clone();
            tokio::task::spawn_blocking(move || verify_sha256(&tmp_for_hash, expected_hash))
                .await
                .map_err(|e| AppError::Other(format!("hash verification task: {e}")))??;
            extract_runtime(&tmp, &dir).await?;
            Ok::<(), AppError>(())
        }
        .await;
        let _ = std::fs::remove_file(&tmp);
        inner_result?;

        if provisioned_runtime_dir().is_none() {
            return Err(AppError::Other(
                "ollama GPU runtime: extraction completed but no usable GPU runtime found".into(),
            ));
        }

        stamp(db, &want_flag).await?;
        Ok(())
    }
    .await;
    if let Err(e) = provision_result {
        emit(app, "error");
        return Err(e);
    }

    emit(app, "ready");
    info!("ollama GPU runtime: provisioned; applying now by restarting the Ollama runtime");

    // Apply THIS session instead of only on the next launch.
    if let Some(state) = app.try_state::<crate::state::AppState>() {
        let client = state.ollama.clone();
        match crate::sidecars::ollama::restart_for_gpu(app, &client).await {
            Ok(()) => info!("ollama GPU runtime: Ollama restarted; GPU acceleration is now live"),
            Err(err) => warn!(
                ?err,
                "ollama GPU runtime provisioned but live restart failed; takes effect on next launch"
            ),
        }
    } else {
        info!("ollama GPU runtime: app state unavailable; GPU acceleration applies on next launch");
    }
    Ok(())
}

async fn local_narration_selected(db: &DbHandle) -> bool {
    let conn = db.lock().await;
    db::meta_get(&conn, "user_chat_model")
        .ok()
        .flatten()
        .map(|model| {
            let cloud = model
                .split_once(':')
                .and_then(|(provider, _)| {
                    crate::inference::cloud::CloudProvider::from_str(provider)
                })
                .is_some();
            !cloud && !crate::inference::codex::is_codex_model(&model)
        })
        .unwrap_or(true)
}

async fn stamp(db: &DbHandle, value: &str) -> AppResult<()> {
    let conn = db.lock().await;
    db::meta_set(&conn, FLAG_KEY, value)
}

/// Map vendor → the Ollama release asset carrying the matching runners. The
/// standard amd64 zip ships CPU + cuda_v12 + cuda_v13 (covers every current
/// NVIDIA including Blackwell); the `-rocm` asset adds the AMD ROCm runner.
fn download_url(vendor: GpuVendor) -> String {
    format!(
        "https://github.com/ollama/ollama/releases/download/{OLLAMA_VERSION}/{}",
        release_asset(vendor)
    )
}

fn release_asset(vendor: GpuVendor) -> &'static str {
    match vendor {
        GpuVendor::Amd => "ollama-windows-amd64-rocm.zip",
        _ => "ollama-windows-amd64.zip",
    }
}

/// SHA-256 of each release asset for `OLLAMA_VERSION`, pinned so the
/// downloaded zip — whose `ollama.exe` this app then EXECUTES — can't be
/// silently corrupted or swapped in transit before extraction.
///
/// Independently re-derived (not copied from Local Waifu) on 2026-08-18:
/// downloaded `https://github.com/ollama/ollama/releases/download/v0.30.8/
/// sha256sum.txt` live and read both `ollama-windows-amd64.zip` and
/// `ollama-windows-amd64-rocm.zip` lines directly. Both values match Local
/// Waifu's own pin for the same release, as expected — same upstream asset,
/// same hash — but this app's copy was checked against the source, not
/// against that app. Re-verify the same way whenever `OLLAMA_VERSION`
/// changes, or every download will fail its integrity check and GPU
/// provisioning silently falls back to the bundled CPU/cuda_v12 runner
/// (fail-safe, not fail-open).
fn expected_sha256(vendor: GpuVendor) -> &'static str {
    match vendor {
        GpuVendor::Amd => "5d4cbc12fc43927692e18c96d0829dd815cce926122ddd84044a0bf9fcf6df29",
        _ => "c2d26d97e698027329c252629d7113bbc05d874b49960cbb03e93a39ae9fd95c",
    }
}

/// Verify `path`'s SHA-256 against the pinned hash for this release asset.
/// Streamed (not read-to-memory) since these zips run ~0.7-1.5 GB.
fn verify_sha256(path: &Path, expected_hex: &str) -> AppResult<()> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if digest.eq_ignore_ascii_case(expected_hex) {
        Ok(())
    } else {
        Err(AppError::Other(format!(
            "ollama GPU runtime download failed integrity check (expected {expected_hex}, got {digest})"
        )))
    }
}

/// Stream a URL to `dest`. 1-hour timeout — these runner zips are large
/// (~0.7-1.5 GB) on home connections, same order of magnitude as a model
/// pull.
async fn download_to_file(url: &str, dest: &Path) -> AppResult<()> {
    use futures_util::StreamExt;
    use std::io::Write;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60 * 60))
        .build()?;
    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        return Err(AppError::Other(format!(
            "ollama GPU runtime download {}: {url}",
            resp.status()
        )));
    }
    let mut file = std::fs::File::create(dest)?;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk)?;
    }
    file.flush()?;
    Ok(())
}

/// Extract the archive with PowerShell's built-in `Expand-Archive` (avoids a
/// zip crate dependency) and place the FULL Ollama runtime (`ollama.exe` +
/// `lib/ollama/<runners>`) directly under `dest`. The whole runtime is
/// moved, not just `lib/ollama`, so the result is self-consistent and can be
/// launched in place — Ollama finds its runners no matter how it probes for
/// them. The zip contains everything in one directory (possibly nested one
/// level); the real root is located by the `ollama.exe` + `lib/ollama`
/// marker.
///
/// The commit is an ATOMIC swap: stage in a SIBLING directory (never inside
/// `dest`), rename `dest` -> `.ollama-gpu-old`, then rename the extracted
/// root -> `dest`. A crash between the two renames leaves `dest` missing but
/// the previous complete runtime intact under `.ollama-gpu-old`, which
/// `recover_orphaned_runtime()` restores on the next fetch — the old code
/// cleared `dest` in place and could strand a half-populated runtime that
/// shadowed the bundled one.
async fn extract_runtime(zip: &Path, dest: &Path) -> AppResult<()> {
    let parent = dest
        .parent()
        .ok_or_else(|| AppError::Other("ollama GPU runtime: dest has no parent".into()))?;
    let staging = parent.join(".ollama-gpu-extract");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;

    let zip_s = zip.to_string_lossy().to_string();
    let staging_s = staging.to_string_lossy().to_string();
    let cmd = format!(
        "Expand-Archive -LiteralPath '{}' -DestinationPath '{}' -Force",
        zip_s.replace('\'', "''"),
        staging_s.replace('\'', "''"),
    );
    if !run_powershell(&cmd)? {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(AppError::Other(
            "ollama GPU runtime: Expand-Archive failed".into(),
        ));
    }

    let root = find_runtime_root(&staging).ok_or_else(|| {
        AppError::Other("ollama GPU runtime: ollama.exe + lib/ollama not found in archive".into())
    })?;

    // Swap: keep the previous good runtime as `.old` until the new one is
    // fully in place.
    let backup = parent.join(".ollama-gpu-old");
    let _ = std::fs::remove_dir_all(&backup);
    let dest_existed = dest.exists();
    if dest_existed {
        if let Err(err) = std::fs::rename(dest, &backup) {
            // Busy (open handle) — most likely a running ollama.exe using
            // this exact directory as its runtime. Never fall back to
            // deleting files inside a LIVE directory here: a partial
            // delete (exe gone, DLLs locked, or vice versa) leaves a
            // runtime that still passes the "looks provisioned" check but
            // cannot actually launch — exactly the failure mode
            // `sidecars::gpu_runtime`'s atomic swap was written to avoid.
            // Fail clean instead: the existing runtime is left exactly as
            // it was, busy or not, and the caller's fail-safe design keeps
            // using it (or the bundled CPU build) until the next fetch.
            let _ = std::fs::remove_dir_all(&staging);
            return Err(AppError::Other(format!(
                "ollama GPU runtime: existing runtime is in use, could not stage the new one: {err}"
            )));
        }
    }
    if std::fs::rename(&root, dest).is_err() {
        // Cross-device or busy — populate by recursive copy instead.
        std::fs::create_dir_all(dest)?;
        for entry in std::fs::read_dir(&root)? {
            let entry = entry?;
            let from = entry.path();
            let to = dest.join(entry.file_name());
            if from.is_dir() {
                copy_dir_recursive(&from, &to)?;
            } else {
                std::fs::copy(&from, &to)?;
            }
        }
    }
    let _ = std::fs::remove_dir_all(&backup);
    let _ = std::fs::remove_dir_all(&staging);
    Ok(())
}

/// Restore a runtime orphaned by a crash between the swap steps in
/// `extract_runtime` (dest missing, `.ollama-gpu-old` intact). Called at the
/// start of every fetch.
fn recover_orphaned_runtime(dest: &Path) {
    let Some(parent) = dest.parent() else {
        return;
    };
    let backup = parent.join(".ollama-gpu-old");
    if !dest.exists() && backup.exists() {
        let _ = std::fs::rename(&backup, dest);
    }
}

/// Require the executable, runner directory, and at least one non-empty GPU
/// upgrade runner. Marker directories alone are not enough: interrupted
/// extraction can leave a partial `cuda_v13`/ROCm directory behind.
fn runtime_root_is_complete(root: &Path) -> bool {
    let lib = root.join("lib").join("ollama");
    if !root.join("ollama.exe").is_file() || !lib.is_dir() {
        return false;
    }
    std::fs::read_dir(lib)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .any(|entry| {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            let upgrade = name.contains("cuda_v13")
                || name.contains("rocm")
                || name.contains("hip")
                || name.starts_with("vulkan");
            upgrade
                && entry.path().is_dir()
                && std::fs::read_dir(entry.path())
                    .map(|mut files| files.next().is_some())
                    .unwrap_or(false)
        })
}

/// Locate the directory containing a complete runtime under `root` (the
/// archive may nest everything one folder deep).
fn find_runtime_root(root: &Path) -> Option<PathBuf> {
    let ok = |d: &Path| runtime_root_is_complete(d);
    if ok(root) {
        return Some(root.to_path_buf());
    }
    if let Ok(entries) = std::fs::read_dir(root) {
        for e in entries.flatten() {
            if e.path().is_dir() && ok(&e.path()) {
                return Some(e.path());
            }
        }
    }
    None
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> AppResult<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let target = dst.join(entry.file_name());
        if path.is_dir() {
            copy_dir_recursive(&path, &target)?;
        } else {
            std::fs::copy(&path, &target)?;
        }
    }
    Ok(())
}

/// Whether an NVIDIA box needs the downloaded runtime: true when the
/// highest GPU compute capability exceeds what the bundled `cuda_v12`
/// runner supports (Hopper / 9.0). Blackwell (10.0 / 12.0) needs the newer
/// runner. Unknown → true (fetch to be safe).
pub fn nvidia_needs_runtime_fetch() -> bool {
    match nvidia_max_compute_cap() {
        Some(cap) => cap >= 10.0,
        None => true,
    }
}

/// Highest CUDA compute capability across installed NVIDIA GPUs, via
/// `nvidia-smi --query-gpu=compute_cap` (e.g. "8.9", "12.0"). `None` if
/// `nvidia-smi` is absent / unparseable.
fn nvidia_max_compute_cap() -> Option<f32> {
    let out = command_output(
        "nvidia-smi",
        &["--query-gpu=compute_cap", "--format=csv,noheader"],
    )?;
    out.lines()
        .filter_map(|l| l.trim().parse::<f32>().ok())
        .fold(None, |acc: Option<f32>, v| {
            Some(acc.map_or(v, |a| a.max(v)))
        })
}

fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let mut cmd = std::process::Command::new(program);
    cmd.args(args);
    hide_window(&mut cmd);
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn run_powershell(command: &str) -> AppResult<bool> {
    let mut cmd = std::process::Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", command]);
    hide_window(&mut cmd);
    let status = cmd
        .status()
        .map_err(|e| AppError::Other(format!("powershell: {e}")))?;
    Ok(status.success())
}

/// The app is a GUI-subsystem binary with no console, so a raw spawn
/// FLASHES a console window unless this flag is set. No `#[cfg(not(windows))]`
/// fallback needed — this whole module is `#[cfg(windows)] pub mod` in
/// `sidecars/mod.rs`, so it never compiles on macOS.
fn hide_window(cmd: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_url_picks_rocm_for_amd() {
        assert!(download_url(GpuVendor::Amd).ends_with("ollama-windows-amd64-rocm.zip"));
        assert!(download_url(GpuVendor::Nvidia).ends_with("ollama-windows-amd64.zip"));
        assert!(download_url(GpuVendor::Other).ends_with("ollama-windows-amd64.zip"));
    }

    #[test]
    fn expected_sha256_is_well_formed_for_every_vendor() {
        for v in [GpuVendor::Nvidia, GpuVendor::Amd, GpuVendor::Other] {
            let h = expected_sha256(v);
            assert_eq!(h.len(), 64, "sha256 hex must be 64 chars for {v:?}");
            assert!(
                h.chars().all(|c| c.is_ascii_hexdigit()),
                "sha256 must be pure hex for {v:?}: {h}"
            );
        }
    }

    #[test]
    fn verify_sha256_accepts_matching_and_rejects_mismatched() {
        use sha2::{Digest, Sha256};

        let dir = std::env::temp_dir().join(format!("fpv_test_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("payload.bin");
        std::fs::write(&f, b"hello world").unwrap();

        let mut hasher = Sha256::new();
        hasher.update(b"hello world");
        let real: String = hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();

        assert!(verify_sha256(&f, &real).is_ok());
        assert!(verify_sha256(
            &f,
            "0000000000000000000000000000000000000000000000000000000000000"
        )
        .is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nvidia_and_amd_pins_are_the_live_verified_v0_30_8_hashes() {
        // Regression guard: both hashes were independently re-derived
        // from Ollama's live sha256sum.txt for v0.30.8 on 2026-08-18, not
        // copied from Local Waifu — pin the exact values so a future edit
        // can't silently drift from what was actually checked.
        assert_eq!(
            expected_sha256(GpuVendor::Nvidia),
            "c2d26d97e698027329c252629d7113bbc05d874b49960cbb03e93a39ae9fd95c"
        );
        assert_eq!(
            expected_sha256(GpuVendor::Amd),
            "5d4cbc12fc43927692e18c96d0829dd815cce926122ddd84044a0bf9fcf6df29"
        );
    }
}
