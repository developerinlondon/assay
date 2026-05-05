#!/usr/bin/env bash
# E2E runner for libs/sysops dashboard.
#
# 1. Boot `assay tests-e2e/boot.lua` (mounts sysops with stubs).
# 2. Wait for /__e2e_alive on $E2E_PORT.
# 3. Run Playwright.
# 4. Tear down.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$HERE/../../.." && pwd)"
ASSAY_BIN="${ASSAY_BIN:-$REPO_ROOT/target/release/assay}"
LOG_FILE="${SYSOPS_E2E_LOG:-/tmp/sysops-e2e.log}"
PORT="${E2E_PORT:-47921}"
BASE="http://127.0.0.1:$PORT"

say() { printf "[sysops-e2e] %s\n" "$*"; }

if [[ ! -x "$ASSAY_BIN" ]]; then
  echo "[sysops-e2e] $ASSAY_BIN not found — run \`cargo build --release -p assay-lua\` first" >&2
  exit 1
fi

cd "$REPO_ROOT"

LUA_PATH='libs/?.lua;libs/?/init.lua;libs/sysops/?.lua;libs/sysops/tests-lua/?.lua;libs/sysops/tests-e2e/?.lua;;'
export LUA_PATH

say "booting sysops on :$PORT"
E2E_PORT="$PORT" "$ASSAY_BIN" libs/sysops/tests-e2e/boot.lua >"$LOG_FILE" 2>&1 &
BOOT_PID=$!

cleanup() {
  say "tearing down (pid $BOOT_PID)"
  kill "$BOOT_PID" 2>/dev/null || true
  wait "$BOOT_PID" 2>/dev/null || true
}
trap cleanup EXIT

say "waiting for $BASE/__e2e_alive"
for _ in $(seq 1 30); do
  if curl -fs -m 1 "$BASE/__e2e_alive" >/dev/null 2>&1; then
    say "ready"
    break
  fi
  sleep 0.5
done
if ! curl -fs -m 1 "$BASE/__e2e_alive" >/dev/null 2>&1; then
  echo "[sysops-e2e] FATAL: did not come up — tail of $LOG_FILE:" >&2
  tail -n 80 "$LOG_FILE" >&2
  exit 1
fi

cd "$HERE"

if [[ ! -d node_modules ]]; then
  say "installing playwright"
  npm ci
  npx playwright install chromium --with-deps
fi

say "running playwright"
SYSOPS_E2E_BASE="$BASE" npx playwright test "$@"
