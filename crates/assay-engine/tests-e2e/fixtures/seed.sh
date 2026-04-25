#!/usr/bin/env bash
# Seed wrapper for the e2e fixture pipeline. Idempotent — safe to
# re-run between Playwright iterations.
#
# Replaces the old `assay-engine seed-sample` Rust subcommand (retired
# in plan-15 slice 5). Calls examples/seed-sample/seed.lua via the
# assay-lua client runtime, using the admin api-key break-glass.
#
# Usage:  fixtures/seed.sh [BASE_URL] [ADMIN_KEY]
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$HERE/../../../.." && pwd)"

BASE="${1:-http://localhost:8420}"
ADMIN_KEY="${2:-dev-admin-key-change-me}"
ASSAY_BIN="${ASSAY_BIN:-$REPO_ROOT/target/release/assay}"
SEED_LUA="$REPO_ROOT/crates/assay-engine/examples/seed-sample/seed.lua"

if [[ ! -x "$ASSAY_BIN" ]]; then
  # Allow callers to skip when the binary isn't there yet (very first
  # e2e iteration on a fresh clone). Console specs degrade gracefully
  # when no fixture rows exist.
  echo "[seed] $ASSAY_BIN not found — skipping seed.lua" >&2
  exit 0
fi

ASSAY_ENGINE_URL="$BASE" \
ASSAY_ADMIN_KEY="$ADMIN_KEY" \
  "$ASSAY_BIN" run "$SEED_LUA"
