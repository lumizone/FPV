#!/usr/bin/env bash
# Write a machine-readable manifest next to a release artifact.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
if [[ $# -gt 0 ]]; then
  ARTIFACT="$1"
  OUTPUT="${2:-${ARTIFACT%.*}.release-manifest.json}"
else
  shopt -s nullglob
  artifacts=("$REPO_ROOT/src-tauri/target/aarch64-apple-darwin/release/bundle/dmg/"*.dmg)
  (( ${#artifacts[@]} > 0 )) || {
    echo "[manifest] ERROR: no DMG found; run package:release first or pass ARTIFACT [OUTPUT]" >&2
    exit 1
  }
  ARTIFACT="${artifacts[${#artifacts[@]}-1]}"
  OUTPUT="${ARTIFACT%.*}.release-manifest.json"
fi

[[ -f "$ARTIFACT" ]] || {
  echo "[manifest] ERROR: artifact does not exist: $ARTIFACT" >&2
  exit 1
}

VERSION="$(awk -F'"' '/^version = / { print $2; exit }' "$REPO_ROOT/src-tauri/Cargo.toml")"
COMMIT="$(git -C "$REPO_ROOT" rev-parse HEAD 2>/dev/null || printf 'unknown')"
SHA256="$(shasum -a 256 "$ARTIFACT" | cut -d ' ' -f 1)"
SIZE_BYTES="$(stat -f '%z' "$ARTIFACT")"
TARGET="${FPV_TARGET:-aarch64-apple-darwin}"
NOTARIZED="${FPV_NOTARIZED:-false}"

mkdir -p "$(dirname "$OUTPUT")"
python3 - "$OUTPUT" "$VERSION" "$COMMIT" "$SHA256" "$SIZE_BYTES" "$TARGET" "$NOTARIZED" "$(basename "$ARTIFACT")" <<'PY'
import json
import sys

output, version, commit, sha256, size, target, notarized, artifact = sys.argv[1:]
manifest = {
    "product": "FPV",
    "version": version,
    "target": target,
    "artifact": artifact,
    "artifact_sha256": sha256,
    "artifact_size_bytes": int(size),
    "commit": commit,
    "notarized": notarized == "true",
}
with open(output, "w", encoding="utf-8") as handle:
    json.dump(manifest, handle, indent=2)
    handle.write("\n")
PY

echo "[manifest] wrote ${OUTPUT#$REPO_ROOT/}"
