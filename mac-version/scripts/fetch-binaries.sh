#!/usr/bin/env bash
# FPV - fetch native release dependencies into src-tauri/binaries/.
# Run after a fresh clone or whenever you need to refresh bundled Ollama.
# The sd-cli source is supplied separately because it is built from the
# pinned stable-diffusion.cpp release artifact, not downloaded from Ollama.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN_DIR="$REPO_ROOT/src-tauri/binaries"
LOCK_FILE="$REPO_ROOT/src-tauri/binaries.lock"
mkdir -p "$BIN_DIR"

# Do not retain versioned libraries from an older architecture/version. They
# are otherwise copied into the bundle forever and can shadow the fresh
# runtime at load time.
find "$BIN_DIR" -maxdepth 1 -type f \( -name '*.dylib' -o -name '*.so' \) -delete

[[ -f "$LOCK_FILE" ]] || { echo "[fetch] ERROR: missing binaries.lock" >&2; exit 1; }
# shellcheck disable=SC1090
source "$LOCK_FILE"

OLLAMA_VERSION="${OLLAMA_VERSION}"

# Optional path to a locally built or downloaded, verified sd-cli binary.
# The package validator will fail if this is not supplied and the target file
# is not already present in the ignored binaries directory.

if [[ -n "${FPV_SD_CLI_SOURCE:-}" ]]; then
  [[ -f "$FPV_SD_CLI_SOURCE" ]] || {
    echo "[fetch] ERROR: FPV_SD_CLI_SOURCE does not exist: $FPV_SD_CLI_SOURCE" >&2
    exit 1
  }
  [[ -n "${FPV_SD_CLI_SHA256:-}" ]] || {
    echo "[fetch] ERROR: FPV_SD_CLI_SHA256 is required when staging sd-cli" >&2
    exit 1
  }
fi

echo "[fetch] Ollama ${OLLAMA_VERSION} -> $BIN_DIR"
TMP="$(mktemp -d)"
curl -fsSL "$OLLAMA_URL" -o "$TMP/ollama-darwin.tgz"
tar xzf "$TMP/ollama-darwin.tgz" -C "$TMP"

# Tauri sidecar naming convention: <name>-<rust-target-triple>
HOST_TRIPLE="$(rustc -vV | grep '^host:' | awk '{print $2}')"
cp "$TMP/ollama" "$BIN_DIR/ollama-${HOST_TRIPLE}"
chmod +x "$BIN_DIR/ollama-${HOST_TRIPLE}"
actual="$(shasum -a 256 "$BIN_DIR/ollama-${HOST_TRIPLE}" | cut -d ' ' -f 1)"
[[ "$actual" == "$OLLAMA_SHA256" ]] || { echo "[fetch] ERROR: Ollama SHA-256 mismatch" >&2; exit 1; }

if [[ -n "${FPV_SD_CLI_SOURCE:-}" ]]; then
  echo "[fetch] sd-cli source -> $BIN_DIR/sd-cli-${HOST_TRIPLE}"
  cp "$FPV_SD_CLI_SOURCE" "$BIN_DIR/sd-cli-${HOST_TRIPLE}"
  chmod +x "$BIN_DIR/sd-cli-${HOST_TRIPLE}"
  actual="$(shasum -a 256 "$BIN_DIR/sd-cli-${HOST_TRIPLE}" | cut -d ' ' -f 1)"
  [[ "$actual" == "$FPV_SD_CLI_SHA256" ]] || {
    echo "[fetch] ERROR: sd-cli SHA-256 mismatch: expected $FPV_SD_CLI_SHA256, got $actual" >&2
    exit 1
  }
fi

# Ollama >= 0.30 split inference into a separate `llama-server` binary
# (plus `llama-quantize`) that the main `ollama` process spawns at
# generation time. Without it `/api/chat` fails with "llama-server
# binary not found" while `/api/tags` + model listing still work — so
# the app launches and lists models but every reply comes back EMPTY.
# Stage them next to the sidecar (package.sh bundles them into the
# .app's Contents/MacOS so the shipped `ollama` finds them too).
for exe in llama-server llama-quantize; do
  if [[ -f "$TMP/$exe" ]]; then
    cp "$TMP/$exe" "$BIN_DIR/$exe"
    chmod +x "$BIN_DIR/$exe"
  fi
done
for entry in "llama-server:$LLAMA_SERVER_SHA256" "llama-quantize:$LLAMA_QUANTIZE_SHA256"; do
  exe="${entry%%:*}" expected="${entry##*:}"
  actual="$(shasum -a 256 "$BIN_DIR/$exe" | cut -d ' ' -f 1)"
  [[ "$actual" == "$expected" ]] || { echo "[fetch] ERROR: $exe SHA-256 mismatch" >&2; exit 1; }
done

# Ollama's GGML and MLX backends are loaded at runtime from the binary's
# directory. Stage them next to the sidecar so dev-mode `tauri dev`
# can spawn it cleanly.
for f in "$TMP"/*.dylib "$TMP"/*.so; do
  [[ -f "$f" ]] && cp "$f" "$BIN_DIR/"
done
for d in mlx_metal_v3 mlx_metal_v4; do
  if [[ -d "$TMP/$d" ]]; then
    rm -rf "$BIN_DIR/$d"
    cp -R "$TMP/$d" "$BIN_DIR/"
  fi
done

# An arm64 release must not carry x86-only runtime plugins. Universal Mach-O
# files are retained; optional x86 CPU backends are safely omitted because the
# target process cannot load them.
if command -v file >/dev/null 2>&1 && [[ "$HOST_TRIPLE" == aarch64-* ]]; then
  for runtime_file in "$BIN_DIR"/*.dylib "$BIN_DIR"/*.so; do
    [[ -f "$runtime_file" ]] || continue
    if [[ "$(file "$runtime_file")" != *"arm64"* ]]; then
      rm -f "$runtime_file"
    fi
  done
fi

xattr -cr "$BIN_DIR" || true
rm -rf "$TMP"

echo "[fetch] done. size: $(du -sh "$BIN_DIR" | awk '{print $1}')"
