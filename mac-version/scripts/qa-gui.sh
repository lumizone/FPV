#!/usr/bin/env bash
# Minimal macOS GUI smoke test for the packaged application.
# Requires Accessibility permission for the calling terminal if System Events
# cannot inspect the FPV window.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP_BUNDLE="${QA_APP_BUNDLE:-$ROOT/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/FPV.app}"
TIMEOUT="${QA_GUI_TIMEOUT:-30}"

if [[ ! -d "$APP_BUNDLE" ]]; then
  printf 'GUI QA FAIL: app bundle not found: %s\n' "$APP_BUNDLE" >&2
  exit 1
fi

if ! codesign --verify --deep --strict "$APP_BUNDLE"; then
  printf 'GUI QA FAIL: app signature is invalid\n' >&2
  exit 1
fi

open -na "$APP_BUNDLE"
cleanup() {
  osascript -e 'tell application "FPV" to quit' >/dev/null 2>&1 || true
}
trap cleanup EXIT

for ((second = 1; second <= TIMEOUT; second++)); do
  if pgrep -f "$APP_BUNDLE/Contents/MacOS/fpv-desktop" >/dev/null 2>&1; then
    if osascript -e 'tell application "System Events" to tell process "FPV" to return exists window 1' 2>/dev/null | rg -q '^true$'; then
      printf 'GUI QA PASS: FPV window opened in %ss\n' "$second"
      exit 0
    fi
  fi
  sleep 1
done

printf 'GUI QA FAIL: FPV window did not open within %ss\n' "$TIMEOUT" >&2
exit 1
