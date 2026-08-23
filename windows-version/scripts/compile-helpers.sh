#!/usr/bin/env bash
# FPV — compile the native Swift helpers into standalone Mach-O
# binaries.
#
# Why: shipping `.swift` source and running it through `/usr/bin/swift`
# at runtime requires Xcode Command Line Tools on the END USER's Mac.
# Most users don't have CLT, so the swift shim pops an "install
# developer tools" dialog and every native bridge (OCR, calendar,
# reminders, Telegram voice convert) fails. Compiling here — on the
# build machine, which DOES have the toolchain — produces binaries that
# run on any clean Mac and are insulated from Swift-toolchain churn
# across macOS releases (incl. the upcoming macOS 27).
#
# Output: src-tauri/helpers/<name>  (one per scripts/<name>.swift)
# These are bundled into Contents/Resources/helpers/ via the
# `resources` entry in tauri.conf.json and codesigned by package.sh.
#
# Run standalone, or automatically via `beforeBuildCommand` and
# package.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SCRIPTS_DIR="$REPO_ROOT/src-tauri/scripts"
OUT_DIR="$REPO_ROOT/src-tauri/helpers"
INFO_PLIST="$REPO_ROOT/src-tauri/helpers-info.plist"

# Deployment target matches tauri.conf.json bundle.macOS.minimumSystemVersion.
# The scripts gate macOS 14+ APIs with `#available`, so a 13.0 target
# compiles cleanly and runs from 13.0 up.
TARGET="${FPV_HELPER_TARGET:-arm64-apple-macos13.0}"

if ! command -v swiftc >/dev/null 2>&1; then
  echo "ERROR: swiftc not found — install Xcode or Command Line Tools on the BUILD machine." >&2
  echo "  (end users do NOT need this; it's only used to pre-compile the helpers)" >&2
  exit 1
fi

mkdir -p "$OUT_DIR"

# Embed a minimal Info.plist (TCC usage strings + bundle id) directly
# into each helper's __TEXT,__info_plist section. EventKit on macOS 14+
# wants the *FullAccess* usage keys in the requesting executable; the
# embedded plist makes the calendar/reminders permission prompts robust
# instead of relying solely on responsible-process attribution.
PLIST_FLAGS=()
if [[ -f "$INFO_PLIST" ]]; then
  PLIST_FLAGS=(-Xlinker -sectcreate -Xlinker __TEXT -Xlinker __info_plist -Xlinker "$INFO_PLIST")
else
  echo "[compile-helpers] WARN: $INFO_PLIST missing; helpers built without embedded plist"
fi

shopt -s nullglob
built=0
for src in "$SCRIPTS_DIR"/*.swift; do
  name="$(basename "$src" .swift)"
  echo "[compile-helpers] swiftc -> helpers/$name  (target $TARGET)"
  swiftc -O -target "$TARGET" "${PLIST_FLAGS[@]}" -o "$OUT_DIR/$name" "$src"
  built=$((built + 1))
done

if [[ "$built" -eq 0 ]]; then
  # FPV ships no native Swift helpers — the EventKit/OCR/audio bridges
  # were Local Waifu companion-AI features, deleted along with the
  # scripts that produced them. Zero is the expected count now, not a
  # broken build; package.sh already treats a missing helpers/ dir as
  # a warning, not a failure.
  echo "[compile-helpers] no .swift helpers under $SCRIPTS_DIR — nothing to compile, skipping"
  exit 0
fi

echo "[compile-helpers] done — $built helper(s) in $OUT_DIR"
