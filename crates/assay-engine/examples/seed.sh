#!/usr/bin/env bash
# Seed sample data into a running assay-engine instance.
#
# Usage:
#   ./examples/seed.sh [BASE_URL]
#
# Default BASE_URL is http://127.0.0.1:3000 (from examples/sqlite.toml).
# Override if you're running the engine on a different port/host.

set -euo pipefail

BASE="${1:-http://127.0.0.1:3000}"

say() { printf "\033[1;34m=>\033[0m %s\n" "$*"; }

say "target: $BASE"

# ── sanity: engine reachable? ────────────────────────────────────────────────
if ! curl -fs "$BASE/api/v1/engine/workflow/health" >/dev/null; then
  echo "error: $BASE/api/v1/engine/workflow/health unreachable — is the engine running?" >&2
  echo "  start it with:  cargo run -p assay-engine --bin assay-engine -- serve --config crates/assay-engine/examples/sqlite.toml" >&2
  exit 1
fi

# ── Namespaces (demo, prod) ──────────────────────────────────────────────────
say "creating namespaces"
for ns in demo prod; do
  curl -fs -X POST "$BASE/api/v1/engine/workflow/namespaces" \
    -H 'content-type: application/json' \
    -d "{\"name\":\"$ns\"}" >/dev/null || echo "  (namespace $ns already exists, skipping)"
done

# ── Workflows ────────────────────────────────────────────────────────────────
say "creating workflows"
NOW=$(date +%s)

for i in 1 2 3 4 5; do
  curl -fs -X POST "$BASE/api/v1/engine/workflow/workflows" \
    -H 'content-type: application/json' \
    -d "{
      \"workflow_id\":\"demo-wf-$i-$NOW\",
      \"workflow_type\":\"demo.greet\",
      \"namespace\":\"demo\",
      \"task_queue\":\"default\",
      \"input\":\"{\\\"name\\\":\\\"world-$i\\\"}\"
    }" >/dev/null && echo "  demo-wf-$i-$NOW created" || echo "  demo-wf-$i-$NOW failed"
done

for i in 1 2; do
  curl -fs -X POST "$BASE/api/v1/engine/workflow/workflows" \
    -H 'content-type: application/json' \
    -d "{
      \"workflow_id\":\"prod-job-$i-$NOW\",
      \"workflow_type\":\"prod.sync_daily\",
      \"namespace\":\"prod\",
      \"task_queue\":\"sync\",
      \"input\":\"{\\\"date\\\":\\\"2026-04-22\\\"}\"
    }" >/dev/null && echo "  prod-job-$i-$NOW created" || echo "  prod-job-$i-$NOW failed"
done

# ── Schedules ────────────────────────────────────────────────────────────────
# assay-workflow uses the `cron` crate which requires a 6-field format
# (sec min hour day month weekday), not the 5-field Unix format. The field
# is also called `cron_expr` (not `cron`), and `input` is a JSON value,
# not a string.
say "creating schedules"
curl -fs -X POST "$BASE/api/v1/engine/workflow/schedules" \
  -H 'content-type: application/json' \
  -d '{
    "namespace":"demo",
    "name":"heartbeat",
    "cron_expr":"0 * * * * *",
    "timezone":"UTC",
    "workflow_type":"demo.heartbeat",
    "task_queue":"default",
    "input":{}
  }' >/dev/null && echo "  demo/heartbeat (every minute at :00) created" \
  || echo "  demo/heartbeat failed (may already exist)"

curl -fs -X POST "$BASE/api/v1/engine/workflow/schedules" \
  -H 'content-type: application/json' \
  -d '{
    "namespace":"prod",
    "name":"daily-sync",
    "cron_expr":"0 0 2 * * *",
    "timezone":"UTC",
    "workflow_type":"prod.sync_daily",
    "task_queue":"sync",
    "input":{"scope":"all"}
  }' >/dev/null && echo "  prod/daily-sync (02:00 UTC daily) created" \
  || echo "  prod/daily-sync failed (may already exist)"

say "done"
echo
say "browse the dashboard"
echo "  open $BASE/workflow/            # Workflows tab"
echo "  open $BASE/workflow/schedules   # Schedules tab"
echo "  open $BASE/workflow/queues      # Queue stats tab"
echo
say "list via API"
echo "  curl $BASE/api/v1/engine/workflow/namespaces"
echo "  curl $BASE/api/v1/engine/workflow/workflows"
