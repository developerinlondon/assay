#!/usr/bin/env bash
#
# Usage: check-bumped.sh <crate-name>
#
# Compares the workspace Cargo.toml version for <crate-name> against the latest
# version of that crate on crates.io. Writes two lines to $GITHUB_OUTPUT:
#
#   bumped=true|false
#   version=<version-from-Cargo.toml>
#
# `bumped=true` means the local manifest is ahead of crates.io — i.e. the
# downstream publish/tag/release steps should run. `bumped=false` means this
# exact version is already published (workflow is idempotent on re-runs).
#
# Exit codes:
#   0  — decision written (bumped=true OR bumped=false)
#   1  — crate not found in workspace, or crates.io returned a status we don't know how to interpret

set -euo pipefail

crate="${1:?usage: check-bumped.sh <crate-name>}"

version=$(cargo metadata --format-version 1 --no-deps \
    | jq -r --arg c "$crate" '.packages[] | select(.name==$c) | .version')

if [ -z "$version" ] || [ "$version" = "null" ]; then
    echo "error: crate '$crate' not found in workspace metadata" >&2
    exit 1
fi

status=$(curl -sS -o /dev/null -w '%{http_code}' \
    -A 'assay-release-ci (https://github.com/developerinlondon/assay)' \
    "https://crates.io/api/v1/crates/${crate}/${version}")

case "$status" in
    200)
        bumped=false
        note="already on crates.io — no-op"
        ;;
    404)
        bumped=true
        note="not yet on crates.io — will publish"
        ;;
    *)
        echo "error: unexpected HTTP ${status} querying crates.io for ${crate}@${version}" >&2
        exit 1
        ;;
esac

# Local stdout is for humans reading the workflow log.
echo "${crate}@${version}: ${note}"

# $GITHUB_OUTPUT is the machine-readable handoff to later workflow steps.
# When run outside CI (local dry-run), GITHUB_OUTPUT is unset — tolerate that.
if [ -n "${GITHUB_OUTPUT:-}" ]; then
    echo "bumped=${bumped}" >> "$GITHUB_OUTPUT"
    echo "version=${version}" >> "$GITHUB_OUTPUT"
fi
