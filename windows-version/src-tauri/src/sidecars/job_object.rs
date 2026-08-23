//! Kill the bundled Ollama sidecar when the app dies, however it dies.
//!
//! `SidecarManager::shutdown_all` (see `sidecars/mod.rs`) handles every
//! GRACEFUL exit — tray Quit, window close, `RunEvent::Exit` — because
//! that event fires for all of them. What it cannot cover is the app not
//! getting to run any code at all: a Rust panic that unwinds past the
//! runtime, a WebView2 crash, End Task in Task Manager, a power cut.
//!
//! On those paths the bundled `ollama.exe` (and any GPU-runner children
//! it spawns) is simply inherited by the OS and keeps running — holding
//! the port and, once a model is loaded, several GB of VRAM — until the
//! user reboots or kills it by hand. `spawn_bundled_windows`
//! (`sidecars/ollama.rs`, Task 6) has no cooperation from the child to
//! prevent this; the orphan is only survivable in the sense that the next
//! launch's `ping()` can adopt an already-running instance, not in the
//! sense that resources are ever reclaimed.
//!
//! A Windows Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` moves
//! the guarantee into the kernel: every handle to the job closes when our
//! process object is destroyed — which happens on EVERY exit path,
//! including the ones that run no user code — and Windows then
//! terminates every process still assigned to it. This is the standard
//! solution to the problem and needs no cooperation from the child.
//!
//! Best-effort by design: any failure here logs and returns. Not being
//! able to create a job is not a reason to refuse to start Ollama — it
//! just means we are back to the previous behaviour (rely on
//! `SidecarManager::shutdown_all` for graceful exits, nothing for
//! abnormal ones).
//!
//! Windows-only. Ported from the Local Waifu Windows fork
//! (`sidecars/job_object.rs`, self-contained there — no character/voice
//! dependencies), where it is compile-verified against the
//! `x86_64-pc-windows-gnu` target; the runtime behaviour needs a real
//! Windows box (kill the app from Task Manager mid-session and confirm
//! `ollama.exe` disappears with it — Task 12).

#![cfg(windows)]

use std::os::windows::io::RawHandle;
use std::sync::OnceLock;

use tracing::{debug, warn};

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

/// The process-wide job. Created once, never closed by us: we WANT the
/// handle to live exactly as long as the process, because its
/// destruction is the trigger that kills the children.
///
/// `HANDLE` is a raw pointer and therefore not `Send`/`Sync`, but a job
/// handle is a kernel object usable from any thread, so the wrapper
/// asserts that explicitly rather than leaking the pointer type.
struct JobHandle(HANDLE);
// SAFETY: a Windows job-object handle is a kernel handle. It is valid
// process-wide and every API used here (AssignProcessToJobObject) is
// thread-safe. The handle is never mutated after creation.
unsafe impl Send for JobHandle {}
unsafe impl Sync for JobHandle {}

fn job() -> Option<&'static JobHandle> {
    static JOB: OnceLock<Option<JobHandle>> = OnceLock::new();
    JOB.get_or_init(|| {
        // SAFETY: a null name creates an unnamed job owned by this
        // process; null attributes means default security. Both are the
        // documented way to make a private job.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            warn!("could not create a job object; the ollama sidecar will outlive an abnormal exit");
            return None;
        }

        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

        // SAFETY: `info` is a correctly-sized, fully-initialised struct
        // of the class named by `JobObjectExtendedLimitInformation`.
        let ok = unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                std::ptr::addr_of!(info).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if ok == 0 {
            warn!("could not set kill-on-close on the job object; closing it");
            // Without the limit the job is useless AND would silently do
            // nothing, so don't keep it around pretending otherwise.
            unsafe { CloseHandle(handle) };
            return None;
        }

        debug!("job object ready — the ollama sidecar will be killed if this process dies abnormally");
        Some(JobHandle(handle))
    })
    .as_ref()
}

/// Force the job object to be created now, rather than lazily on the
/// first `adopt()` call. Called once at app startup (`lib.rs`, right
/// after `SidecarManager::new()`) purely so a creation failure is logged
/// at boot instead of silently deferred to the first Ollama spawn.
pub fn init() {
    let _ = job();
}

/// Put a freshly-spawned child under the process-wide job, so Windows
/// terminates it if we die without running our own shutdown.
///
/// Call immediately after `spawn()`. Safe to call for any child; a
/// process already in a job it cannot leave is simply left alone (that
/// is the documented failure of `AssignProcessToJobObject`, and it is
/// not fatal for us).
pub fn adopt(child: RawHandle, label: &str) {
    let Some(job) = job() else { return };
    // SAFETY: `child` is the handle of a process we just spawned and
    // still own; the job handle is valid for the process lifetime.
    let ok = unsafe { AssignProcessToJobObject(job.0, child as HANDLE) };
    if ok == 0 {
        // Common and harmless when something else (a debugger, some
        // container runtimes) already placed us in a non-breakaway job.
        warn!(sidecar = label, "could not assign the sidecar to the job object");
    } else {
        debug!(sidecar = label, "sidecar assigned to the job object");
    }
}
