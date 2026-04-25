#!/usr/bin/env bash
# Wrapper around `assay-engine seed-sample` for the e2e fixture
# pipeline. Idempotent — safe to re-run between Playwright iterations.
#
# Usage:  fixtures/seed.sh [BASE_URL] [ADMIN_KEY]
#
# Defaults match fixtures/engine.toml + playwright.config.ts.
set -euo pipefail

BASE="${1:-http://localhost:8420}"
ADMIN_KEY="${2:-dev-admin-key-change-me}"
ENGINE_BIN="${ASSAY_ENGINE_BIN:-../../target/release/assay-engine}"

if [[ ! -x "$ENGINE_BIN" ]]; then
  # Allow callers to skip the seed if the binary isn't there yet
  # (e.g. during the very first e2e iteration on a fresh clone). The
  # console specs degrade gracefully when no fixture rows exist.
  echo "[seed] $ENGINE_BIN not found — skipping seed-sample" >&2
  exit 0
fi

"$ENGINE_BIN" seed-sample --base-url "$BASE" --admin-key "$ADMIN_KEY"
