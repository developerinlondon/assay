#!/usr/bin/env bash
# scripts/smoke-examples.sh — boots / validates every shipped example
# artifact and reports pass/fail.
#
# Exit codes:
#   0  all cases passed (or only XFAIL cases failed as expected)
#   1  at least one unexpected failure
#
# XFAIL cases (known bugs fixed on wt/docs — do NOT cause exit 1):
#   A) examples/workflows/*/worker.lua: require("assay.workflow") — module
#      renamed to assay.engine.workflow.
#      TODO: remove XFAIL once wt/docs is merged.
#   B) engine/examples/sqlite.toml: ${VAR} literal in TOML comment triggers
#      the env-expander (expander runs before TOML parsing, comments not
#      stripped). Engine exits with "env var VAR not set".
#      TODO: remove XFAIL once wt/docs env-expander comment-skip fix lands.
#
# Usage:
#   ./scripts/smoke-examples.sh [--engine-bin PATH] [--assay-bin PATH]
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# ---------- defaults --------------------------------------------------------
_DEFAULT_TARGET="${CARGO_TARGET_DIR:-$REPO_ROOT/target}"
# Prefer server-release for the engine; fall back to release
if [[ -x "$_DEFAULT_TARGET/server-release/assay-engine" ]]; then
    ENGINE_BIN="${ENGINE_BIN:-$_DEFAULT_TARGET/server-release/assay-engine}"
else
    ENGINE_BIN="${ENGINE_BIN:-$_DEFAULT_TARGET/release/assay-engine}"
fi
ASSAY_BIN="${ASSAY_BIN:-$_DEFAULT_TARGET/release/assay}"

ENGINE_TIMEOUT="${ENGINE_TIMEOUT:-10}"   # seconds to wait for engine bind

# ---------- arg parsing -----------------------------------------------------
while [[ $# -gt 0 ]]; do
    case "$1" in
        --engine-bin) ENGINE_BIN="$2"; shift 2 ;;
        --assay-bin)  ASSAY_BIN="$2";  shift 2 ;;
        *) echo "Unknown argument: $1" >&2; exit 1 ;;
    esac
done

# ---------- counters ---------------------------------------------------------
PASS=0
FAIL=0
XFAIL=0   # expected failures (known bugs on wt/docs branch)
ENGINE_PID=""

cleanup() {
    if [[ -n "$ENGINE_PID" ]] && kill -0 "$ENGINE_PID" 2>/dev/null; then
        kill "$ENGINE_PID" 2>/dev/null || true
        wait "$ENGINE_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT

# ---------- helpers ----------------------------------------------------------
pass()  { echo "  PASS  $1"; ((PASS++))  || true; }
fail()  { echo "  FAIL  $1: $2"; ((FAIL++)) || true; }
xfail() { echo "  XFAIL $1: $2"; ((XFAIL++)) || true; }
skip()  { echo "  SKIP  $1: $2"; }

# Boot the engine with a config, wait until it binds or fails.
# Sets ENGINE_PID. Returns 0 on success, 1 on error.
# Caller must call kill_engine when done.
boot_engine() {
    local config="$1" port="$2"
    local logfile
    logfile="$(mktemp /tmp/assay-engine-smoke-XXXXXX.log)"

    "$ENGINE_BIN" serve --config "$config" >"$logfile" 2>&1 &
    ENGINE_PID=$!

    local waited=0
    while (( waited < ENGINE_TIMEOUT )); do
        if nc -z 127.0.0.1 "$port" 2>/dev/null; then
            rm -f "$logfile"
            return 0
        fi
        if ! kill -0 "$ENGINE_PID" 2>/dev/null; then
            echo "    engine exited early; last log lines:" >&2
            tail -5 "$logfile" >&2
            rm -f "$logfile"
            return 1
        fi
        sleep 1
        ((waited++)) || true
    done
    echo "    engine did not bind on :$port within ${ENGINE_TIMEOUT}s; last log:" >&2
    tail -5 "$logfile" >&2
    rm -f "$logfile"
    return 1
}

kill_engine() {
    if [[ -n "$ENGINE_PID" ]] && kill -0 "$ENGINE_PID" 2>/dev/null; then
        kill "$ENGINE_PID" 2>/dev/null || true
        wait "$ENGINE_PID" 2>/dev/null || true
    fi
    ENGINE_PID=""
}

# ---------- Section 1: engine config smoke ----------------------------------
echo ""
echo "=== Engine config smoke ==="
echo "    engine binary: $ENGINE_BIN"

if [[ ! -x "$ENGINE_BIN" ]]; then
    skip "engine configs" "assay-engine binary not found at $ENGINE_BIN — run: cargo build --profile server-release -p assay-engine"
else
    # sqlite.toml — XFAIL because the comment on line 14 contains the literal
    # text '${VAR}' (inside backticks in a TOML # comment).  The env-expander
    # runs on the raw file before TOML parsing so it sees the token, looks up
    # VAR, finds nothing, and aborts.  Fix is on wt/docs (skip TOML comments
    # in expand_env_vars).
    # TODO: flip to a real boot test once wt/docs is merged.
    CONFIG_SQLITE="$REPO_ROOT/crates/assay-engine/examples/sqlite.toml"
    TMPDATA="$(mktemp -d /tmp/assay-smoke-data-XXXXXX)"
    if DATA_DIR="$TMPDATA" boot_engine "$CONFIG_SQLITE" 3000; then
        pass "engine/examples/sqlite.toml (boot + bind)"
        kill_engine
    else
        kill_engine
        # Confirm this is the known comment-bug and not a new failure
        err=$( DATA_DIR="$TMPDATA" "$ENGINE_BIN" serve --config "$CONFIG_SQLITE" 2>&1 || true )
        if echo "$err" | grep -q 'env var.*VAR.*not set\|expand env vars.*VAR'; then
            # TODO: remove xfail once wt/docs env-expander comment-skip fix is merged
            xfail "engine/examples/sqlite.toml" "env-expander hits \${VAR} in TOML comment (wt/docs)"
        else
            fail "engine/examples/sqlite.toml" "$err"
        fi
    fi
    rm -rf "$TMPDATA"

    # postgres.toml — skip when DATABASE_URL is absent (CI sets it for PG jobs)
    CONFIG_PG="$REPO_ROOT/crates/assay-engine/examples/postgres.toml"
    if [[ -z "${DATABASE_URL:-}" ]]; then
        skip "engine/examples/postgres.toml" "DATABASE_URL not set"
    else
        if DATABASE_URL="$DATABASE_URL" PUBLIC_URL="${PUBLIC_URL:-http://localhost:3000}" \
           boot_engine "$CONFIG_PG" 3000; then
            pass "engine/examples/postgres.toml (boot + bind)"
        else
            fail "engine/examples/postgres.toml" "engine failed to start/bind"
        fi
        kill_engine
    fi
fi

# ---------- Section 2: Lua module resolution check -------------------------
# For each shipped Lua example we verify that its top-level require() calls
# resolve against the bundled stdlib.  This catches module renames (e.g.
# assay.workflow → assay.engine.workflow) without running any network code.
echo ""
echo "=== Lua module resolution check ==="
echo "    assay binary: $ASSAY_BIN"

if [[ ! -x "$ASSAY_BIN" ]]; then
    skip "Lua examples" "assay binary not found at $ASSAY_BIN — run: cargo build --release -p assay-lua"
else
    # Extract `require("...")` calls from a Lua file and test each one
    check_lua_requires() {
        local file="$1"
        local label="$2"
        local xfail_pattern="${3:-}"   # if non-empty, match against error to gate XFAIL

        # Collect unique quoted module names from require() calls
        local modules
        modules=$(grep -oP "require\(['\"]\\K[^'\"]+(?=['\"])" "$file" | sort -u)

        if [[ -z "$modules" ]]; then
            pass "$label (no require calls)"
            return
        fi

        local all_ok=1
        local first_err=""
        while IFS= read -r mod; do
            local result exit_code
            result=$("$ASSAY_BIN" exec -e "require('$mod')" 2>&1) || true
            exit_code=$("$ASSAY_BIN" exec -e "require('$mod')" 2>/dev/null; echo $?) 2>/dev/null || true
            # A successful require exits 0; a failed one exits non-zero or prints ERROR
            if echo "$result" | grep -qE 'ERROR|not found|module.*not found'; then
                all_ok=0
                # Extract just the key error line, not the full traceback
                first_err="$mod: $(echo "$result" | grep -E 'not found|module.*not found' | head -1 | sed 's/.*ERROR[[:space:]]*//')"
                break
            fi
        done <<< "$modules"

        if [[ "$all_ok" -eq 1 ]]; then
            pass "$label"
        elif [[ -n "$xfail_pattern" ]] && echo "$first_err" | grep -qE "$xfail_pattern"; then
            xfail "$label" "$first_err"
        else
            fail "$label" "$first_err"
        fi
    }

    # Top-level standalone examples
    for lua_file in "$REPO_ROOT"/examples/*.lua; do
        name="examples/$(basename "$lua_file")"
        check_lua_requires "$lua_file" "$name"
    done

    # Workflow worker examples — all use require("assay.workflow") which was
    # renamed to assay.engine.workflow (tracked in wt/docs).
    # TODO: remove xfail pattern once wt/docs is merged.
    for lua_file in "$REPO_ROOT"/examples/workflows/*/worker.lua; do
        wf_name="$(basename "$(dirname "$lua_file")")"
        name="examples/workflows/$wf_name/worker.lua"
        check_lua_requires "$lua_file" "$name" "assay\.workflow.*not found|module.*assay\.workflow"
    done
fi

# ---------- Section 3: checks.yaml parse validation -------------------------
echo ""
echo "=== checks.yaml validation ==="
CHECKS_YAML="$REPO_ROOT/examples/checks.yaml"
if [[ ! -f "$CHECKS_YAML" ]]; then
    fail "examples/checks.yaml" "file missing"
elif command -v python3 &>/dev/null && python3 -c "import yaml; yaml.safe_load(open('$CHECKS_YAML'))" 2>/dev/null; then
    pass "examples/checks.yaml (YAML parse)"
elif command -v ruby &>/dev/null && ruby -ryaml -e "YAML.safe_load(File.read('$CHECKS_YAML'))" 2>/dev/null; then
    pass "examples/checks.yaml (YAML parse via ruby)"
else
    skip "examples/checks.yaml" "no yaml validator available (python3/ruby); validate manually"
fi

# ---------- Summary ----------------------------------------------------------
echo ""
echo "=== Smoke summary ==="
echo "  PASS:  $PASS"
echo "  FAIL:  $FAIL"
echo "  XFAIL: $XFAIL (expected — pending wt/docs merge)"
echo ""

if (( FAIL > 0 )); then
    echo "RESULT: FAILED ($FAIL unexpected failure(s))"
    exit 1
else
    echo "RESULT: OK"
    exit 0
fi
