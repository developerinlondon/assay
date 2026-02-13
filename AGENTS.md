# AGENTS.md

## Proposal-First Rule (CRITICAL)

NEVER create or modify project files without explicit approval. Always:

1. PROPOSE the change (what, why, which files)
2. WAIT for approval
3. IMPLEMENT only after approval

Exceptions: bug fixes in already-approved work, read-only research, formatting.

## Team

- **Nayeem Syed** - Project owner
- **AI Agent** - Primary developer (Claude Opus, via OpenCode)

## Project

**Assay** - A lightweight, open-source deployment verification runner. Rust runtime + Lua 5.4
scripting. Runs as an ArgoCD PostSync hook Job to verify deployments actually work.

- **Repo**: [github.com/developerinlondon/assay](https://github.com/developerinlondon/assay)
- **Image**: `ghcr.io/developerinlondon/assay` (scratch, target ~6MB)
- **Landing**: assay.sbs (planned)
- **Parent project**: [jeebon](https://github.com/jarvisai-run/jeebon) (B2B2C AI knowledge base)

## Stack

| Layer     | Component                                                           |
| --------- | ------------------------------------------------------------------- |
| Language  | Rust (2024 edition)                                                 |
| Runtime   | Tokio (async)                                                       |
| Scripting | Lua 5.4 via mlua 0.11.6 (`lua54`, `vendored`, `async`, `serialize`) |
| HTTP      | reqwest 0.13.x (`json`, `rustls`, `query`)                          |
| CLI       | clap 4.x (derive)                                                   |
| Config    | serde_yml (YAML) + serde_json                                       |
| Logging   | tracing + tracing-subscriber (stderr, env-filter)                   |
| Errors    | anyhow                                                              |
| CI/CD     | GitHub Actions -> ghcr.io                                           |
| Container | Multi-stage Rust builder -> scratch                                 |

## Architecture

```
+------------------------------------------------------------------+
| assay binary (~5MB, statically linked, scratch image)            |
|                                                                  |
|  +------------------------------------------------------------+  |
|  | Rust Core (tokio async runtime)                            |  |
|  |                                                            |  |
|  |  +-- Config parser (serde_yml) -- reads checks.yaml        |  |
|  |  +-- CLI (clap) -- `assay checks.yaml` / `assay script.lua` |  |
|  |  +-- Runner -- orchestrates checks, retries, timeouts      |  |
|  |  +-- HTTP client (reqwest) -- async GET/POST               |  |
|  |  +-- JSON engine (serde_json) -- parse + encode            |  |
|  |  +-- Structured output -- JSON results to stdout           |  |
|  |  +-- Exit code -- 0 (all pass) or 1 (any fail)             |  |
|  +---------------------------+--------------------------------+  |
|                              | exposed as Lua builtins           |
|  +---------------------------v--------------------------------+  |
|  | Lua 5.4 VM (mlua, sandboxed -- os/io/debug/load removed)  |  |
|  |                                                            |  |
|  |  http.get/post, json.parse/encode, prometheus.query        |  |
|  |  assert.{eq,gt,lt,contains,not_nil,matches}                |  |
|  |  log.{info,warn,error}, env.get, sleep                     |  |
|  +------------------------------------------------------------+  |
+------------------------------------------------------------------+
```

## Directory Structure

```
assay/
+-- Cargo.toml
+-- Cargo.lock
+-- AGENTS.md              # This file
+-- Dockerfile             # Multi-stage: rust builder -> scratch
+-- src/
|   +-- main.rs            # CLI entry point (clap)
|   +-- config.rs          # YAML config parser (checks.yaml)
|   +-- runner.rs          # Orchestrator: retries, backoff, timeout
|   +-- output.rs          # Structured JSON results + exit code
|   +-- checks/
|   |   +-- mod.rs         # Check dispatcher
|   |   +-- http.rs        # HTTP check type (YAML mode)
|   |   +-- prometheus.rs  # Prometheus check type (YAML mode)
|   |   +-- script.rs      # Lua script check type
|   +-- lua/
|       +-- mod.rs         # Lua 5.4 VM setup + sandbox
|       +-- builtins.rs    # All builtin functions
|       +-- async_bridge.rs# Async Lua execution
+-- examples/              # Example check configs and Lua scripts
+-- tests/                 # Integration test configs
+-- .github/workflows/     # CI (release.yml)
+-- .opencode/
    +-- plans/             # Implementation plans
```

## Commands

```bash
cargo check                        # Type check
cargo clippy -- -D warnings        # Lint (warnings = errors)
cargo test                         # Run tests
cargo build --release              # Release build (~5MB)
cargo build --release --target x86_64-unknown-linux-musl  # Static binary for Docker
```

## Coding Practices

1. **Proposal-First**: Analyze -> propose -> get approval -> implement
2. **Warnings Are Errors**: `cargo clippy -- -D warnings` must pass. Fix ALL warnings.
3. **No Underscore Prefix**: Never use `_variable` to silence linters -- use or remove it.
4. **No Suppression**: No `#[allow(...)]`, `as any`, `@ts-ignore` equivalents.
5. **Real Error Messages**: Use `anyhow::Context` for all fallible operations. No empty catch.
6. **Test After Change**: Run `cargo check && cargo clippy -- -D warnings && cargo test` after
   edits.
7. **Latest Versions**: Check `cargo search <crate> --limit 1` before adding dependencies.
8. **Performance-Aware**: Every code change must consider performance impact. Avoid unnecessary
   allocations, cloning, and copies. Prefer zero-cost abstractions. When adding new builtins or
   modifying hot paths: reuse existing clients/connections (don't create per-request), use lazy
   initialization for expensive resources, extract shared logic into helpers instead of duplicating
   code. Profile before optimizing, but never introduce obviously wasteful patterns. The Rust core
   must stay lean -- assay runs as short-lived K8s Jobs where startup time and memory matter.

## Design Decisions (FINAL -- do not change)

| Decision         | Choice   | Reason                                                                   |
| ---------------- | -------- | ------------------------------------------------------------------------ |
| Language runtime | Lua 5.4  | ArgoCD compatible, 30yr ecosystem, native int64, perf irrelevant for I/O |
| Not Luau         | Rejected | Benchmark used JIT (unfair), Lua 5.1 base, Roblox ecosystem, no int64    |
| Not Rhai         | Rejected | 6x slower, no async, no coroutines                                       |
| Not Wasmtime     | Rejected | Fastest but requires compile step, bad for script iteration              |

## Autonomous Operation

### Deviation Rules

- Rule 1: AUTO-FIX bugs -- wrong logic, type errors, null pointers, broken tests
- Rule 2: AUTO-ADD missing safety -- error handling, input validation
- Rule 3: AUTO-FIX blockers -- missing deps, broken imports
- Rule 4: ASK about architecture -- new dependencies, changing APIs, new features

Priority: Rule 4 (stop and ask) > Rules 1-3 (fix silently and note what you did).
