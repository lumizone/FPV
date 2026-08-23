#!/usr/bin/env bash
# FPV — dev launcher.
#
# Loads .env.local (Dodo/Polar IDs, keygen pubkey, embed model
# override, etc.) into the environment so option_env! / env! macros
# bake the right values into the dev build, then runs
# `npm run tauri dev`.
#
# .env.local is gitignored. Edit it with your live Dodo product IDs
# (or sandbox Polar UUIDs if you've flipped FPV_LICENSE_PROVIDER=polar).

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ENV_FILE="$REPO_ROOT/.env.local"

if [[ -f "$ENV_FILE" ]]; then
  echo "[dev] loading $ENV_FILE"
  # shellcheck disable=SC2046
  set -a
  # shellcheck disable=SC1090
  source "$ENV_FILE"
  set +a
else
  echo "[dev] no .env.local; running with placeholder license config (every license key will be rejected)"
fi

# Sanity check the active license-provider config so we fail loudly
# instead of silently rejecting every license redemption. The check
# set depends on FPV_LICENSE_PROVIDER (defaults to "dodo" post-v0.1.10
# migration, matching the Rust-side default in license/provider.rs).
require() {
  local name="$1"
  local value="${!name:-}"
  if [[ -z "$value" || "$value" == "00000000-0000-0000-0000-000000000000" ]]; then
    echo "[dev] WARNING: $name is unset/placeholder — license redemption will fail"
  fi
}
PROVIDER="${FPV_LICENSE_PROVIDER:-dodo}"
case "$PROVIDER" in
  dodo)
    require FPV_DODO_BASE_URL
    require FPV_DODO_PRODUCT_ID_LIFETIME
    require FPV_DODO_PRODUCT_ID_RENEWAL
    require FPV_DODO_CHECKOUT_URL_LIFETIME
    require FPV_DODO_CHECKOUT_URL_RENEWAL
    require FPV_DODO_PORTAL_URL
    ;;
  polar)
    require FPV_POLAR_BASE_URL
    require FPV_POLAR_ORG_ID
    require FPV_POLAR_BENEFIT_LIFETIME_ID
    require FPV_POLAR_BENEFIT_RENEWAL_ID
    require FPV_POLAR_CHECKOUT_LIFETIME_URL
    require FPV_POLAR_CHECKOUT_RENEWAL_URL
    require FPV_POLAR_PORTAL_URL
    ;;
  *)
    echo "[dev] WARNING: FPV_LICENSE_PROVIDER='$PROVIDER' is not recognised (expected 'dodo' or 'polar')"
    ;;
esac

cd "$REPO_ROOT"

# The shipped app has no optional speech or store feature flags. Keep an
# override for local experiments, but direct development builds use the
# same feature set as production by default.
FEATURES="${FPV_DEV_FEATURES-}"
if [[ -n "$FEATURES" ]]; then
  echo "[dev] building with cargo features: $FEATURES"
  exec npm run tauri dev -- --features "$FEATURES" "$@"
fi

exec npm run tauri dev -- "$@"
exec npm run tauri dev "$@"
