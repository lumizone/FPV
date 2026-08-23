#!/usr/bin/env bash
# Validate the native binaries required for a direct macOS release.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN_DIR="$REPO_ROOT/src-tauri/binaries"
LOCK_FILE="$REPO_ROOT/src-tauri/binaries.lock"
TARGET="${FPV_TARGET:-aarch64-apple-darwin}"
REQUESTED_TARGET="$TARGET"

fail() {
  echo "[binaries] ERROR: $*" >&2
  exit 1
}

require_file() {
  local path="$1"
  [[ -f "$path" ]] || fail "missing required file: ${path#$REPO_ROOT/}"
  [[ -x "$path" ]] || fail "required file is not executable: ${path#$REPO_ROOT/}"
}

[[ -d "$BIN_DIR" ]] || fail "missing binary directory: ${BIN_DIR#$REPO_ROOT/}"
[[ -f "$LOCK_FILE" ]] || fail "missing binary manifest: ${LOCK_FILE#$REPO_ROOT/}"
# shellcheck disable=SC1090
source "$LOCK_FILE"
[[ "$REQUESTED_TARGET" == "$TARGET" ]] || fail "binary manifest target mismatch"

# These two files are resolved by Tauri's externalBin target convention.
require_file "$BIN_DIR/ollama-$TARGET"
require_file "$BIN_DIR/sd-cli-$TARGET"

# Ollama >= 0.30 starts llama-server as a child process. The shared runtime
# files and Metal backends must sit beside the sidecar inside the app bundle.
require_file "$BIN_DIR/llama-server"
require_file "$BIN_DIR/llama-quantize"

shopt -s nullglob
dylibs=()
while IFS= read -r library; do
  dylibs+=("$library")
done < <(find "$BIN_DIR" -type f \( -name '*.dylib' -o -name '*.so' \) -print | sort)
(( ${#dylibs[@]} > 0 )) || fail "no Ollama runtime libraries found in ${BIN_DIR#$REPO_ROOT/}"

for runtime_dir in mlx_metal_v3 mlx_metal_v4; do
  [[ -d "$BIN_DIR/$runtime_dir" ]] || fail "missing Ollama runtime directory: $runtime_dir"
done

if command -v file >/dev/null 2>&1; then
  case "$TARGET" in
    aarch64-*) expected_arch="arm64" ;;
    x86_64-*) expected_arch="x86_64" ;;
    *) fail "unsupported Apple target: $TARGET" ;;
  esac
  for binary in "$BIN_DIR/ollama-$TARGET" "$BIN_DIR/sd-cli-$TARGET" "$BIN_DIR/llama-server" "$BIN_DIR/llama-quantize"; do
    file_output="$(file "$binary")"
    [[ "$file_output" == *"Mach-O"* ]] || fail "not a macOS Mach-O binary: ${binary#$REPO_ROOT/}"
    [[ "$file_output" == *"$expected_arch"* ]] || fail "wrong architecture for $TARGET: ${binary#$REPO_ROOT/} ($file_output)"
  done
  for library in "${dylibs[@]}"; do
    file_output="$(file "$library")"
    [[ "$file_output" == *"Mach-O"* ]] || fail "not a macOS Mach-O runtime library: ${library#$REPO_ROOT/}"
    [[ "$file_output" == *"$expected_arch"* ]] || fail "wrong runtime library architecture for $TARGET: ${library#$REPO_ROOT/} ($file_output)"
  done
fi

verify_hash() {
  local path="$1" expected="$2"
  local actual
  actual="$(shasum -a 256 "$path" | cut -d ' ' -f 1)"
  [[ "$actual" == "$expected" ]] || fail "SHA-256 mismatch: ${path#$REPO_ROOT/}"
}

verify_hash "$BIN_DIR/ollama-$TARGET" "$OLLAMA_SHA256"
verify_hash "$BIN_DIR/llama-server" "$LLAMA_SERVER_SHA256"
verify_hash "$BIN_DIR/llama-quantize" "$LLAMA_QUANTIZE_SHA256"
verify_hash "$BIN_DIR/sd-cli-$TARGET" "$SD_CLI_SHA256"

runtime_tree_hash="$(
  cd "$BIN_DIR"
  {
    while IFS= read -r -d '' path; do
      shasum -a 256 "$path"
    done < <(find . -type f \( -name '*.dylib' -o -name '*.so' -o -name '*.metallib' \) -print0 | sort -z)
  } | shasum -a 256 | cut -d ' ' -f 1
)"
[[ "$runtime_tree_hash" == "$OLLAMA_RUNTIME_TREE_SHA256" ]] \
  || fail "Ollama runtime library tree SHA-256 mismatch"

if command -v otool >/dev/null 2>&1; then
  minos="$(otool -l "$BIN_DIR/sd-cli-$TARGET" | awk '/LC_BUILD_VERSION/{seen=1; next} seen && /minos/{print $2; exit}')"
  [[ -n "$minos" ]] || fail "could not read sd-cli minimum macOS version"
  awk -v actual="$minos" -v expected="$SD_CLI_MIN_MACOS" 'BEGIN { exit (actual <= expected) ? 0 : 1 }' \
    || fail "sd-cli requires macOS $minos; release target is macOS $SD_CLI_MIN_MACOS"
fi

echo "[binaries] valid: target=$TARGET, dylibs=${#dylibs[@]}"
