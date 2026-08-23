//! Windows GPU runtime provisioning for the local image engine
//! (stable-diffusion.cpp / `sd-cli.exe`).
//!
//! WHY THIS EXISTS: on macOS there is nothing to provision — Metal is
//! compiled into the `sd-cli` binary FPV bundles, so `sd_cli_path()` is a
//! 16-line "look next to the exe" helper. Windows has no equivalent: the
//! upstream `leejet/stable-diffusion.cpp` release ships SEPARATE
//! per-backend Windows builds (CPU / CUDA 12 / ROCm / Vulkan), and the
//! accelerated ones are far too large to bundle inside an NSIS installer
//! (makensis is 32-bit and dies past ~2 GB of uncompressed payload, which
//! the Ollama runtime already eats most of).
//!
//! So the Windows story is two-tier:
//!   1. The **CPU build is BUNDLED** (`src-tauri/image-runtime/`, staged by
//!      `scripts/fetch-binaries-win.ps1`, shipped as a Tauri resource). It
//!      always works — slowly, but it works, on every machine, offline,
//!      with no download.
//!   2. The **vendor-matched GPU build is FETCHED on demand** into app-data
//!      by `fetch_gpu_runtime`, and `image::sdcpp::sd_cli_path()` prefers
//!      it over the bundled CPU build once, and only once, it is fully
//!      provisioned.
//!
//! Properties this module is built around:
//!   - **Fail-safe, never fail-open.** Any failure (no network, bad hash,
//!     extract error, unknown GPU) leaves the bundled CPU build in charge.
//!     A GPU runtime can only ever ADD speed; it can never be the reason
//!     image generation stops working.
//!   - **A partial extract must never masquerade as ready.** The GPU dir is
//!     assembled in a STAGING directory and moved into place with a single
//!     `rename` (same volume, so it is atomic), with the `.provisioned`
//!     sentinel written INSIDE the staging dir before the move. An
//!     interrupted download/extract therefore leaves either the old state
//!     or nothing — never a half-populated directory that `sd_cli_path()`
//!     would happily spawn. (NVIDIA needs TWO assets — the sd build and the
//!     CUDA runtime DLLs — so "sd-cli.exe exists" is emphatically not proof
//!     of a usable runtime.)
//!   - **The sentinel is VERSIONED**, `"{release}:{vendor}"`. A bare
//!     "it's there" marker cannot tell a current runtime from one left by an
//!     older app version pinned to an older upstream release, or from one
//!     provisioned for a GPU that has since been swapped — both of which must
//!     re-provision rather than be trusted forever.
//!   - **Provisioning is serialized** by `PROVISION_LOCK`. Boot-time
//!     provisioning and a user-triggered retry both write the same fixed
//!     paths; without a shared lock one pass can be mid-write on files the
//!     other is deleting.
//!   - **Windows-only.** The whole module is `#[cfg(windows)] pub mod` in
//!     `sidecars/mod.rs`, so it does not compile at all on macOS — which is
//!     also why nothing in here can be verified by a host `cargo check`.
//!
//! Adapted from Local Waifu's Windows fork — the vendor detection,
//! `PROVISION_LOCK`, sentinel and download/verify/extract machinery come
//! from its `sidecars/gpu_runtime.rs` (which provisions OLLAMA runners) and
//! its `image/sdcpp.rs` (which provisions the sd.cpp GPU build). Only the
//! sd.cpp/image half is carried over; everything in that codebase about
//! character voices, whisper, chatterbox and licence gating is deliberately
//! absent here because FPV's desktop port has none of it.

use std::path::{Path, PathBuf};

use tauri::AppHandle;

use crate::error::{AppError, AppResult};

/// The sd.cpp CLI binary name on Windows. Must match `image::sdcpp`'s
/// `SD_EXE` — this module locates the same executable, just in the
/// app-data GPU directory instead of the bundled resource directory.
pub const SD_EXE: &str = "sd-cli.exe";

/// Written LAST, only after every asset for this vendor has extracted
/// successfully, and then only inside the staging directory that is about
/// to be renamed into place. Its presence plus matching content is the
/// ONLY definition of "provisioned" this module accepts.
const PROVISIONED_SENTINEL: &str = ".provisioned";

/// Upstream release the pinned asset names and SHA-256 digests below belong
/// to. Bump the tag, the asset names AND the digests together — a mismatched
/// set fails the integrity check on every download, which is fail-safe
/// (users stay on the bundled CPU build) but permanently disables GPU.
///
/// Asset names verified live against the GitHub release API on 2026-08-18:
/// this release publishes `sd-master-cc73429-bin-win-cpu-x64.zip`,
/// `-cuda12-x64.zip`, `-rocm-7.13.0-x64.zip`, `-vulkan-x64.zip` plus
/// `cudart-sd-bin-win-cu12-x64.zip`.
const SD_RELEASE_TAG: &str = "master-769-cc73429";

const SD_RELEASE_BASE: &str =
    "https://github.com/leejet/stable-diffusion.cpp/releases/download/master-769-cc73429";

/// Which GPU the machine has, as far as we can tell with tools that exist
/// on a stock Windows install (no extra dependencies, no driver SDKs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuVendor {
    Nvidia,
    Amd,
    /// Intel / virtual / none / undetectable. Upstream sd.cpp publishes no
    /// prebuilt Windows build we can use for these, so they keep the
    /// bundled CPU build and never pay for a download.
    Other,
}

impl GpuVendor {
    /// Stable tag baked into the `.provisioned` sentinel. Changing these
    /// strings invalidates every existing user's sentinel (they re-provision
    /// once) — so don't, without meaning to.
    pub fn tag(self) -> &'static str {
        match self {
            GpuVendor::Nvidia => "nvidia",
            GpuVendor::Amd => "amd",
            GpuVendor::Other => "other",
        }
    }
}

// ── Paths + sentinel ─────────────────────────────────────────────────

/// App-data dir holding the fetched GPU build. Deliberately NOT inside the
/// install directory: an app update replaces the install dir wholesale, and
/// re-downloading ~1 GB on every update would be indefensible.
fn gpu_runtime_dir() -> AppResult<PathBuf> {
    Ok(crate::storage::app_data_dir()?.join("image-runtime-gpu"))
}

/// Where a provisioning pass assembles the runtime before committing it.
/// A SIBLING of the real dir, deliberately — same parent means same volume,
/// which is what makes the final `rename` an atomic move rather than a
/// copy that can be interrupted halfway.
fn gpu_staging_dir() -> AppResult<PathBuf> {
    Ok(crate::storage::app_data_dir()?.join("image-runtime-gpu.staging"))
}

/// The exact sentinel content a runtime provisioned RIGHT NOW, on THIS
/// machine, would carry: `"{release}:{vendor}"`.
fn expected_marker(vendor: GpuVendor) -> String {
    format!("{SD_RELEASE_TAG}:{}", vendor.tag())
}

/// Pure predicate, split out from the I/O so it is readable and so the
/// staleness rule is stated in exactly one place: a sentinel is current
/// only when BOTH the upstream release and the detected vendor match.
/// Anything else — an older release, a different vendor, an empty file,
/// a truncated write — is stale and must re-provision.
fn marker_is_current(marker: &str, vendor: GpuVendor) -> bool {
    marker == expected_marker(vendor)
}

/// True when `dir` holds a runtime provisioned for this exact upstream
/// release AND this machine's GPU vendor.
fn gpu_runtime_is_current(dir: &Path) -> bool {
    let vendor = detect_vendor();
    std::fs::read_to_string(dir.join(PROVISIONED_SENTINEL))
        .map(|marker| marker_is_current(marker.trim(), vendor))
        .unwrap_or(false)
}

/// The provisioned GPU `sd-cli.exe`, or `None`.
///
/// This is the single entry point `image::sdcpp::sd_cli_path()` calls. It
/// is deliberately conservative and I/O-only (no network, no spawning, no
/// mutation): it returns a path ONLY when the binary exists AND the
/// versioned sentinel confirms a complete, current provisioning. Every
/// other outcome returns `None`, and the caller falls through to the
/// bundled CPU build — the "never worse than shipped" invariant.
pub fn current_sd_cli_path() -> Option<PathBuf> {
    let dir = gpu_runtime_dir().ok()?;
    let exe = dir.join(SD_EXE);
    if exe.is_file() && gpu_runtime_is_current(&dir) {
        Some(exe)
    } else {
        None
    }
}

// ── Provisioning ─────────────────────────────────────────────────────
//
// Reached from `spawn_boot_provision` (called at boot, gated on provider ==
// Local AND the selected model's weights already cached — see below), and
// from the user-visible retry entry point, `commands::hardware::
// gpu_runtime_retry_image` — a thin Tauri command that calls
// `fetch_gpu_runtime` directly. `PROVISION_LOCK` serializes the two against
// each other, so a boot+retry overlap cannot interleave a provisioning pass.

/// Serializes the entire provisioning pass.
///
/// The boot trigger and any future user-visible "retry" both write the SAME
/// fixed paths (the staging dir, the final dir). Without a shared lock, a
/// double-click or a boot+retry overlap has one pass extracting into a
/// directory the other is mid-`remove_dir_all` on. The lock is module-level,
/// not local to the function, so any future entry point shares it by
/// construction.
static PROVISION_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Bumped exactly once, right after each successful `commit_staged_runtime`
/// — i.e. the instant a new runtime becomes live. Lets `invalidate_current_runtime`
/// tell "the runtime I just failed to spawn" from "a DIFFERENT, freshly
/// provisioned runtime that superseded it while my spawn was failing":
/// comparing file *content* cannot do this, because two provisioning passes
/// for the same machine (same release, same vendor) write the byte-identical
/// sentinel string. A monotonic generation number can. Not persisted — a
/// restart naturally resets it to 0, which is safe: nothing is provisioned
/// yet at that point either.
static PROVISION_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Current provisioning generation. Callers that are about to spawn the
/// fetched GPU binary snapshot this value first, then pass it back to
/// `invalidate_current_runtime` if that spawn fails — see the doc comment
/// there for why the snapshot must be taken at spawn time, not at
/// invalidation time.
pub fn current_generation() -> u64 {
    PROVISION_GENERATION.load(std::sync::atomic::Ordering::SeqCst)
}

/// The release assets to fetch for a given vendor.
///
/// NVIDIA needs TWO: the sd build itself, and the CUDA runtime DLLs it
/// links against (upstream ships those as a separate archive). This is
/// exactly why a completion sentinel exists — after the first asset alone,
/// `sd-cli.exe` is present but every render fails on a missing DLL.
fn assets_for(vendor: GpuVendor) -> &'static [&'static str] {
    match vendor {
        GpuVendor::Nvidia => &[
            "sd-master-cc73429-bin-win-cuda12-x64.zip",
            "cudart-sd-bin-win-cu12-x64.zip",
        ],
        GpuVendor::Amd => &["sd-master-cc73429-bin-win-rocm-7.13.0-x64.zip"],
        GpuVendor::Other => &[],
    }
}

/// SHA-256 of each pinned asset. The archive we extract contains an
/// executable this app then SPAWNS, so it must not be possible to swap or
/// corrupt it in transit; an unpinned asset is refused outright rather than
/// downloaded unverified.
///
/// PROVENANCE: these three digests are carried over from the Local Waifu
/// Windows fork, which verified them against this same release tag and these
/// same asset names on 2026-08-11. They have NOT been independently
/// re-verified here. If they are wrong the failure mode is safe and loud in
/// the log — every download fails its integrity check and the bundled CPU
/// build keeps serving — but GPU acceleration would never engage, so
/// re-verify them on the first real Windows build.
fn expected_sha256(asset: &str) -> Option<&'static str> {
    match asset {
        "sd-master-cc73429-bin-win-cuda12-x64.zip" => {
            Some("3e4d7fac743fa120530ccbaf0901feef03518be2672844d229c90710313941ca")
        }
        "cudart-sd-bin-win-cu12-x64.zip" => {
            Some("fe20366827d357c00797eebb58244dddab7fd9a348d70090c3871004c320f38d")
        }
        "sd-master-cc73429-bin-win-rocm-7.13.0-x64.zip" => {
            Some("3a60d94d6247611733dfcd6cfb6390f21308b498671214861589393a5b266d99")
        }
        _ => None,
    }
}

fn emit(app: &AppHandle, phase: &str, percent: Option<u8>, message: &str) {
    use tauri::Emitter;
    // Same event + payload shape `commands::image::image_local_prewarm`
    // already emits, so the existing frontend listener needs no changes.
    let _ = app.emit(
        "image:prewarm",
        serde_json::json!({
            "phase": phase,
            "percent": percent,
            "message": message,
        }),
    );
    // `image:prewarm` is only listened to by the lazily-mounted
    // `ImageModelTab` Settings tab — at boot nobody is listening, so this
    // module's own download would otherwise be completely invisible: no
    // toast, no progress, nothing on screen. `gpu-setup` is the shared
    // global channel App.tsx turns into a toast, and
    // `sidecars::ollama_gpu_runtime` emits the exact same phase vocabulary
    // on it — one generic "setting up GPU acceleration" toast covers both
    // subsystems.
    if let Some(toast_phase) = match phase {
        "downloading" => Some("downloading"),
        "done" => Some("ready"),
        "error" => Some("error"),
        _ => None,
    } {
        let _ = app.emit("gpu-setup", toast_phase);
    }
}

/// Fetch the vendor-matched accelerated sd.cpp build into app-data, where
/// `sd_cli_path()` prefers it over the bundled CPU build.
///
/// No-op (and `Ok`) when the runtime is already current, or when the GPU
/// vendor has no prebuilt Windows build. Fail-safe on every error path: the
/// staging dir is destroyed and the previously-installed runtime (or the
/// bundled CPU build) is left untouched and serving.
///
/// Called from `spawn_boot_provision` (boot trigger, gated on the local
/// provider actually being selected and weights already cached). `pub` so a
/// future user-visible "retry" entry point can call it directly without
/// reopening this module — `PROVISION_LOCK` already serializes the two.
pub async fn fetch_gpu_runtime(app: &AppHandle) -> AppResult<()> {
    // Serialize with any other provisioning pass BEFORE looking at the
    // filesystem: a concurrent caller must wait and then observe the
    // freshly-committed state, not race it.
    let _guard = PROVISION_LOCK.lock().await;

    let final_dir = gpu_runtime_dir()?;

    // A previous pass may have crashed (power loss, forced quit) between
    // `commit_staged_runtime`'s step 1 (retire the live runtime) and step 2
    // (install the new one) — see that function's doc comment. `final_dir`
    // is then missing but the retired copy, which was WORKING right up
    // until the crash, survives untouched under a `.old-*` sibling name.
    // Restore it before anything else so a crash costs nothing worse than
    // "renders on the bundled CPU build until this runs again", instead of
    // silently discarding a good runtime and re-paying its download.
    recover_orphaned_runtime(&final_dir);

    let vendor = detect_vendor();
    tracing::info!(vendor = vendor.tag(), "image GPU runtime: detected vendor");

    if gpu_runtime_is_current(&final_dir) {
        return Ok(());
    }
    let assets = assets_for(vendor);
    if assets.is_empty() {
        tracing::info!("image GPU runtime: no prebuilt build for this GPU; keeping bundled CPU build");
        return Ok(());
    }

    let staging = gpu_staging_dir()?;
    // A staging dir left by a previous interrupted pass is garbage by
    // definition — it never got renamed, so it never counted for anything.
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;

    emit(app, "downloading", Some(0), "Fetching GPU image runtime...");

    let result = provision_into(&staging, vendor, assets).await;
    match result {
        Ok(()) => {}
        Err(err) => {
            let _ = std::fs::remove_dir_all(&staging);
            emit(app, "error", None, &err.to_string());
            return Err(err);
        }
    }

    // COMMIT. Everything above wrote only into `staging`; the live runtime
    // has not been touched yet. See `commit_staged_runtime` for why this is
    // a three-step swap and not a delete-then-rename.
    if let Err(err) = commit_staged_runtime(&staging, &final_dir) {
        let _ = std::fs::remove_dir_all(&staging);
        emit(app, "error", None, "GPU image runtime install failed");
        return Err(err);
    }
    // The new runtime is live as of the line above. Bump AFTER the commit
    // succeeds, never before — a reader observing the bumped generation must
    // be guaranteed the sentinel it pairs with is already on disk.
    PROVISION_GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    emit(app, "done", Some(100), "GPU image runtime ready");
    tracing::info!("image GPU runtime: provisioned; local renders now use the GPU build");
    Ok(())
}

/// Sibling name a runtime being retired is parked under while the new one
/// is renamed into place. A UUID suffix, not a fixed `.old`, so a leftover
/// directory from an earlier pass (its `sd-cli.exe` still locked by a render
/// that was in flight) can never collide with — and thereby block — this
/// pass's rename.
fn retired_dir_for(final_dir: &Path) -> AppResult<PathBuf> {
    let parent = final_dir.parent().ok_or_else(|| {
        AppError::Other("GPU image runtime: runtime dir has no parent directory".to_string())
    })?;
    let name = final_dir.file_name().ok_or_else(|| {
        AppError::Other("GPU image runtime: runtime dir has no file name".to_string())
    })?;
    Ok(parent.join(format!(
        "{}.old-{}",
        name.to_string_lossy(),
        uuid::Uuid::new_v4().simple()
    )))
}

/// Recovers a runtime orphaned by a crash between `commit_staged_runtime`'s
/// step 1 (retire the live runtime under a `.old-*` name) and step 2
/// (install the new one). In that window `final_dir` does not exist, but the
/// retired copy — which was fully working right up until the crash — sits
/// untouched beside it. Restore it so the crash costs at most "one boot on
/// the bundled CPU build" instead of silently losing a good runtime and
/// re-paying its download on the next provisioning pass.
///
/// No-op when `final_dir` already exists (the overwhelmingly common case —
/// this only ever matters after a genuine crash) or when no `.old-*` sibling
/// with an intact sentinel is found. Only trusts an orphan whose sentinel
/// survived — a directory left mid-`copy_tree` by a DIFFERENT, unrelated
/// interruption must never be promoted back to live. If more than one
/// candidate somehow survives (should not happen — a pass only ever retires
/// one runtime, and `sweep_retired_dirs` cleans up after itself), the
/// newest by mtime wins.
fn recover_orphaned_runtime(final_dir: &Path) {
    if final_dir.exists() {
        return;
    }
    let (Some(parent), Some(name)) = (final_dir.parent(), final_dir.file_name()) else {
        return;
    };
    let prefix = format!("{}.old-", name.to_string_lossy());
    let Ok(entries) = std::fs::read_dir(parent) else {
        return;
    };
    let mut candidates: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        if !entry.file_name().to_string_lossy().starts_with(&prefix) {
            continue;
        }
        let path = entry.path();
        if !path.join(PROVISIONED_SENTINEL).is_file() {
            continue;
        }
        let mtime = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        candidates.push((mtime, path));
    }
    candidates.sort_by_key(|(mtime, _)| *mtime);
    let Some((_, newest)) = candidates.pop() else {
        return;
    };
    match std::fs::rename(&newest, final_dir) {
        Ok(()) => tracing::warn!(
            path = %newest.display(),
            "image GPU runtime: recovered a runtime orphaned by an interrupted provisioning pass"
        ),
        Err(err) => tracing::warn!(
            path = %newest.display(),
            %err,
            "image GPU runtime: found an orphaned runtime but could not restore it; bundled CPU build serves until the next provisioning pass"
        ),
    }
}

/// Best-effort removal of retired directories left by earlier passes.
/// Failure is expected and harmless: a `.old-*` dir whose exe is still held
/// by a running render simply survives until a later pass can delete it. It
/// is never consulted by `current_sd_cli_path()`, so it costs disk and
/// nothing else.
fn sweep_retired_dirs(final_dir: &Path) {
    let (Some(parent), Some(name)) = (final_dir.parent(), final_dir.file_name()) else {
        return;
    };
    let prefix = format!("{}.old-", name.to_string_lossy());
    let Ok(entries) = std::fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with(&prefix) {
            if let Err(err) = std::fs::remove_dir_all(entry.path()) {
                tracing::warn!(
                    path = %entry.path().display(),
                    %err,
                    "image GPU runtime: could not clean up a retired runtime dir (harmless, retried next provision)"
                );
            }
        }
    }
}

/// Swap `staging` into `final_dir` without the live directory ever being
/// observable in a partial state.
///
/// WHY NOT `remove_dir_all(final_dir)` THEN `rename`: on Windows a render
/// that happens to be in flight holds an OS-level lock on the live
/// `sd-cli.exe`. `remove_dir_all` deletes what it can — typically the
/// backend DLLs — and only then fails on the locked exe, leaving a
/// half-emptied live directory: exe plus sentinel present, DLLs gone. That
/// still satisfies `current_sd_cli_path()`'s "exe + sentinel" test, so every
/// subsequent render spawns a binary that cannot load. The following
/// `rename` then fails too, because `final_dir` still exists. The old code
/// discarded the delete's error (`let _ =`), so this was silent.
///
/// The swap below never deletes anything that is live:
///   1. rename the existing runtime aside (metadata-only; it cannot
///      half-empty the directory the way a recursive delete can),
///   2. rename staging into place — the single, atomic moment the new
///      runtime becomes live,
///   3. only then delete the retired copy, best-effort.
///
/// If step 2 fails after step 1 succeeded the live directory is missing, so
/// the retired copy is renamed back: a stale-but-whole runtime beats none.
/// Either way the error is returned, so the caller keeps treating the CPU
/// build as what is actually running.
///
/// Runs under `PROVISION_LOCK` (held by `fetch_gpu_runtime` across this
/// call), so two provisioning passes cannot interleave the three steps.
fn commit_staged_runtime(staging: &Path, final_dir: &Path) -> AppResult<()> {
    sweep_retired_dirs(final_dir);

    // Step 1 — move the existing runtime aside, if there is one.
    let retired = retired_dir_for(final_dir)?;
    let had_previous = final_dir.exists();
    if had_previous {
        std::fs::rename(final_dir, &retired).map_err(|err| {
            AppError::Other(format!(
                "GPU image runtime: could not move the existing runtime aside: {err}"
            ))
        })?;
    }

    // Step 2 — the commit itself.
    if let Err(err) = std::fs::rename(staging, final_dir) {
        if had_previous {
            match std::fs::rename(&retired, final_dir) {
                Ok(()) => tracing::warn!(
                    "image GPU runtime: install failed; restored the previous runtime"
                ),
                Err(restore_err) => tracing::error!(
                    %restore_err,
                    "image GPU runtime: install failed AND the previous runtime could not be restored; renders fall back to the bundled CPU build"
                ),
            }
        }
        return Err(AppError::Other(format!(
            "GPU image runtime: could not install staged runtime: {err}"
        )));
    }

    // Step 3 — the new runtime is already live; this is pure housekeeping.
    if had_previous {
        if let Err(err) = std::fs::remove_dir_all(&retired) {
            tracing::warn!(
                path = %retired.display(),
                %err,
                "image GPU runtime: installed, but the retired runtime dir could not be deleted (harmless, retried next provision)"
            );
        }
    }
    Ok(())
}

/// Mark the installed GPU runtime as not-provisioned by deleting its
/// sentinel, so the very next `current_sd_cli_path()` returns `None` and
/// renders fall back to the bundled CPU build.
///
/// WHY: the sentinel proves the extract COMPLETED, not that the result
/// RUNS. A CUDA build against a too-old driver, an upstream archive whose
/// layout moved, a ROCm asset missing a DLL — all commit cleanly and then
/// fail at spawn time, forever, because nothing downgrades them. This is the
/// downgrade: one cheap, idempotent file delete, no directory mutation (the
/// files stay put, only the claim about them is withdrawn), and the next
/// `fetch_gpu_runtime` sees a stale runtime and re-provisions it.
///
/// `expected_generation` MUST be a snapshot of `current_generation()` taken
/// at the moment the caller resolved the path it went on to spawn — NOT at
/// the moment this function runs. Without it, a failure against runtime A
/// can delete the sentinel of runtime B: a boot-time `fetch_gpu_runtime`
/// pass can commit a brand-new runtime WHILE an in-flight render is still
/// using the old one, and if that old render then fails and calls this
/// function, a bare "delete whatever sentinel exists now" would discard the
/// fresh runtime for a failure that predates it — a runtime that has never
/// even been tried gets downgraded for someone else's crash. Comparing file
/// *content* cannot catch this: two provisioning passes for the same
/// machine (same pinned release, same detected vendor) write the
/// byte-identical marker string, so only a monotonic counter distinguishes
/// "still the runtime I spawned" from "already superseded".
///
/// Returns `true` if a sentinel was actually removed. Mostly does NOT take
/// the provisioning lock — it is called from the render path, which must
/// never block behind a gigabyte-sized download — EXCEPT for a non-blocking
/// `try_lock`: if a provisioning pass is running RIGHT NOW, one is either
/// about to change what "current" means or just did, so this closes the
/// otherwise-open window between the generation check above and the delete
/// below without ever waiting on the lock holder.
pub fn invalidate_current_runtime(expected_generation: u64) -> bool {
    if expected_generation != current_generation() {
        tracing::info!(
            "image GPU runtime: skipping invalidation — a newer runtime was \
             provisioned after this failure occurred"
        );
        return false;
    }
    // Non-blocking. A held lock means a provisioning pass is in progress —
    // its commit may be seconds away, so this failure is likely already
    // stale; skip rather than block the render's error path on a download.
    let Ok(_guard) = PROVISION_LOCK.try_lock() else {
        tracing::info!(
            "image GPU runtime: a provisioning pass is in progress; skipping \
             invalidation of a possibly-stale failure"
        );
        return false;
    };
    // Re-check under the lock: this closes the window between the
    // generation check above and the delete below. The lock was free a
    // moment ago (we just acquired it), so this is not a blocking wait —
    // just the final word on whether the runtime is still the one that
    // failed.
    if expected_generation != current_generation() {
        return false;
    }
    let Ok(dir) = gpu_runtime_dir() else {
        return false;
    };
    let sentinel = dir.join(PROVISIONED_SENTINEL);
    match std::fs::remove_file(&sentinel) {
        Ok(()) => {
            tracing::warn!(
                "image GPU runtime: marked unusable; renders fall back to the bundled CPU build"
            );
            true
        }
        // Benign: nothing was provisioned in the first place (already
        // invalidated, never provisioned, or a concurrent provisioning pass
        // just swapped the directory out from under us). No downgrade was
        // needed, so this is not an error worth a log line.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        // NOT benign: the sentinel exists but could not be removed — e.g.
        // PermissionDenied, or a Windows sharing violation from an AV
        // scanner holding the file open. This means the downgrade did NOT
        // happen: the broken GPU runtime stays "provisioned" and every
        // future render keeps failing the same way forever. Log it so a
        // real failure to downgrade is at least visible.
        Err(e) => {
            tracing::error!(
                error = %e,
                path = %sentinel.display(),
                "image GPU runtime: failed to invalidate sentinel; runtime stays marked provisioned"
            );
            false
        }
    }
}

/// True when `path` is the `sd-cli.exe` inside the fetched GPU runtime dir
/// (as opposed to the bundled CPU build). Lets the render path tell a
/// GPU-runtime failure — which `invalidate_current_runtime()` can fix —
/// from a bundled-build failure, which it cannot.
pub fn is_gpu_runtime_path(path: &Path) -> bool {
    gpu_runtime_dir()
        .map(|dir| path.starts_with(&dir))
        .unwrap_or(false)
}

// ── Boot-time trigger ────────────────────────────────────────────────

/// Boot-time GPU runtime provisioning for the image sd-cli build. Fire and
/// forget: spawned once at app startup, logs and emits on its own, never
/// awaited by the caller.
///
/// WHY THIS EXISTS: FPV's Image Models settings tab only exposes WEIGHT
/// downloads (the diffusion checkpoint) — a Tauri command exists for that
/// (`image_local_prewarm`) but nothing in the UI calls `fetch_gpu_runtime`
/// (the sd-cli BACKEND binary this module provisions, a different concept).
/// Without this boot trigger `fetch_gpu_runtime` would have no reachable
/// caller at all: a user whose weights are already downloaded has no path
/// that ever runs it, so `sd_cli_path()` silently keeps preferring the
/// bundled CPU build forever — renders take minutes instead of seconds,
/// with no log line and no UI signal explaining why. Mirrors Local Waifu's
/// `image::sdcpp::spawn_gpu_provision` / `run_gpu_provision`, adapted to
/// FPV's own provider/weights model (`image_provider` app-meta key +
/// `image::local::weights_status_of`, which is an exact per-manifest check
/// rather than LW's "weights dir is non-empty" heuristic).
///
/// Gated on the user actually using local image generation (provider is
/// Local AND the selected model's weights are already cached) so a
/// cloud-only user or a fresh install never pays for an unwanted
/// several-hundred-MB-to-multi-GB download. Fail-safe: any failure leaves
/// the bundled CPU build active, exactly like every other path into this
/// module.
pub fn spawn_boot_provision(app: AppHandle, db: crate::db::DbHandle) {
    tauri::async_runtime::spawn(async move {
        match run_boot_provision(&app, &db).await {
            Ok(true) => {
                tracing::info!(
                    "image GPU runtime: boot provisioning ran and completed"
                );
            }
            Ok(false) => {}
            Err(err) => {
                tracing::warn!(
                    %err,
                    "image GPU runtime: boot provisioning failed (non-fatal; \
                     bundled CPU build still active)"
                );
            }
        }
    });
}

/// `Ok(true)` means a download actually ran and the runtime is now current;
/// `Ok(false)` covers every no-op path — already current, a cloud provider
/// is selected, no weights cached yet, or the GPU vendor has no prebuilt
/// build (`fetch_gpu_runtime` no-ops for `GpuVendor::Other`, which this
/// function distinguishes from a real success by re-checking
/// `gpu_runtime_is_current` afterwards rather than trusting `Ok(())` alone).
async fn run_boot_provision(app: &AppHandle, db: &crate::db::DbHandle) -> AppResult<bool> {
    // Cheap fast path before touching the DB or shelling out to detect the
    // GPU vendor: the overwhelmingly common case after the first boot is
    // "already provisioned, nothing to do".
    let final_dir = gpu_runtime_dir()?;
    if gpu_runtime_is_current(&final_dir) {
        // The live runtime is confirmed current, so any `.old-*` sibling
        // left behind by a prior pass (a crash before its own best-effort
        // cleanup ran, or one `recover_orphaned_runtime` never needed
        // because this boot's runtime was already fine) can never be
        // needed for recovery again — recovery only ever restores INTO a
        // missing `final_dir`. Reap them here so a machine that reaches
        // "current" and then never re-provisions (the common case) isn't
        // stuck carrying that disk cost forever.
        sweep_retired_dirs(&final_dir);
        return Ok(false);
    }

    let (provider_local, model) = {
        let conn = db.lock().await;
        let provider = crate::db::meta_get(&conn, "image_provider")
            .ok()
            .flatten()
            .and_then(|s| crate::image::ImageProvider::from_str(&s))
            .unwrap_or_default();
        let model = crate::commands::image::read_local_model(&conn);
        (
            matches!(provider, crate::image::ImageProvider::Local),
            model,
        )
    };
    if !provider_local {
        return Ok(false);
    }
    if crate::image::local::weights_status_of(model) != Some(true) {
        return Ok(false);
    }

    fetch_gpu_runtime(app).await?;
    // `fetch_gpu_runtime` itself no-ops (Ok(())) both on real success AND
    // when the vendor has no prebuilt build — re-checking here is what
    // tells those two apart honestly instead of reporting "ran" for a
    // no-op.
    Ok(gpu_runtime_is_current(&final_dir))
}

/// Download + verify + extract every asset into `staging`, then write the
/// sentinel. Split out from `fetch_gpu_runtime` so that function's error
/// handling is one `match` instead of a cleanup call on every `?`.
async fn provision_into(staging: &Path, vendor: GpuVendor, assets: &[&str]) -> AppResult<()> {
    for asset in assets {
        download_and_extract(&format!("{SD_RELEASE_BASE}/{asset}"), staging).await?;
    }
    // Cheap structural check before declaring success — an archive whose
    // layout changed upstream would otherwise be committed as "provisioned"
    // and then fail at spawn time with a far more confusing error.
    if !staging.join(SD_EXE).is_file() {
        return Err(AppError::Other(format!(
            "GPU image runtime: {SD_EXE} missing after extracting all assets"
        )));
    }
    // LAST write before the caller's rename: this is what makes the
    // committed directory count as provisioned.
    std::fs::write(staging.join(PROVISIONED_SENTINEL), expected_marker(vendor))?;
    Ok(())
}

/// Stream `url` to a temp file, verify its pinned SHA-256, expand it, and
/// copy the extracted tree flat into `dest`.
///
/// The hash is checked BEFORE extraction, never after: the archive contains
/// an executable this app will spawn, so tampered or truncated bytes must
/// never reach the filesystem we then run from.
async fn download_and_extract(url: &str, dest: &Path) -> AppResult<()> {
    use futures_util::StreamExt;
    use std::io::Write;

    let filename = url.rsplit('/').next().unwrap_or_default();
    // Refuse an unknown asset up front — before spending a gigabyte of the
    // user's bandwidth on something we could not verify anyway.
    let expected = expected_sha256(filename).ok_or_else(|| {
        AppError::Other(format!(
            "GPU image runtime: asset is not pinned, refusing to download: {filename}"
        ))
    })?;

    let tmp = std::env::temp_dir().join(format!("fpv_sdgpu_{}.zip", uuid::Uuid::new_v4().simple()));

    // An idle READ timeout, not a total request cap: these archives run
    // several hundred MB, and a total cap would kill a download that is
    // slow but perfectly healthy.
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .read_timeout(std::time::Duration::from_secs(120))
        .build()?;
    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        return Err(AppError::Other(format!(
            "GPU image runtime download {}: {url}",
            resp.status()
        )));
    }
    {
        let mut file = std::fs::File::create(&tmp)?;
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(err) => {
                    let _ = std::fs::remove_file(&tmp);
                    return Err(err.into());
                }
            };
            if let Err(err) = file.write_all(&chunk) {
                let _ = std::fs::remove_file(&tmp);
                return Err(err.into());
            }
        }
        if let Err(err) = file.flush() {
            let _ = std::fs::remove_file(&tmp);
            return Err(err.into());
        }
    }

    let actual = match sha256_file(&tmp) {
        Ok(hash) => hash,
        Err(err) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(err);
        }
    };
    if !actual.eq_ignore_ascii_case(expected) {
        let _ = std::fs::remove_file(&tmp);
        return Err(AppError::Other(format!(
            "GPU image runtime: SHA-256 mismatch for {filename} (expected {expected}, got {actual})"
        )));
    }

    // Expand-Archive is built into Windows PowerShell, so unzipping costs
    // no extra crate dependency. `_extract` lives under the staging dir
    // (same volume, cleaned up either way) and never under the live dir.
    let extract = dest.join("_extract");
    let _ = std::fs::remove_dir_all(&extract);
    std::fs::create_dir_all(&extract)?;
    let ps = format!(
        "Expand-Archive -LiteralPath '{}' -DestinationPath '{}' -Force",
        tmp.to_string_lossy().replace('\'', "''"),
        extract.to_string_lossy().replace('\'', "''"),
    );
    let ok = run_powershell(&ps);
    let _ = std::fs::remove_file(&tmp);
    if !ok {
        let _ = std::fs::remove_dir_all(&extract);
        return Err(AppError::Other(format!(
            "GPU image runtime: Expand-Archive failed for {filename}"
        )));
    }

    // Upstream nests the payload one folder deep in some archives and not
    // in others; locate the real root by its `sd-cli.exe` marker, and fall
    // back to the archive root for the CUDA-runtime archive, which carries
    // only DLLs and no executable.
    let src = find_dir_with(&extract, SD_EXE).unwrap_or_else(|| extract.clone());
    copy_tree(&src, dest)?;
    let _ = std::fs::remove_dir_all(&extract);
    Ok(())
}

fn sha256_file(path: &Path) -> AppResult<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    // Formatted byte-by-byte on purpose: the current RustCrypto digest
    // output type does not implement `LowerHex`, so `format!("{:x}", ..)`
    // does not compile against it.
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Depth-first search for the directory directly containing `name`.
fn find_dir_with(root: &Path, name: &str) -> Option<PathBuf> {
    if root.join(name).is_file() {
        return Some(root.to_path_buf());
    }
    for entry in std::fs::read_dir(root).ok()?.flatten() {
        let p = entry.path();
        if p.is_dir() {
            if let Some(found) = find_dir_with(&p, name) {
                return Some(found);
            }
        }
    }
    None
}

/// Copy everything under `src` into `dest`, preserving relative structure.
/// Skips `_extract` so a second asset's extraction scratch dir can never be
/// copied into the runtime we are assembling.
fn copy_tree(src: &Path, dest: &Path) -> AppResult<()> {
    for entry in std::fs::read_dir(src)?.flatten() {
        let p = entry.path();
        let target = dest.join(entry.file_name());
        if p.is_dir() {
            if p.file_name().map(|n| n == "_extract").unwrap_or(false) {
                continue;
            }
            std::fs::create_dir_all(&target)?;
            copy_tree(&p, &target)?;
        } else {
            std::fs::copy(&p, &target)?;
        }
    }
    Ok(())
}

// ── GPU detection ────────────────────────────────────────────────────

/// Memoised primary-GPU vendor. The GPU cannot change while the process
/// runs, and the uncached path shells out to `nvidia-smi` and — on
/// non-NVIDIA machines — a 1-3 s PowerShell CIM query, which is far too
/// expensive to repeat on every `sd_cli_path()` call (and `sd_cli_path()`
/// is called on every render).
pub fn detect_vendor() -> GpuVendor {
    use std::sync::OnceLock;
    static VENDOR: OnceLock<GpuVendor> = OnceLock::new();
    *VENDOR.get_or_init(detect_vendor_uncached)
}

/// Uses only tools present on a stock Windows install. Returns `Other`
/// whenever unsure — the conservative answer, since `Other` means "keep the
/// bundled CPU build", which always works.
fn detect_vendor_uncached() -> GpuVendor {
    // NVIDIA ships `nvidia-smi` with its driver; if it runs at all, there is
    // a working NVIDIA GPU and a working driver.
    if run_ok("nvidia-smi", &["-L"]) {
        return GpuVendor::Nvidia;
    }
    if let Some(names) = video_controller_names() {
        let n = names.to_lowercase();
        if n.contains("nvidia")
            || n.contains("geforce")
            || n.contains("rtx")
            || n.contains("quadro")
            || n.contains("tesla")
        {
            return GpuVendor::Nvidia;
        }
        if n.contains("amd") || n.contains("radeon") {
            return GpuVendor::Amd;
        }
    }
    GpuVendor::Other
}

fn video_controller_names() -> Option<String> {
    let out = powershell_output(
        "(Get-CimInstance Win32_VideoController | Select-Object -ExpandProperty Name) -join '|'",
    )?;
    let trimmed = out.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn run_ok(program: &str, args: &[&str]) -> bool {
    let mut cmd = std::process::Command::new(program);
    cmd.args(args);
    hide_window(&mut cmd);
    cmd.output().map(|o| o.status.success()).unwrap_or(false)
}

fn run_powershell(command: &str) -> bool {
    let mut cmd = std::process::Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", command]);
    hide_window(&mut cmd);
    cmd.status().map(|s| s.success()).unwrap_or(false)
}

fn powershell_output(command: &str) -> Option<String> {
    let mut cmd = std::process::Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", command]);
    hide_window(&mut cmd);
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The app is a GUI-subsystem binary with no console, so a raw spawn
/// FLASHES a console window unless this flag is set. Every probe here can
/// run while the user is looking at the app, so all of them set it — same
/// rule the Ollama sidecar spawn already follows.
fn hide_window(cmd: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vendor_tags_are_stable() {
        assert_eq!(GpuVendor::Nvidia.tag(), "nvidia");
        assert_eq!(GpuVendor::Amd.tag(), "amd");
        assert_eq!(GpuVendor::Other.tag(), "other");
    }

    #[test]
    fn a_marker_is_current_only_for_its_own_release_and_vendor() {
        let nvidia = expected_marker(GpuVendor::Nvidia);
        assert!(marker_is_current(&nvidia, GpuVendor::Nvidia));
        // Same release, different GPU: the CUDA build is useless on AMD.
        assert!(!marker_is_current(&nvidia, GpuVendor::Amd));
        // Same GPU, older release: must re-provision, not be trusted.
        assert!(!marker_is_current(
            &format!("master-000-deadbee:{}", GpuVendor::Nvidia.tag()),
            GpuVendor::Nvidia
        ));
        // Degenerate contents left by a truncated or legacy write.
        assert!(!marker_is_current("", GpuVendor::Nvidia));
        assert!(!marker_is_current("complete", GpuVendor::Nvidia));
    }

    #[test]
    fn every_asset_we_would_download_is_hash_pinned() {
        for vendor in [GpuVendor::Nvidia, GpuVendor::Amd] {
            for asset in assets_for(vendor) {
                let hash = expected_sha256(asset)
                    .unwrap_or_else(|| panic!("asset {asset} has no pinned SHA-256"));
                assert_eq!(hash.len(), 64, "sha256 must be 64 hex chars for {asset}");
                assert!(
                    hash.chars().all(|c| c.is_ascii_hexdigit()),
                    "sha256 must be pure hex for {asset}: {hash}"
                );
            }
        }
        // Intel/unknown must never trigger a download at all.
        assert!(assets_for(GpuVendor::Other).is_empty());
    }

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "fpv_gpu_rt_test_{name}_{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn populate(dir: &Path, marker: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(SD_EXE), marker).unwrap();
        std::fs::write(dir.join("backend.dll"), marker).unwrap();
        std::fs::write(dir.join(PROVISIONED_SENTINEL), marker).unwrap();
    }

    #[test]
    fn a_retired_dir_is_a_uniquely_named_sibling_of_the_live_dir() {
        let live = Path::new("C:\\data\\image-runtime-gpu");
        let a = retired_dir_for(live).unwrap();
        let b = retired_dir_for(live).unwrap();
        assert_eq!(a.parent(), live.parent(), "must be a sibling (same volume)");
        assert_ne!(a, b, "two passes must never collide on one retired name");
        for p in [&a, &b] {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            assert!(name.starts_with("image-runtime-gpu.old-"), "{name}");
            assert_ne!(p, live, "the retired dir must not be the live dir");
        }
    }

    #[test]
    fn committing_over_an_existing_runtime_replaces_it_whole() {
        let root = scratch_dir("replace");
        let live = root.join("image-runtime-gpu");
        let staging = root.join("image-runtime-gpu.staging");
        populate(&live, "old");
        populate(&staging, "new");

        commit_staged_runtime(&staging, &live).unwrap();

        assert_eq!(std::fs::read_to_string(live.join(SD_EXE)).unwrap(), "new");
        // The DLLs are the point: the old delete-then-rename could drop them.
        assert_eq!(
            std::fs::read_to_string(live.join("backend.dll")).unwrap(),
            "new"
        );
        assert_eq!(
            std::fs::read_to_string(live.join(PROVISIONED_SENTINEL)).unwrap(),
            "new"
        );
        assert!(!staging.exists(), "staging is consumed by the rename");
        // Nothing retired should survive a clean commit.
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn committing_with_no_previous_runtime_just_installs() {
        let root = scratch_dir("fresh");
        let live = root.join("image-runtime-gpu");
        let staging = root.join("image-runtime-gpu.staging");
        populate(&staging, "new");

        commit_staged_runtime(&staging, &live).unwrap();

        assert_eq!(std::fs::read_to_string(live.join(SD_EXE)).unwrap(), "new");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_failed_commit_leaves_the_previous_runtime_whole_and_live() {
        let root = scratch_dir("restore");
        let live = root.join("image-runtime-gpu");
        let staging = root.join("image-runtime-gpu.staging");
        populate(&live, "old");
        // No staging dir at all: step 2's rename cannot succeed, which is the
        // "live dir is now missing" window the restore path exists for.

        let err = commit_staged_runtime(&staging, &live).unwrap_err();
        assert!(err.to_string().contains("could not install staged runtime"));

        assert_eq!(std::fs::read_to_string(live.join(SD_EXE)).unwrap(), "old");
        assert_eq!(
            std::fs::read_to_string(live.join("backend.dll")).unwrap(),
            "old"
        );
        assert!(live.join(PROVISIONED_SENTINEL).is_file());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_leftover_retired_dir_does_not_block_a_later_commit() {
        let root = scratch_dir("sweep");
        let live = root.join("image-runtime-gpu");
        let staging = root.join("image-runtime-gpu.staging");
        populate(&live, "old");
        populate(&root.join("image-runtime-gpu.old-deadbeef"), "ancient");
        populate(&staging, "new");

        commit_staged_runtime(&staging, &live).unwrap();

        assert_eq!(std::fs::read_to_string(live.join(SD_EXE)).unwrap(), "new");
        assert!(!root.join("image-runtime-gpu.old-deadbeef").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn only_the_fetched_runtime_is_treated_as_invalidatable() {
        // The bundled CPU build must never be mistaken for the fetched one:
        // invalidating it would be a no-op at best and misleading at worst.
        let Ok(dir) = gpu_runtime_dir() else {
            return;
        };
        assert!(is_gpu_runtime_path(&dir.join(SD_EXE)));
        assert!(!is_gpu_runtime_path(Path::new("C:\\Program Files\\FPV\\image-runtime\\sd-cli.exe")));
    }

    #[test]
    fn the_release_base_url_matches_the_pinned_tag() {
        assert!(
            SD_RELEASE_BASE.ends_with(SD_RELEASE_TAG),
            "release base URL and pinned tag drifted apart"
        );
    }

    #[test]
    fn invalidating_a_stale_generation_is_a_no_op() {
        // The whole point of the generation guard: a failure captured
        // against an OLDER runtime must never touch whatever is current
        // now. `current_generation() + 1` can never equal the real
        // generation, so this returns false before touching the
        // filesystem at all — safe to run without any scratch dir.
        let stale = current_generation() + 1;
        assert!(!invalidate_current_runtime(stale));
    }

    #[test]
    fn recovering_when_the_live_dir_already_exists_is_a_no_op() {
        let root = scratch_dir("recover_noop");
        let live = root.join("image-runtime-gpu");
        populate(&live, "already-live");

        recover_orphaned_runtime(&live);

        assert_eq!(
            std::fs::read_to_string(live.join(SD_EXE)).unwrap(),
            "already-live",
            "an existing live dir must never be touched"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn recovering_restores_an_orphan_left_by_a_crash_between_swap_steps() {
        let root = scratch_dir("recover_orphan");
        let live = root.join("image-runtime-gpu");
        // Simulates the crash window: `commit_staged_runtime` step 1
        // (retire) ran, step 2 (install) never did, so `live` is missing
        // and only the retired-but-working copy survives.
        populate(&root.join("image-runtime-gpu.old-deadbeef"), "orphaned-good");

        recover_orphaned_runtime(&live);

        assert_eq!(
            std::fs::read_to_string(live.join(SD_EXE)).unwrap(),
            "orphaned-good",
            "the orphan must be restored to the live path"
        );
        assert!(!root.join("image-runtime-gpu.old-deadbeef").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn recovering_ignores_an_orphan_with_no_sentinel() {
        let root = scratch_dir("recover_incomplete");
        let live = root.join("image-runtime-gpu");
        // A DIFFERENT interruption (mid-`copy_tree`) can also leave a
        // `.old-*` sibling — but without a sentinel it was never a
        // complete, working runtime and must not be promoted back.
        let incomplete = root.join("image-runtime-gpu.old-deadbeef");
        std::fs::create_dir_all(&incomplete).unwrap();
        std::fs::write(incomplete.join(SD_EXE), "half-copied").unwrap();

        recover_orphaned_runtime(&live);

        assert!(!live.exists(), "an incomplete orphan must not be promoted");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn recovering_prefers_the_newest_orphan_when_more_than_one_survives() {
        let root = scratch_dir("recover_multi");
        let live = root.join("image-runtime-gpu");
        let older = root.join("image-runtime-gpu.old-aaaaaaaa");
        let newer = root.join("image-runtime-gpu.old-bbbbbbbb");
        populate(&older, "older");
        std::thread::sleep(std::time::Duration::from_millis(20));
        populate(&newer, "newer");

        recover_orphaned_runtime(&live);

        assert_eq!(std::fs::read_to_string(live.join(SD_EXE)).unwrap(), "newer");
        let _ = std::fs::remove_dir_all(&root);
    }
}
