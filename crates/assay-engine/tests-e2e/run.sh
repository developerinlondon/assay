#!/usr/bin/env bash
# E2E runner for the engine + auth consoles.
#
# 1. Wipe and recreate the SQLite data dir
# 2. Boot `assay-engine serve` with fixtures/engine.toml
# 3. Wait for /api/v1/engine/core/info to answer
# 4. Run examples/seed-sample/seed.lua (idempotent fixtures, via the
#    assay binary using the admin api-key break-glass)
# 5. Run Playwright against http://localhost:8420
# 6. Tear down the engine + tail logs on failure
#
# Exit status mirrors Playwright's. CI uploads the engine log on
# failure (see .github/workflows/ci.yml).
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$HERE/../../.." && pwd)"
ENGINE_BIN="${ASSAY_ENGINE_BIN:-$REPO_ROOT/target/release/assay-engine}"
DATA_DIR="${ASSAY_E2E_DATA_DIR:-/tmp/assay-engine-e2e-data}"
LOG_FILE="${ASSAY_E2E_ENGINE_LOG:-/tmp/assay-engine-e2e.log}"
CONFIG="$HERE/fixtures/engine.toml"
BASE="${ASSAY_E2E_BASE:-http://localhost:8420}"
ADMIN_KEY="${ASSAY_E2E_ADMIN_KEY:-dev-admin-key-change-me}"

say() { printf "[e2e] %s\n" "$*"; }

if [[ ! -x "$ENGINE_BIN" ]]; then
  echo "[e2e] $ENGINE_BIN not found — run \`cargo build --release -p assay-engine --features auth\` first" >&2
  exit 1
fi

# Fresh data dir so module + audit + instance fixtures land clean.
rm -rf "$DATA_DIR"
mkdir -p "$DATA_DIR"

say "starting assay-engine on :8420 (data_dir=$DATA_DIR)"
"$ENGINE_BIN" serve --config "$CONFIG" >"$LOG_FILE" 2>&1 &
ENGINE_PID=$!

cleanup() {
  say "tearing down (pid $ENGINE_PID)"
  kill "$ENGINE_PID" 2>/dev/null || true
  wait "$ENGINE_PID" 2>/dev/null || true
}
trap cleanup EXIT

# Wait for /api/v1/engine/core/info to respond — public, no auth.
say "waiting for engine to come up"
for _ in $(seq 1 30); do
  if curl -fs -m 1 "$BASE/api/v1/engine/core/info" >/dev/null 2>&1; then
    say "engine ready"
    break
  fi
  sleep 0.5
done
if ! curl -fs -m 1 "$BASE/api/v1/engine/core/info" >/dev/null 2>&1; then
  echo "[e2e] FATAL: engine never came up — tail of $LOG_FILE:" >&2
  tail -n 80 "$LOG_FILE" >&2
  exit 1
fi

# Idempotent fixture seed. The engine console specs assume a small
# set of users + OIDC clients + audit rows — seed.lua provides
# exactly that, via the new assay-lua client.
say "running seed.lua"
ASSAY_BIN="$REPO_ROOT/target/release/assay" "$HERE/fixtures/seed.sh" "$BASE" "$ADMIN_KEY"

# Hand over to Playwright. Single worker — see playwright.config.ts.
say "running playwright"
cd "$HERE"
ASSAY_E2E_BASE="$BASE" \
ASSAY_E2E_ADMIN_KEY="$ADMIN_KEY" \
  npx playwright test
