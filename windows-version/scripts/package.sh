#!/usr/bin/env bash
# FPV — production packaging.
#
# Reads secrets from .env.local. Run from any directory.
#
# Required entries in .env.local:
#   APPLE_TEAM_ID            — 10-char team identifier
#   APPLE_DEV_ID_APP         — full cert name, e.g. "Developer ID Application: NAME (TEAMID)"
#   APPLE_NOTARY_KEYCHAIN_PROFILE — preferred notarytool profile name
#   APPLE_NOTARY_APPLE_ID    — Apple ID email
#   APPLE_NOTARY_PASSWORD    — app-specific password (appleid.apple.com → app passwords)
#
# No auto-updater: the app does not self-update (tauri-plugin-updater was
# removed). Ship new versions as a fresh DMG.
#
# Architecture / pre-bundled binaries expected in src-tauri/binaries/:
#   ollama-aarch64-apple-darwin
#   sd-cli-aarch64-apple-darwin
#   libggml-*.dylib / .so + mlx_metal_v*

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

# Auto-load .env.local so the operator only has to remember `./scripts/package.sh`.
if [[ -f .env.local ]]; then
  set -a
  source .env.local
  set +a
fi

if [[ -z "${APPLE_TEAM_ID:-}" || -z "${APPLE_DEV_ID_APP:-}" ]]; then
  echo "ERROR: set APPLE_TEAM_ID and APPLE_DEV_ID_APP in .env.local" >&2
  exit 1
fi

# Tauri reads APPLE_SIGNING_IDENTITY (not APPLE_DEV_ID_APP). Mirror it.
export APPLE_SIGNING_IDENTITY="$APPLE_DEV_ID_APP"

TARGET="aarch64-apple-darwin"
BUNDLE_DIR="$REPO_ROOT/src-tauri/target/$TARGET/release/bundle"
APP="$BUNDLE_DIR/macos/FPV.app"
DMG_DIR="$BUNDLE_DIR/dmg"

# 1. Make sure sidecars are present.
if [[ ! -d "$REPO_ROOT/src-tauri/binaries" || -z "$(ls -A "$REPO_ROOT/src-tauri/binaries" 2>/dev/null | grep -v '^\.gitkeep$')" ]]; then
  echo "[package] sidecars missing, fetching..."
  "$REPO_ROOT/scripts/fetch-binaries.sh"
fi

"$REPO_ROOT/scripts/validate-binaries.sh"

# 2. Pre-sign every dylib + .so the bundled sidecars depend on.
#    Without runtime + timestamp on these, the .app's deep verify fails
#    later and Apple notarization rejects the DMG.
echo "[package] codesigning bundled libs"
find "$REPO_ROOT/src-tauri/binaries" \( -name '*.dylib' -o -name '*.so' \) -print0 \
  | xargs -0 -I{} codesign --force --options runtime --timestamp \
      --sign "$APPLE_DEV_ID_APP" "{}"

# 3. Build the Tauri app. Cargo features here are the production set.
#    `--bundles app` skips Tauri's built-in DMG bundler — that one
#    shells out to AppleScript to lay out icons in Finder, which fails
#    with TCC -1743 ("Brak autoryzacji do wysłania zdarzeń Apple do
#    Finder.") whenever the parent process isn't pre-approved in
#    System Settings → Privacy & Security → Automation. We build the
#    DMG ourselves below with --sandbox-safe to dodge that whole
#    permission dance. No `updater` target: tauri-plugin-updater was
#    removed, the app doesn't self-update — ship new versions as a
#    fresh DMG.
echo "[package] building tauri release (5-15 min)"
# No feature flags: voice, Telegram, and the rest of the Local Waifu
# companion-AI surface were stripped, and the two optional features that
# outlived it (`hid`, `encrypt-db`) were removed from Cargo.toml — neither
# gated any code that still exists in FPV.
FPV_KEYGEN_ACCOUNT_ID="${FPV_KEYGEN_ACCOUNT_ID:-}" \
FPV_KEYGEN_PUBKEY_HEX="${FPV_KEYGEN_PUBKEY_HEX:-}" \
  npm run tauri build -- --target "$TARGET" --bundles app

if [[ ! -d "$APP" ]]; then
  echo "ERROR: .app not produced at $APP" >&2
  exit 1
fi

# 3b. Stage Ollama's runtime next to the bundled `ollama` sidecar.
#     Ollama >= 0.30 spawns a separate `llama-server` (plus shared
#     libs + the MLX metal backends) for inference. Tauri's externalBin
#     only bundles the single `ollama` binary, so without this step the
#     shipped app lists models fine but EVERY chat reply is empty
#     ("llama-server binary not found"). The sidecar searches its own
#     dir (Contents/MacOS) first, so copy them there.
OLLAMA_RT_DST="$APP/Contents/MacOS"
echo "[package] staging Ollama runtime (llama-server + libs + mlx) + sd-cli into the bundle"
for f in llama-server llama-quantize; do
  cp "$REPO_ROOT/src-tauri/binaries/$f" "$OLLAMA_RT_DST/"
done
cp "$REPO_ROOT/src-tauri/binaries/sd-cli-$TARGET" "$OLLAMA_RT_DST/sd-cli"
find "$REPO_ROOT/src-tauri/binaries" -maxdepth 1 \( -name '*.dylib' -o -name '*.so' \) \
  -exec cp {} "$OLLAMA_RT_DST/" \;
for d in mlx_metal_v3 mlx_metal_v4; do
  if [[ -d "$REPO_ROOT/src-tauri/binaries/$d" ]]; then
    rm -rf "$OLLAMA_RT_DST/$d"
    cp -R "$REPO_ROOT/src-tauri/binaries/$d" "$OLLAMA_RT_DST/"
  fi
done

# 4. Re-sign the bundle inside-out. Tauri's bundler signs each binary
#    in pieces and the result sometimes fails Apple notary's stricter
#    validation ("The signature of the binary is invalid"). A fresh
#    pass with --options runtime + timestamp + entitlements always
#    passes — and is cheap, so we always do it.
echo "[package] re-signing bundle inside-out"
# 4a. Native Swift helpers (Vision OCR, EventKit calendar/reminders,
#     AVFoundation audio convert). Compiled by compile-helpers.sh via
#     the beforeBuildCommand and bundled into Resources/helpers/. They
#     are standalone Mach-O executables, so each needs its own hardened-
#     runtime signature or `codesign --verify --deep --strict` (and
#     Apple notary) rejects the bundle. Sign these FIRST — inside-out.
HELPERS_DIR="$APP/Contents/Resources/helpers"
if [[ -d "$HELPERS_DIR" ]]; then
  for helper in "$HELPERS_DIR"/*; do
    [[ -f "$helper" ]] || continue
    echo "[package]   signing helper $(basename "$helper")"
    codesign --force --options runtime --timestamp \
      --sign "$APPLE_DEV_ID_APP" "$helper"
  done
else
  echo "[package] WARN: $HELPERS_DIR missing — native bridges (OCR/calendar/voice) won't work in this DMG" >&2
fi
# Ollama runtime staged in step 3b — sign libs + MLX dylibs first, then
# the helper executables, so the deep verify below passes. (.metallib
# files are data, not Mach-O, so they're skipped.)
echo "[package] signing staged Ollama runtime"
find "$OLLAMA_RT_DST" -maxdepth 1 \( -name '*.dylib' -o -name '*.so' \) -print0 \
  | xargs -0 -I{} codesign --force --options runtime --timestamp \
      --sign "$APPLE_DEV_ID_APP" "{}"
for d in mlx_metal_v3 mlx_metal_v4; do
  [[ -d "$OLLAMA_RT_DST/$d" ]] && find "$OLLAMA_RT_DST/$d" -type f -print0 \
    | xargs -0 -I{} codesign --force --options runtime --timestamp \
        --sign "$APPLE_DEV_ID_APP" "{}"
done
for f in llama-server llama-quantize sd-cli; do
  codesign --force --options runtime --timestamp \
    --sign "$APPLE_DEV_ID_APP" "$OLLAMA_RT_DST/$f"
done
codesign --force --options runtime --timestamp \
  --sign "$APPLE_DEV_ID_APP" \
  "$APP/Contents/MacOS/ollama"
codesign --force --options runtime --timestamp \
  --sign "$APPLE_DEV_ID_APP" \
  --entitlements src-tauri/entitlements.plist \
  "$APP/Contents/MacOS/fpv-desktop"
codesign --force --options runtime --timestamp \
  --sign "$APPLE_DEV_ID_APP" \
  --entitlements src-tauri/entitlements.plist \
  "$APP"

codesign --verify --deep --strict --verbose=2 "$APP" >/dev/null

# 5. DMG. We used to drive this through bundle_dmg.sh (create-dmg),
#    which lays out icons via Finder AppleScript and fails with TCC
#    error -1743 ("Brak autoryzacji do wysłania zdarzeń Apple do
#    Finder.") whenever the parent shell hasn't been granted
#    Automation → Finder. dmgbuild (Python `ds_store` lib) writes the
#    `.DS_Store` directly, so the chevron-on-pastel layout works in
#    any shell — CI, a fresh clone, a remote SSH session, you name it.
#    No system prompts, no one-off TCC grants.
#
#    dmgbuild is required. A release must never silently switch to a
#    different DMG layout or an untested fallback implementation.
echo "[package] building DMG"
VERSION="$(grep -m1 '^version = ' "$REPO_ROOT/src-tauri/Cargo.toml" | sed 's/.*"\(.*\)".*/\1/')"
DMG="$DMG_DIR/FPV_${VERSION}_$(uname -m | sed s/arm64/aarch64/).dmg"
# Tauri only creates the dmg/ subdir when `--bundles dmg` is part of
# the build (we ship with `--bundles app` and roll our own DMG below).
# After a cargo clean the whole `target/` tree is fresh and
# `bundle/dmg/` doesn't exist yet — dmgbuild's NamedTemporaryFile fails
# the moment it tries to write a scratch file there. mkdir first.
mkdir -p "$DMG_DIR"
rm -f "$DMG"
find "$DMG_DIR" -name "rw.*.dmg" -delete 2>/dev/null || true

DMG_BACKGROUND="$REPO_ROOT/src-tauri/dmg/background.png"
DMG_SETTINGS="$REPO_ROOT/src-tauri/dmg/dmg-settings.py"

# Resolve dmgbuild: prefer $DMGBUILD env override, then PATH, then
# pip's user-script dir (where `pip3 install --user dmgbuild` puts
# the binary on macOS).
DMGBUILD_BIN="${DMGBUILD:-}"
if [[ -z "$DMGBUILD_BIN" ]]; then
  if command -v dmgbuild >/dev/null 2>&1; then
    DMGBUILD_BIN="$(command -v dmgbuild)"
  elif [[ -x "$HOME/Library/Python/3.9/bin/dmgbuild" ]]; then
    DMGBUILD_BIN="$HOME/Library/Python/3.9/bin/dmgbuild"
  fi
fi

[[ -n "$DMGBUILD_BIN" ]] || {
  echo "ERROR: dmgbuild is required; install with: python3 -m pip install dmgbuild" >&2
  exit 1
}

echo "[package] using dmgbuild ($DMGBUILD_BIN) with chevron background"
"$DMGBUILD_BIN" \
  -s "$DMG_SETTINGS" \
  -D "app=$APP" \
  -D "bg=$DMG_BACKGROUND" \
  "FPV" "$DMG"

if [[ ! -f "$DMG" ]]; then
  echo "ERROR: DMG not produced at $DMG" >&2
  exit 1
fi
echo "[package] dmg: $DMG"

# 6. Sign the DMG itself (Tauri only signs the .app inside).
echo "[package] codesigning DMG"
codesign --force --options runtime --timestamp \
  --sign "$APPLE_DEV_ID_APP" "$DMG"

# 7. Notarize. Blocks until Apple finishes (5-30 min, longer for
#    first-time submissions). Add --verbose to see polling progress.
#
#    Local-only builds skip this entirely: set FPV_SKIP_NOTARIZE=1 to
#    produce a Developer-ID-signed DMG without ever contacting Apple's
#    notary service. The app still runs locally; it just isn't stapled
#    for frictionless distribution to other machines.
if [[ "${FPV_SKIP_NOTARIZE:-0}" == "1" ]]; then
  echo "[package] FPV_SKIP_NOTARIZE=1 — signed local build, NOT contacting Apple (no notarize/staple)"
  FPV_NOTARIZED=false
else
  FPV_NOTARIZED=true
fi

if [[ "$FPV_NOTARIZED" == "true" ]]; then
  echo "[package] notarizing (5-30 min)"
  if [[ -n "${APPLE_NOTARY_KEYCHAIN_PROFILE:-}" ]]; then
    xcrun notarytool submit "$DMG" \
      --keychain-profile "$APPLE_NOTARY_KEYCHAIN_PROFILE" \
      --wait
  elif [[ -n "${APPLE_NOTARY_APPLE_ID:-}" && -n "${APPLE_NOTARY_PASSWORD:-}" ]]; then
    xcrun notarytool submit "$DMG" \
      --apple-id "$APPLE_NOTARY_APPLE_ID" \
      --password "$APPLE_NOTARY_PASSWORD" \
      --team-id "$APPLE_TEAM_ID" \
      --wait
  else
    echo "ERROR: configure APPLE_NOTARY_KEYCHAIN_PROFILE or APPLE_NOTARY_APPLE_ID + APPLE_NOTARY_PASSWORD" >&2
    exit 1
  fi

  echo "[package] stapling ticket"
  xcrun stapler staple "$DMG"
  xcrun stapler validate "$DMG"

  echo "[package] verifying Gatekeeper acceptance"
  spctl -a -t open --context context:primary-signature -v "$DMG"
else
  echo "[package] verifying local code signature"
  codesign --verify --deep --strict --verbose=2 "$DMG" >/dev/null
fi

FPV_NOTARIZED="$FPV_NOTARIZED" \
  "$REPO_ROOT/scripts/write-release-manifest.sh" "$DMG" "$DMG_DIR/release-manifest.json"

echo "[package] DONE: $DMG"
