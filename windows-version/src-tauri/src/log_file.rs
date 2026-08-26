//! The tracing stream, on disk.
//!
//! `init_tracing` used to send everything to stdout and nowhere else.
//! That is fine in `npm run tauri dev` and useless in the shipped app:
//! a `.app` launched from Finder has no terminal, so every `info!` and
//! `warn!` the process ever wrote went straight to the void. Which is
//! exactly the material a bug report needs — the voice loop logs its
//! per-turn timing breakdown (`stt_ms` / `reply_ms` / `first_audio_ms`),
//! smart-turn logs the shadow scores it exists to collect, dictation
//! logs how long whisper took, and the microphone logs a device that
//! disappeared. None of it survived the walk from `cargo` to `/Applications`.
//!
//! So: a second fmt layer writing the same stream to
//! `~/Library/Application Support/com.lumizone.fpvdesktop/app.log`.
//!
//! **Deliberately synchronous, deliberately crude.** `tracing-appender`
//! would add a dependency and a background flush thread whose guard has
//! to outlive the process, and the win — not blocking on a write — does
//! not apply here: nothing in this app logs per audio frame. The
//! frequent sites are per-utterance and per-chat-turn, which is a
//! handful of lines a minute. Rotation matches `crash_log`: at the
//! limit, rename to `app.log.old` (overwriting the previous one) and
//! start fresh. Two files, bounded, no rolling indexes to explain to
//! someone who is trying to send them to me.
//!
//! Nothing here is ever uploaded. It is written for a user to find and
//! send deliberately — see `commands::advanced::diagnostics_write`.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::Mutex;

use crate::storage;

/// Rotate at 8 MB. A verbose voice session is a few hundred KB, so this
/// holds many sessions — long enough that "it happened yesterday" is
/// still in the file when someone gets round to reporting it.
const ROTATE_BYTES: u64 = 8_000_000;

/// A `MakeWriter` that appends to `app.log`, rotating on size.
///
/// Failures are swallowed on purpose. A full disk or a permission
/// problem must not take down the app, and it must certainly not
/// recurse: reporting a logging failure through `tracing` would come
/// straight back here.
pub struct AppLogWriter;

impl AppLogWriter {
    fn open() -> Option<File> {
        let path = storage::app_data_dir().ok()?.join("app.log");
        if let Ok(meta) = std::fs::metadata(&path) {
            if meta.len() >= ROTATE_BYTES {
                let _ = std::fs::rename(&path, path.with_file_name("app.log.old"));
            }
        }
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .ok()
    }
}

/// One line's worth of buffer, flushed to the file on drop.
///
/// `tracing` calls `write` several times per event (timestamp, level,
/// target, fields). Opening the file once per event instead of once per
/// fragment keeps the syscall count down, and — more importantly — keeps
/// an event from being interleaved with another thread's halfway
/// through, which is what makes a concurrent log unreadable.
pub struct LineBuffer {
    buf: Vec<u8>,
}

impl Write for LineBuffer {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for LineBuffer {
    fn drop(&mut self) {
        if self.buf.is_empty() {
            return;
        }
        // One lock for the whole event, so two threads cannot split each
        // other's lines.
        static LOCK: Mutex<()> = Mutex::new(());
        let _guard = LOCK.lock();
        if let Some(mut file) = AppLogWriter::open() {
            let _ = file.write_all(&self.buf);
        }
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for AppLogWriter {
    type Writer = LineBuffer;
    fn make_writer(&'a self) -> Self::Writer {
        LineBuffer {
            buf: Vec::with_capacity(256),
        }
    }
}

/// Where the log lives, for the UI to reveal.
pub fn path() -> Option<std::path::PathBuf> {
    Some(storage::app_data_dir().ok()?.join("app.log"))
}

/// The last `bytes` of the log, for a diagnostics bundle.
///
/// Reads from the END: a bug report wants what just happened, and the
/// whole file can be 8 MB. Starts at the first newline after the cut so
/// the excerpt never opens mid-line.
pub fn tail(bytes: usize) -> String {
    let Some(p) = path() else {
        return String::new();
    };
    let Ok(data) = std::fs::read(&p) else {
        return String::new();
    };
    String::from_utf8_lossy(tail_slice(&data, bytes)).into_owned()
}

/// The last `bytes` of `data`, advanced to the next line start.
///
/// Pure, so the rule can actually be tested. The first version of this
/// lived inline in `tail` and its "tests" re-implemented the slicing
/// next to it — they would have passed against any bug, because they
/// never called the code they claimed to cover.
fn tail_slice(data: &[u8], bytes: usize) -> &[u8] {
    let start = data.len().saturating_sub(bytes);
    if start == 0 {
        // Nothing was cut, so there is no partial line to skip — and
        // skipping here would eat the genuine first line.
        return data;
    }
    // When the cut lands exactly on a line boundary (the byte before
    // `start` is already a newline), the slice opens on a complete line.
    // Skipping to the next newline anyway — which is what this did before
    // — threw away one whole valid line whenever the cut happened to
    // align that way.
    if data[start - 1] == b'\n' {
        return &data[start..];
    }
    let slice = &data[start..];
    match slice.iter().position(|b| *b == b'\n') {
        Some(i) => &slice[i + 1..],
        None => slice,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tail_never_starts_mid_line() {
        let data = b"first line\nsecond line\nthird line\n";
        // 15 bytes back lands inside "second line".
        assert_eq!(
            String::from_utf8_lossy(tail_slice(data, 15)),
            "third line\n",
            "the excerpt opened halfway through a line"
        );
    }

    #[test]
    fn a_file_shorter_than_the_window_keeps_its_first_line() {
        // The line-skip must NOT run when nothing was cut, or the whole
        // first line disappears from a short log — which is the only
        // line a crash-on-boot report has.
        let data = b"only line\nsecond\n";
        assert_eq!(
            String::from_utf8_lossy(tail_slice(data, 4096)),
            "only line\nsecond\n"
        );
    }

    /// The writer itself, against the real file.
    ///
    /// Ignored because it appends to this machine's actual `app.log` —
    /// harmless (it is a log, and the line says what wrote it) but not
    /// something the default test run should do. Everything above is
    /// pure; this is the only check that the `MakeWriter` wiring, the
    /// per-event buffering and the file path all actually line up.
    ///
    ///   cargo test --lib -- --ignored writer_really_appends
    #[test]
    #[ignore = "appends to the real app.log"]
    fn writer_really_appends_to_the_log_file() {
        use tracing_subscriber::layer::SubscriberExt as _;

        let path = path().expect("app-data dir");
        let before = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);

        let subscriber = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(AppLogWriter),
        );
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(marker = "log_file_selftest", "self-test line");
        });

        let after = std::fs::read_to_string(&path).expect("read app.log");
        assert!(
            after.len() as u64 > before,
            "the log did not grow: {} -> {}",
            before,
            after.len()
        );
        assert!(
            after.contains("log_file_selftest"),
            "the event never reached the file"
        );
    }

    #[test]
    fn a_window_landing_exactly_on_a_newline_does_not_drop_the_next_line() {
        let data = b"aaa\nbbb\nccc\n";
        // Exactly the last 8 bytes: "bbb\nccc\n" — the cut sits on the
        // newline that ends "aaa", so BOTH remaining lines are complete
        // and neither may be skipped. This assertion used to demand
        // "ccc\n", locking in the very bug the test is named after.
        assert_eq!(String::from_utf8_lossy(tail_slice(data, 8)), "bbb\nccc\n");
    }
}
