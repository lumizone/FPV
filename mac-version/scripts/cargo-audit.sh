#!/usr/bin/env bash
# RustSec policy gate for FPV's transitive desktop platform stack.
#
# anyhow/event-listener are not waived: Cargo.lock must keep them at versions
# without known unsound advisories. The remaining IDs are GTK3/proc-macro/unic
# transitive dependencies pulled by Tauri's current WebKit/tray stack; they are
# unmaintained (or the GTK glib iterator advisory) and cannot be removed without
# changing the desktop runtime. Keep this list explicit and review on upgrades.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LOCK="$ROOT/src-tauri/Cargo.lock"

cargo audit --no-fetch --stale --deny warnings --file "$LOCK" \
  --ignore RUSTSEC-2024-0411 \
  --ignore RUSTSEC-2024-0412 \
  --ignore RUSTSEC-2024-0413 \
  --ignore RUSTSEC-2024-0414 \
  --ignore RUSTSEC-2024-0415 \
  --ignore RUSTSEC-2024-0416 \
  --ignore RUSTSEC-2024-0417 \
  --ignore RUSTSEC-2024-0418 \
  --ignore RUSTSEC-2024-0419 \
  --ignore RUSTSEC-2024-0420 \
  --ignore RUSTSEC-2024-0429 \
  --ignore RUSTSEC-2024-0370 \
  --ignore RUSTSEC-2025-0075 \
  --ignore RUSTSEC-2025-0080 \
  --ignore RUSTSEC-2025-0081 \
  --ignore RUSTSEC-2025-0098 \
  --ignore RUSTSEC-2025-0100
