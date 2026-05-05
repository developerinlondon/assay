#!/usr/bin/env bash
# Wrap `npx playwright install --with-deps <browser>` in a retry loop with
# linear backoff. Survives transient apt mirror failures (Ubuntu 521s,
# Cloudflare blips) without forcing the whole CI run to be re-triggered.
#
# Usage:
#   bash scripts/playwright-install-with-retry.sh chromium

set -euo pipefail

MAX_ATTEMPTS=${MAX_ATTEMPTS:-3}
BROWSER=${1:-chromium}

for attempt in $(seq 1 "$MAX_ATTEMPTS"); do
    if npx playwright install --with-deps "$BROWSER"; then
        exit 0
    fi
    if [[ "$attempt" -lt "$MAX_ATTEMPTS" ]]; then
        delay=$((attempt * 15))
        echo "[playwright-install] attempt $attempt/$MAX_ATTEMPTS failed; retrying in ${delay}s..." >&2
        sleep "$delay"
    fi
done

echo "[playwright-install] all $MAX_ATTEMPTS attempts failed" >&2
exit 1
