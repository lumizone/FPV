//! Append-only local crash log. Two writers:
//!
//!   1. The Rust panic hook installed in `lib.rs::run` — catches
//!      `panic!()` from any thread + the eventual abort signal.
//!   2. The renderer-side React `ErrorBoundary` via the
//!      `log_renderer_crash` Tauri command.
//!
//! Both formats land in the app data dir (`crash.log` under
//! `com.lumizone.fpvdesktop/`) so a user reporting a problem can
//! paste one file. We never ship this anywhere automatically;
//! it's diagnostic only.
//!
//! Rotation is intentionally crude — at 1 MB we rename the file to
//! `crash.log.old` (overwriting any previous rotation) and start
//! fresh. No rolling indexes; crashes are rare enough.

use std::fs::OpenOptions;
use std::io::Write;

use chrono::Utc;
use tracing::warn;

use crate::error::AppResult;
use crate::storage;

const ROTATE_BYTES: u64 = 1_000_000;

/// Append a record. Source = "panic" or "renderer". Best-effort —
/// failures here are logged via `tracing` but never propagated.
pub fn record(source: &str, body: &str) {
    if let Err(err) = record_inner(source, body) {
        warn!(?err, "crash_log write failed");
    }
}

fn record_inner(source: &str, body: &str) -> AppResult<()> {
    let path = storage::crash_log_path()?;

    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() >= ROTATE_BYTES {
            let old = path.with_file_name("crash.log.old");
            let _ = std::fs::rename(&path, &old);
        }
    }

    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    writeln!(
        file,
        "\n=== {ts} · {source} ===\n{body}",
        ts = Utc::now().format("%Y-%m-%dT%H:%M:%SZ"),
        source = source,
        body = body
    )?;
    Ok(())
}

/// Install a `std::panic::set_hook` that mirrors the default
/// behaviour (logs to stderr / tracing) AND appends to the crash
/// log. Call once at app boot.
pub fn install_panic_hook() {
    let prior = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<unknown>".to_string());
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<no message>".to_string());
        let body = format!(
            "thread panicked at {location}\nmessage: {payload}\nbacktrace:\n{bt}",
            bt = std::backtrace::Backtrace::force_capture()
        );
        record("panic", &body);
        prior(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    /// We can't test `record()` directly without scribbling on the
    /// user's real app data dir. Test the rotation logic against a
    /// caller-supplied path so the assertion stays hermetic.
    fn record_to(path: &std::path::Path, source: &str, body: &str) -> AppResult<()> {
        if let Ok(meta) = std::fs::metadata(path) {
            if meta.len() >= ROTATE_BYTES {
                let old = path.with_file_name("crash.log.old");
                let _ = std::fs::rename(path, &old);
            }
        }
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        writeln!(
            file,
            "\n=== {ts} · {source} ===\n{body}",
            ts = Utc::now().format("%Y-%m-%dT%H:%M:%SZ"),
            source = source,
            body = body
        )?;
        Ok(())
    }

    #[test]
    fn appends_records() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("crash.log");
        record_to(&path, "panic", "first").unwrap();
        record_to(&path, "renderer", "second").unwrap();
        let mut buf = String::new();
        std::fs::File::open(&path)
            .unwrap()
            .read_to_string(&mut buf)
            .unwrap();
        assert!(buf.contains("panic"));
        assert!(buf.contains("first"));
        assert!(buf.contains("renderer"));
        assert!(buf.contains("second"));
    }

    #[test]
    fn rotates_at_threshold() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("crash.log");
        // Pre-fill above the threshold so the next record() rotates.
        let big = "x".repeat(ROTATE_BYTES as usize + 10);
        std::fs::write(&path, &big).unwrap();
        record_to(&path, "panic", "post-rotation").unwrap();

        let old = dir.path().join("crash.log.old");
        assert!(old.exists(), "rotation should have moved old file aside");
        let mut current = String::new();
        std::fs::File::open(&path)
            .unwrap()
            .read_to_string(&mut current)
            .unwrap();
        assert!(
            current.contains("post-rotation"),
            "fresh log should hold only the post-rotation entry"
        );
        assert!(
            !current.contains(&"x".repeat(100)),
            "fresh log should not inherit the pre-rotation bulk"
        );
    }
}
