#!/usr/bin/env bash
# Lua test runner — spins up a fresh assay-engine on an in-memory
# SQLite backend, runs init.lua to seed namespaces + admin user,
# spawns a worker for the e2e test, runs each *.test.lua via
# `assay run`, and tears everything down.
#
# Requires `cargo` (to build assay-engine + assay) and a Lua-capable
# `assay` binary on PATH (or as built in target/debug/).

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")"/../../.. && pwd)"
TESTS_DIR="$REPO_ROOT/crates/assay-engine/tests-lua"
INIT_LUA="$REPO_ROOT/crates/assay-engine/examples/init/init.lua"

PORT="${ASSAY_E2E_PORT:-18420}"
ADMIN_KEY="lua-tests-key-$$"
DATA_DIR="$(mktemp -d)"
ENGINE_TOML="$DATA_DIR/engine.toml"

cleanup() {
  set +e
  if [[ -n "${WORKER_PID:-}" ]]; then kill "$WORKER_PID" 2>/dev/null || true; fi
  if [[ -n "${ENGINE_PID:-}" ]]; then kill "$ENGINE_PID" 2>/dev/null || true; fi
  rm -rf "$DATA_DIR"
}
trap cleanup EXIT

cat >"$ENGINE_TOML" <<EOF
[server]
bind_addr = "127.0.0.1:$PORT"
public_url = "http://127.0.0.1:$PORT"

[backend]
type = "sqlite"
data_dir = "$DATA_DIR"

[auth]
admin_api_keys = ["$ADMIN_KEY"]

[logging]
level = "warn"
format = "pretty"
EOF

echo "==> building assay + assay-engine"
cargo build -p assay-engine -p assay-lua --bin assay-engine --bin assay >&2

ENGINE_BIN="$REPO_ROOT/target/debug/assay-engine"
ASSAY_BIN="$REPO_ROOT/target/debug/assay"

echo "==> starting engine on port $PORT"
"$ENGINE_BIN" serve --config "$ENGINE_TOML" >"$DATA_DIR/engine.log" 2>&1 &
ENGINE_PID=$!

# Wait until the engine answers /api/v1/engine/core/health.
deadline=$(( $(date +%s) + 30 ))
until curl -fs -m 1 "http://127.0.0.1:$PORT/api/v1/engine/core/health" >/dev/null 2>&1; do
  if [[ $(date +%s) -ge $deadline ]]; then
    echo "engine did not become ready" >&2
    cat "$DATA_DIR/engine.log" >&2
    exit 1
  fi
  sleep 0.2
done
echo "engine ready."

export ASSAY_ENGINE_URL="http://127.0.0.1:$PORT"
export ASSAY_ADMIN_KEY="$ADMIN_KEY"

echo "==> running init.lua"
"$ASSAY_BIN" run "$INIT_LUA" -- \
  --email admin@example.com --password lua-tests-pw

echo "==> spawning worker"
"$ASSAY_BIN" run "$TESTS_DIR/worker.lua" >"$DATA_DIR/worker.log" 2>&1 &
WORKER_PID=$!
sleep 1  # give the worker time to register

failed=0
for t in core auth workflow e2e; do
  echo "==> running $t.test.lua"
  if ! "$ASSAY_BIN" run "$TESTS_DIR/$t.test.lua"; then
    echo "  FAILED" >&2
    failed=1
  fi
done

if [[ $failed -ne 0 ]]; then
  echo "engine log tail:" >&2
  tail -50 "$DATA_DIR/engine.log" >&2
  echo "worker log tail:" >&2
  tail -20 "$DATA_DIR/worker.log" >&2
  exit 1
fi

echo "all Lua tests passed"
