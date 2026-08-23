#!/usr/bin/env bash
# FPV automated QA runner.
#
# Runs deterministic checks only. Real model quality, GPU rendering, Keychain
# prompts, VoiceOver, and notarization remain in the manual/release matrix.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REPORT_DIR="${QA_REPORT_DIR:-$ROOT/qa-reports}"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
REPORT="$REPORT_DIR/qa-$STAMP.md"
mkdir -p "$REPORT_DIR"

run_step() {
  local name="$1"
  shift
  printf '\n## %s\n\n```text\n' "$name" | tee -a "$REPORT"
  if "$@" 2>&1 | tee -a "$REPORT"; then
    printf '\n```\n\nPASS\n' | tee -a "$REPORT"
  else
    printf '\n```\n\nFAIL\n' | tee -a "$REPORT"
    exit 1
  fi
}

printf '# FPV QA Report\n\n- Date: %s\n- App: %s\n' "$STAMP" "$ROOT" > "$REPORT"

run_step "Frontend type-check" npm --prefix "$ROOT" exec tsc -- --noEmit
run_step "Frontend unit tests" npm --prefix "$ROOT" test -- --run
run_step "Frontend production build" npm --prefix "$ROOT" run build
run_step "i18n parity" python3 "$ROOT/scripts/i18n_check_parity.py"
run_step "i18n active-key audit" python3 "$ROOT/scripts/i18n_audit.py"
run_step "Rust backend tests" cargo test --manifest-path "$ROOT/src-tauri/Cargo.toml" --lib --quiet
run_step "Rust compile check" cargo check --manifest-path "$ROOT/src-tauri/Cargo.toml"
run_step "Shell syntax" bash -n "$ROOT/scripts/package.sh" "$ROOT/scripts/qa.sh" "$ROOT/scripts/qa-gui.sh" "$ROOT/scripts/cargo-audit.sh" "$ROOT/scripts/validate-binaries.sh" "$ROOT/scripts/write-release-manifest.sh"
run_step "Diff whitespace" git -C "$ROOT" diff --check

printf '\n## Static privacy checks\n\n' | tee -a "$REPORT"
if rg -n 'supabase|SUPABASE_URL|createClient\(' "$ROOT/src" "$ROOT/src-tauri/src"; then
  printf '\nFAIL: external database references found\n' | tee -a "$REPORT"
  exit 1
fi
printf 'PASS: no Supabase client or URL in shipped source\n' | tee -a "$REPORT"

if [[ -n "${QA_APP_BUNDLE:-}" ]]; then
  run_step "Signed app verification" codesign --verify --deep --strict "$QA_APP_BUNDLE"
fi

cat <<EOF | tee -a "$REPORT"

## Manual/release-only checks (not executed by this script)

- Long local and cloud stories with real Ollama/provider models
- Image generation on supported model/GPU combinations
- Branch asset copying, regenerate, retry, and deletion
- Keychain migration and permission prompts
- Keyboard/VoiceOver smoke test
- Gatekeeper on a clean Mac
- Apple notarization and stapling

Report: $REPORT
EOF

printf '\nQA PASS: %s\n' "$REPORT"
