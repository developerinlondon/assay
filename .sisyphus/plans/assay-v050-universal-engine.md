# Assay v0.5.0 — Universal API Execution Engine

## TL;DR

> **Quick Summary**: Transform assay from a K8s-only Lua runtime into a universal API execution
> engine with tiered search (SQLite FTS5 when `db` feature enabled, hand-rolled BM25 fallback),
> prompt-ready context injection for LLMs, and extensible filesystem module loading — eliminating
> the need for individual MCP servers.
>
> **Deliverables**:
>
> - Tiered search engine: SQLite FTS5 (default CLI) + zero-dep BM25 fallback (crate usage)
> - `SearchEngine` trait abstracting both backends
> - `assay context <query>` — prompt-ready module search for LLM integration
> - `assay exec -e '<script>'` — inline Lua execution
> - `assay modules` — list all available modules across all sources
> - LDoc metadata headers on all 23 stdlib modules with @quickref per function
> - Filesystem module loading from `~/.assay/modules/` and `./modules/`
> - SKILL.md for LLM agent onboarding
> - README rewrite for universal positioning
> - Full backward compatibility (`assay script.lua` + `assay checks.yaml`)
>
> **Estimated Effort**: Large **Parallel Execution**: YES — 5 waves (16 tasks + 4 final
> verification) **Critical Path**: T3 (parser) → T6 (discovery) → T10 (context cmd) → T13 (SKILL.md)

---

## Context

### Original Request

Redesign Assay from a Kubernetes-focused Lua runtime into a Universal API Execution Engine that
replaces the entire MCP ecosystem. Must scale to hundreds/thousands of Lua modules with BM25-powered
search, preemptive LLM context injection, and a community ecosystem.

### Interview Summary

**Key Decisions**:

- **Positioning**: "Both — tiered story" — Start as MCP's missing execution layer, progressively
  show users they don't need individual MCP servers
- **BM25 workflow**: Infrastructure runs search BEFORE LLM sees message, injects results into
  prompt. BM25 runs ONCE, not as a tool call
- **Module spec**: Rich LDoc-style annotations (@module, @description, @keywords, @env, @quickref)
- **Module paths**: Built-in (binary) + `~/.assay/modules/` (global) + `./modules/` (per-project)
- **Security**: Vetted modules via PRs to assay repo; custom modules on user filesystem
- **Scale**: Hundreds to thousands of modules (23 is just the start) **Crate compatibility**: Tiered
  search — SQLite FTS5 when `db` feature enabled (default CLI), hand-rolled BM25 fallback (zero
  deps) for crate users without SQLite
- **Test strategy**: TDD (tests first)
- **Version**: v0.5.0

**Research Findings**:

- MCP overhead: GitHub MCP = 55K tokens/93 tools; multi-server = 181K tokens (91% of context)
- 97.1% of MCP tool descriptions have quality "smells" (Queen's University study)
- Hand-rolled BM25: ~70 lines Rust, zero deps, 1000+ modules in microseconds
- SQLite FTS5: free when `db` feature enabled (upgrade path, not required)
- Oracle: static catalog for ≤23 modules, BM25 for 80+ — plan supports both

### Metis Review

**Identified Gaps (all addressed)**:

- Builtins must be searchable → included in module discovery (T6)
- Module collision priority → project > global > built-in (guardrail)
- Empty query → return full catalog
- BM25 field boosting → keywords 3×, name 2×, description 1×, functions 1× (T4 spec)
- Document frequency for IDF, NOT term count → explicit in T4
- Backward compat for `assay script.lua` → dedicated task (T9)
- Binary size ceiling → < 12MB guardrail
- Auto-extract function names → reduces annotation burden (T3)
- LDoc `--- @tag` convention → locks metadata format

### Technical Specifications

#### LDoc Metadata Format

Every stdlib module MUST have this header block (Lua triple-dash comments):

```lua
--- @module assay.grafana
--- @description Grafana monitoring and dashboards. Health, datasources, annotations, alerts, folders.
--- @keywords grafana, monitoring, dashboards, datasources, annotations, alerts, health
--- @env GRAFANA_URL, GRAFANA_API_KEY
--- @quickref c:health() -> {database, version, commit} | Check Grafana health
--- @quickref c:datasources() -> [{id, name, type, url}] | List all datasources
--- @quickref c:dashboard(uid) -> {dashboard, meta} | Get dashboard by UID
```

Rules:

- Lines MUST start with `--- @` (triple-dash, LDoc convention)
- @module: full require path (`assay.name`)
- @description: one line, max 120 chars, starts with service name
- @keywords: comma-separated, lowercase, include service name
- @env: comma-separated env var names (omit tag if none needed)
- @quickref: `signature -> return_hint | one-line description` (one per client method)
- Header block at TOP of file, before `local M = {}`

#### Tiered Search Architecture

Assay uses a **tiered search** approach:

- **Default (CLI binary with `db` feature)**: SQLite FTS5 via in-memory `:memory:` database. Real
  Okapi BM25 (k1=1.2, b=0.75), porter stemming, unicode61 tokenizer, column weighting. Zero
  additional binary cost — `libsqlite3-sys` already compiles with `-DSQLITE_ENABLE_FTS5`.
- **Fallback (crate usage without `db` feature)**: Hand-rolled BM25 (~70 lines Rust, zero deps).
  Same k1/b parameters, same field boosting, no stemming.
- **Both implement `SearchEngine` trait** — callers are backend-agnostic.
- **Feature gating**: `#[cfg(feature = "db")]` selects FTS5; `#[cfg(not(feature = "db"))]` falls
  back to BM25.

#### SearchEngine Trait

```rust
pub trait SearchEngine {
    fn add_document(&mut self, id: &str, fields: &[(&str, &str, f64)]);
    fn search(&self, query: &str, limit: usize) -> Vec<SearchResult>;
}

pub struct SearchResult {
    pub id: String,
    pub score: f64,
}
```

#### BM25 Algorithm Parameters (both backends)

- k1 = 1.2, b = 0.75 (standard defaults)
- Field weights: keywords=3.0, module_name=2.0, description=1.0, functions=1.0
- Tokenizer: split on `[^a-zA-Z0-9_]`, lowercase, filter tokens with len ≤ 1
- IDF formula: `ln((N - df + 0.5) / (df + 0.5) + 1)` (N=total docs, df=docs containing term)

#### FTS5 Backend Details (when `db` feature enabled)

```sql
CREATE VIRTUAL TABLE modules USING fts5(
    name,           -- weight 2.0
    description,    -- weight 1.0
    keywords,       -- weight 3.0
    functions,      -- weight 1.0
    tokenize="unicode61"
);
SELECT name, bm25(modules, 2.0, 1.0, 3.0, 1.0) AS rank
FROM modules WHERE modules MATCH ? ORDER BY rank;
```

#### Module Resolution Priority

1. Per-project: `./modules/<name>.lua` (highest)
2. Global user: `~/.assay/modules/<name>.lua` (env override: `ASSAY_MODULES_PATH`)
3. Built-in: embedded via `include_dir!` (lowest)

Search indexes ALL sources. Loading uses first-found priority.

#### `assay context` Output Format

```
# Assay Module Context

## Matching Modules

### assay.grafana
Grafana monitoring and dashboards. Health, datasources, annotations, alerts, folders.
Auth: { api_key = "..." } or { username = "...", password = "..." }
Env: GRAFANA_URL, GRAFANA_API_KEY
Methods:
  c:health() -> {database, version, commit} | Check Grafana health
  c:datasources() -> [{id, name, type, url}] | List all datasources
  ...

## Built-in Functions (always available, no require needed)
http.get(url, opts?) -> {status, body, headers}
json.parse(str) -> table | json.encode(tbl) -> str
env.get(key) -> str | log.info(msg) | assert.eq(a, b, msg?)

## Usage
local grafana = require("assay.grafana")
local c = grafana.client(env.get("GRAFANA_URL"), { api_key = env.get("GRAFANA_API_KEY") })
```

---

## Work Objectives

### Core Objective

Make assay a universal API execution engine with intelligent module discovery, enabling LLM agents
to find and use the right API modules without MCP overhead — via a single `assay context` command
that returns prompt-ready module context.

### Concrete Deliverables

`src/metadata.rs` — Module metadata parser (LDoc + auto-extraction) `src/search.rs` — SearchEngine
trait + zero-dep BM25 search engine with field boosting `src/search_fts5.rs` — SQLite FTS5 search
backend (gated behind `db` feature) `src/discovery.rs` — Module discovery across all sources +
search index builder `src/context.rs` — Context output formatter (prompt-ready text) `src/main.rs` —
Restructured CLI with subcommands (backward compat) `src/lua/mod.rs` — Extended module loader for
filesystem paths `stdlib/*.lua` — All 23 modules with LDoc metadata headers `SKILL.md` — LLM agent
skill file `README.md` — Rewritten for universal positioning

### Definition of Done

- [ ] `assay context "grafana health"` → returns grafana module info with quickrefs
- [ ] `assay exec -e 'log.info("hello")'` → prints "hello"
- [ ] `assay modules` → lists all 23+ modules with descriptions
- [ ] `assay script.lua` → works identically to v0.4.x
- [ ] `assay checks.yaml` → works identically to v0.4.x
- [ ] `cargo test` → all new + existing tests pass
- [ ] `cargo clippy -- -D warnings` → zero warnings
- [ ] Binary size < 12MB (current ~9MB)
- [ ] `assay --version` → reports 0.5.0

### Must Have

Tiered search: FTS5 (when `db` feature on, using existing sqlx) + zero-dep BM25 fallback

- BM25 field boosting (keywords 3× > name 2× > description 1× > functions 1×)
- Auto-extraction of function names from Lua source
- Backward compatibility for `assay <file>.lua` and `assay <file>.yaml`
- TDD: tests written before implementation for all new Rust modules
- Builtins (http, json, crypto, etc.) included in search index
- Modules from `~/.assay/modules/` and `./modules/` searchable and loadable

### Must NOT Have (Guardrails)

- NO new crate dependencies for search (BM25 = zero-dep; FTS5 uses existing sqlx)
- NO breaking changes to existing CLI behavior
- NO OpenAPI codegen (`assay generate` is Phase 2)
- NO MCP server implementation (Phase 2)
- NO module registry or community workflow (Phase 2)
- NO embedding/vector search (not planned)
- NO module versioning (not planned)
- NO runtime network calls during search/context
- NO changes to existing stdlib module function signatures (only ADD headers)
- NO binary growth beyond 12MB
- NO `unsafe` blocks without explicit justification in comments

---

## Verification Strategy

> **ZERO HUMAN INTERVENTION** — ALL verification is agent-executed. No exceptions.

### Test Decision

- **Infrastructure exists**: YES (`cargo test`, wiremock pattern)
- **Automated tests**: TDD (tests first)
- **Framework**: `cargo test` + wiremock for HTTP mocking
- **Each task**: RED (write failing test) → GREEN (minimal impl) → REFACTOR

### QA Policy

Every task MUST include agent-executed QA scenarios. Evidence saved to
`.sisyphus/evidence/task-{N}-{scenario-slug}.{ext}`.

- **CLI commands**: Use Bash — run command, assert exit code + stdout/stderr content
- **Rust modules**: Use Bash (`cargo test --test <name>`) — run specific tests, assert pass
- **Lua modules**: Use Bash (`cargo run -- exec -e '...'`) — execute test script, verify output
- **Search quality**: Use Bash — run `assay context` with test queries, verify ranking

---

## Execution Strategy

### Parallel Execution Waves

```
Wave 1 (Start Immediately — foundation, 5 parallel):
├── T1:  LDoc metadata headers on all 23 stdlib modules [quick]
├── T2:  CLI restructure to clap subcommands [unspecified-high]
├── T3:  Metadata parser module (src/metadata.rs) [deep]
├── T4:  BM25 search engine (src/search.rs) [deep]
└── T5:  Filesystem module loader paths [quick]

Wave 2 (After Wave 1 — assembly, 5 parallel):
├── T4b: FTS5 search backend (src/search_fts5.rs) [quick] (dep: 4)
├── T6:  Module discovery + search index builder [unspecified-high] (dep: 1,3,4,4b,5)
├── T7:  `assay exec` subcommand handler [quick] (dep: 2)
├── T8:  Context output formatter [quick] (dep: 3)
└── T9:  Backward-compat default command [quick] (dep: 2)

Wave 3 (After Wave 2 — integration, 3 parallel):
├── T10: `assay context <query>` subcommand [deep] (dep: 2,6,8)
├── T11: `assay modules` subcommand [quick] (dep: 2,6)
└── T12: Integration + e2e tests [deep] (dep: 7,9,10,11)

Wave 4 (After Wave 3 — polish, 3 parallel):
├── T13: SKILL.md for LLM agents [writing] (dep: 10)
├── T14: README rewrite for universal positioning [writing] (dep: 10)
└── T15: Version bump v0.5.0 + final verification [quick] (dep: all)

Wave FINAL (After ALL — independent review, 4 parallel):
├── F1: Plan compliance audit [oracle]
├── F2: Code quality review [unspecified-high]
├── F3: Real manual QA [unspecified-high]
└── F4: Scope fidelity check [deep]

Critical Path: T3 → T6 → T10 → T12 → T15
Parallel Speedup: ~65% faster than sequential
Max Concurrent: 5 (Waves 1 & 2)
```

### Dependency Matrix

| Task | Blocked By          | Blocks           | Wave |
| ---- | ------------------- | ---------------- | ---- |
| T1   | —                   | T6               | 1    |
| T2   | —                   | T7, T9, T10, T11 | 1    |
| T3   | —                   | T6, T8           | 1    |
| T4   | —                   | T4b, T6          | 1    |
| T5   | —                   | T6               | 1    |
| T4b  | T4                  | T6               | 2    |
| T6   | T1, T3, T4, T4b, T5 | T10, T11, T12    | 2    |
| T7   | T2                  | T12              | 2    |
| T8   | T3                  | T10              | 2    |
| T9   | T2                  | T12              | 2    |
| T10  | T2, T6, T8          | T12, T13, T14    | 3    |
| T11  | T2, T6              | T12              | 3    |
| T12  | T7, T9, T10, T11    | T15              | 3    |
| T13  | T10                 | T15              | 4    |
| T14  | T10                 | T15              | 4    |
| T15  | all                 | F1-F4            | 4    |

### Agent Dispatch Summary

| Wave  | Count | Dispatch                                                     |
| ----- | ----- | ------------------------------------------------------------ |
| 1     | 5     | T1→quick, T2→unspecified-high, T3→deep, T4→deep, T5→quick    |
| 2     | 5     | T4b→quick, T6→unspecified-high, T7→quick, T8→quick, T9→quick |
| 3     | 3     | T10→deep, T11→quick, T12→deep                                |
| 4     | 3     | T13→writing, T14→writing, T15→quick                          |
| FINAL | 4     | F1→oracle, F2→unspecified-high, F3→unspecified-high, F4→deep |

---

## TODOs

> Implementation + Test = ONE Task. Never separate. TDD: RED (write failing test) → GREEN (minimal
> impl) → REFACTOR. EVERY task MUST have: Agent Profile + Parallelization + QA Scenarios. **A task
> WITHOUT QA Scenarios is INCOMPLETE. No exceptions.**

- 1. [ ] Add LDoc Metadata Headers to All 23 Stdlib Modules

  **What to do**:
  - Add LDoc-style metadata comment headers to all 23 stdlib modules in `stdlib/`
  - Each module gets: `@module`, `@description`, `@keywords`, `@env`, `@quickref` (per client
    method)
  - Read each module's source to understand its API surface (client methods, auth patterns, env
    vars)
  - Write accurate `@quickref` lines for every `function c:method()` in each module
  - Header block goes at TOP of file, before `local M = {}`
  - Follow the format specified in Technical Specifications section above
  - Existing functional code must NOT be modified — only add comment headers

  **Must NOT do**:
  - Do NOT modify any functional Lua code (only add comments)
  - Do NOT change function signatures, error messages, or behavior
  - Do NOT add `@quickref` for local helper functions (`api_get`, `api_post`, etc.) — only client
    methods
  - Do NOT invent env var names — only document vars the module actually uses (check for `env.get()`
    calls)

  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: Repetitive editing across 23 files, same pattern each time
  - **Skills**: `[]`

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with T2, T3, T4, T5)
  - **Blocks**: T6 (module discovery needs metadata to index)
  - **Blocked By**: None (can start immediately)

  **References**:

  **Pattern References**:
  - `stdlib/grafana.lua` — Reference module (110 lines, 10 client methods, 2 auth patterns)
  - `stdlib/vault.lua` — Complex reference (330 lines, many client methods, token auth)
  - `stdlib/prometheus.lua` — Another reference with different API patterns

  **API/Type References**:
  - All 23 files in `stdlib/` directory — each needs headers added
  - `AGENTS.md` stdlib table — Use as starting point for `@description` and `@keywords`

  **WHY Each Reference Matters**:
  - `grafana.lua` shows the typical module structure: local helpers + client methods
  - `AGENTS.md` stdlib table provides descriptions to use as `@description` values
  - Each module's existing code reveals: auth patterns (→ `@env`), method names (→ `@quickref`)

  **Acceptance Criteria**:

  **TDD**: N/A (Lua comment editing, not Rust code)

  **QA Scenarios (MANDATORY)**:

  ```
  Scenario: All 23 modules have metadata headers
    Tool: Bash
    Preconditions: stdlib/ directory has 23 .lua files
    Steps:
      1. Run: for f in stdlib/*.lua; do head -1 "$f"; done
      2. Assert: every file's first line starts with "--- @module assay."
      3. Run: grep -c "@module" stdlib/*.lua
      4. Assert: each file has exactly 1 @module line
      5. Run: grep -c "@description" stdlib/*.lua
      6. Assert: each file has exactly 1 @description line
    Expected Result: All 23 modules have @module, @description, @keywords headers
    Failure Indicators: Any module missing a header line, or header after local M = {}
    Evidence: .sisyphus/evidence/task-1-metadata-headers.txt

  Scenario: Modules still load and work after header addition
    Tool: Bash
    Preconditions: cargo build succeeds
    Steps:
      1. Run: cargo test
      2. Assert: all existing stdlib tests pass (no regressions)
      3. Run: cargo run -- exec -e 'local g = require("assay.grafana"); assert.not_nil(g.client)'
      4. Assert: exit code 0 (module loads correctly with headers)
    Expected Result: Headers are pure comments and don't affect execution
    Failure Indicators: Any test failure or module load error
    Evidence: .sisyphus/evidence/task-1-modules-load.txt

  Scenario: @quickref lines match actual function signatures
    Tool: Bash
    Preconditions: Headers added to all modules
    Steps:
      1. For grafana.lua: count `function c:` lines vs @quickref lines
      2. Assert: counts match (every client method has a @quickref)
      3. Spot-check: verify c:health(), c:datasources(), c:dashboard(uid) are in @quickref
    Expected Result: 1:1 mapping between client methods and @quickref lines
    Failure Indicators: Missing or extra @quickref lines
    Evidence: .sisyphus/evidence/task-1-quickref-accuracy.txt
  ```

  **Commit**: YES
  - Message: `feat(stdlib): add LDoc metadata headers to all 23 modules`
  - Files: `stdlib/*.lua`
  - Pre-commit: `cargo test`

- 2. [ ] CLI Restructure to Clap Subcommands

  **What to do**:
  - Restructure `src/main.rs` from single positional file arg to clap subcommand architecture
  - Define subcommands: `context`, `exec`, `modules`, `run`
  - Implement ONLY the CLI argument parsing — command handlers are stubs that print "not yet
    implemented"
  - Keep backward-compat logic: when no subcommand given, fall back to file extension detection
  - The existing `run_yaml_checks()` and `run_lua_script()` functions stay as-is
  - CLI structure:
    ```
    assay context <query> [--limit N]   # Module search (stub)
    assay exec -e '<script>'            # Inline Lua (stub)
    assay exec <file.lua>               # File Lua (stub)
    assay modules                       # List modules (stub)
    assay run <file>                    # Explicit run (delegates to existing logic)
    assay <file>                        # Backward compat (auto-detect by extension)
    assay --version                     # Version info
    ```

  **Must NOT do**:
  - Do NOT implement actual command logic (only stubs) — other tasks handle implementations
  - Do NOT remove or modify `run_yaml_checks()` or `run_lua_script()` functions
  - Do NOT break `assay script.lua` or `assay checks.yaml` behavior
  - Do NOT add new crate dependencies beyond what's already in Cargo.toml

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
    - Reason: Requires careful clap design for backward compat + multiple subcommands
  - **Skills**: `[]`

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with T1, T3, T4, T5)
  - **Blocks**: T7 (exec handler), T9 (backward compat), T10 (context cmd), T11 (modules cmd)
  - **Blocked By**: None (can start immediately)

  **References**:

  **Pattern References**:
  - `src/main.rs:26-35` — Current Cli struct with single positional `file` arg
  - `src/main.rs:37-65` — Current main() with extension-based dispatch

  **API/Type References**:
  - `Cargo.toml:43` — clap dependency (v4.5.57, derive feature)

  **External References**:
  - clap derive docs: https://docs.rs/clap/latest/clap/_derive/index.html

  **WHY Each Reference Matters**:
  - `main.rs:26-35` — The current Cli struct must be replaced with subcommand-aware version
  - `main.rs:37-65` — The dispatch logic must be preserved as fallback for backward compat

  **Acceptance Criteria**:

  **TDD**:
  - [ ] Test: `assay --version` returns version string
  - [ ] Test: `assay --help` shows subcommands (context, exec, modules, run)
  - [ ] Test: `assay tests/e2e/check_json.lua` still works (backward compat)

  **QA Scenarios (MANDATORY)**:

  ```
  Scenario: Subcommands registered and visible in help
    Tool: Bash
    Preconditions: cargo build succeeds
    Steps:
      1. Run: cargo run -- --help 2>&1
      2. Assert: output contains "context", "exec", "modules"
      3. Run: cargo run -- context --help 2>&1
      4. Assert: output contains "QUERY" and "--limit"
    Expected Result: All subcommands visible in help output
    Failure Indicators: Missing subcommand in help, or clap parse error
    Evidence: .sisyphus/evidence/task-2-cli-help.txt

  Scenario: Backward compatibility — file extension detection works
    Tool: Bash
    Preconditions: cargo build succeeds
    Steps:
      1. Run: cargo run -- tests/e2e/check_json.lua
      2. Assert: exit code 0 (Lua script runs successfully)
      3. Run: cargo run -- run tests/e2e/check_json.lua
      4. Assert: exit code 0 (explicit run subcommand also works)
    Expected Result: Both `assay <file>` and `assay run <file>` work
    Failure Indicators: "unknown subcommand" error or exit code != 0
    Evidence: .sisyphus/evidence/task-2-backward-compat.txt
  ```

  **Commit**: YES
  - Message: `refactor(cli): restructure to clap subcommands with backward compat`
  - Files: `src/main.rs`
  - Pre-commit: `cargo check && cargo test`

- 3. [ ] Metadata Parser Module (`src/metadata.rs`)

  **What to do**:
  - Create `src/metadata.rs` with a metadata parser for LDoc-style Lua annotations
  - Define `ModuleMetadata` struct: module_name, description, keywords (Vec<String>), env_vars
    (Vec<String>), quickrefs (Vec<QuickRef>), auto_functions (Vec<String>)
  - Define `QuickRef` struct: signature, return_hint, description
  - Implement `parse_metadata(source: &str) -> ModuleMetadata`: parse `--- @tag` lines +
    auto-extract `function c:method()` and `function M.fn()` patterns
  - Graceful degradation: if no headers, return empty metadata with only auto-extracted functions
  - TDD: write tests FIRST in `tests/metadata.rs`, then implement
  - Add `pub mod metadata;` to `src/lib.rs`

  **Must NOT do**:
  - Do NOT parse beyond the header comment block (stop at first non-`---` line)
  - Do NOT require all fields — graceful degradation for missing
  - Do NOT add external dependencies — use std + regex-lite (already in deps)

  **Recommended Agent Profile**:
  - **Category**: `deep` — Core infrastructure with parsing logic + TDD
  - **Skills**: `[]`

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with T1, T2, T4, T5)
  - **Blocks**: T6, T8
  - **Blocked By**: None

  **References**:
  - `stdlib/grafana.lua` — Reference module showing `function c:method()` patterns
  - `stdlib/vault.lua` — Complex module (330 lines), stress test for parser
  - `stdlib/k8s.lua` — May have `M.function` patterns (not just `c:method`)
  - Context section "LDoc Metadata Format" — Authoritative parsing contract
  - `Cargo.toml:71` — `regex-lite` already available

  **Acceptance Criteria**:
  - [ ] `cargo test --test metadata` → PASS (tests written first)

  **QA Scenarios (MANDATORY)**:
  ```
  Scenario: Parse complete metadata from annotated module
    Tool: Bash
    Steps:
      1. parse_metadata() with grafana-style input → module=="assay.grafana", keywords contain "grafana","monitoring"
      2. env_vars contain "GRAFANA_URL", quickrefs.len() >= 8
      3. cargo test --test metadata → all pass
    Evidence: .sisyphus/evidence/task-3-parse-complete.txt

  Scenario: Auto-extract functions without headers
    Tool: Bash
    Steps:
      1. Input without headers but with `function c:health()` and `function M.client(url)`
      2. auto_functions contains "health", "client"; other fields empty/default
    Evidence: .sisyphus/evidence/task-3-auto-extract.txt

  Scenario: Graceful degradation
    Tool: Bash
    Steps:
      1. Empty string → default ModuleMetadata, no panic
      2. Partial headers → returns available fields
    Evidence: .sisyphus/evidence/task-3-graceful-degradation.txt
  ```

  **Commit**: YES
  - Message: `feat(metadata): add module metadata parser with function extraction`
  - Files: `src/metadata.rs`, `src/lib.rs`, `tests/metadata.rs`
  - Pre-commit: `cargo test --test metadata`

- 4. [ ] BM25 Search Engine (`src/search.rs`)

  **What to do**:
  - Create `src/search.rs` with `SearchEngine` trait + zero-dep BM25 implementation
  - Define `SearchEngine` trait with `add_document(&mut self, id, fields: &[(&str, &str, f64)])` and
    `search(&self, query, limit) -> Vec<SearchResult>`. Both backends implement this trait.
  - Define `SearchResult`: id (String), score (f64)
  - `BM25Index` implements `SearchEngine`. Methods: `new()`, `add_document()`, `search()`
  - Tokenizer: split `[^a-zA-Z0-9_]`, lowercase, filter len ≤ 1
  - BM25: k1=1.2, b=0.75. IDF: `ln((N - df + 0.5) / (df + 0.5) + 1)`
  - Field boosting: multiply BM25 contribution by field weight
  - **CRITICAL**: Use DOCUMENT frequency (docs containing term), NOT term occurrence count
  - TDD: tests FIRST in `tests/search.rs`. Add `pub mod search;` to `src/lib.rs`
  - Trait and BM25Index must be `pub` — FTS5 backend (T4b) will implement same trait

  **Must NOT do**:
  - Do NOT add crate dependencies (zero-dep)
  - Do NOT use `unsafe`, persistence, fuzzy matching, or stemming

  **Recommended Agent Profile**:
  - **Category**: `deep` — Algorithm with correctness-critical IDF + TDD
  - **Skills**: `[]`

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with T1, T2, T3, T5)
  - **Blocks**: T6
  - **Blocked By**: None

  **References**:
  - BM25 Wikipedia: https://en.wikipedia.org/wiki/Okapi_BM25
  - Context section "BM25 Algorithm Parameters" — exact spec

  **Acceptance Criteria**:
  - [ ] `cargo test --test search` → PASS (tests written first)

  **QA Scenarios (MANDATORY)**:
  ```
  Scenario: Basic ranking
    Tool: Bash
    Steps:
      1. Index grafana/prometheus/loki with distinct keywords
      2. Search each keyword → correct module ranks first
    Evidence: .sisyphus/evidence/task-4-basic-ranking.txt

  Scenario: Field boosting
    Tool: Bash
    Steps:
      1. Doc A: keywords="monitoring"(3.0), desc="grafana"(1.0)
      2. Doc B: keywords="grafana"(3.0), desc="monitoring"(1.0)
      3. Search "monitoring" → A first; Search "grafana" → B first
    Evidence: .sisyphus/evidence/task-4-field-boosting.txt

  Scenario: IDF correctness (document frequency, not term count)
    Tool: Bash
    Steps:
      1. Doc with "grafana" 3x, another with "grafana" 1x, third with "prometheus"
      2. IDF based on df=2, no negative scores
    Evidence: .sisyphus/evidence/task-4-idf-correctness.txt

  Scenario: Edge cases
    Tool: Bash
    Steps: empty query, empty index, no matches, empty doc text → no panics
    Evidence: .sisyphus/evidence/task-4-edge-cases.txt
  ```

  **Commit**: YES
  - Message: `feat(search): add zero-dep BM25 search engine with field boosting`
  - Files: `src/search.rs`, `src/lib.rs`, `tests/search.rs`
  - Pre-commit: `cargo test --test search`

- 4b. [ ] FTS5 Search Backend (`src/search_fts5.rs`)

  **What to do**:
  - Create `src/search_fts5.rs` gated behind `#[cfg(feature = "db")]`
  - Implement `FTS5Index` struct wrapping an in-memory SQLite database with FTS5 virtual table
  - `FTS5Index` implements the `SearchEngine` trait defined in T4 (`src/search.rs`)
  - Use `sqlx::SqlitePool` (already in deps via `db` feature) for SQLite access
  - On `new()`: create `:memory:` SQLite database, execute:
    ```sql
    CREATE VIRTUAL TABLE modules USING fts5(
        name,           -- weight 2.0
        description,    -- weight 1.0
        keywords,       -- weight 3.0
        functions,      -- weight 1.0
        tokenize="unicode61"
    );
    ```
  - `add_document()`: INSERT INTO modules VALUES(name, description, keywords, functions)
  - `search()`: SELECT with `bm25(modules, 2.0, 1.0, 3.0, 1.0)` ranking, MATCH query
  - Handle FTS5 query syntax: escape special characters, handle empty query
  - Add `#[cfg(feature = "db")] pub mod search_fts5;` to `src/lib.rs`
  - TDD: write tests FIRST in `tests/search_fts5.rs`, gated behind `#[cfg(feature = "db")]`
  - ~30-50 lines of implementation (thin wrapper around SQLite FTS5)

  **Must NOT do**:
  - Do NOT duplicate BM25 logic — FTS5 does its own BM25 internally
  - Do NOT persist the SQLite database to disk (in-memory only)
  - Do NOT add new crate dependencies — sqlx with sqlite feature already in Cargo.toml
  - Do NOT modify `src/search.rs` — only import the trait from it

  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: Thin wrapper around existing SQLite+FTS5, mostly SQL + trait impl wiring
  - **Skills**: `[]`

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with T6, T7, T8, T9)
  - **Blocks**: T6 (discovery needs both backends available)
  - **Blocked By**: T4 (defines SearchEngine trait)

  **References**:

  **Pattern References**:
  - `src/search.rs` (T4) — `SearchEngine` trait definition and `SearchResult` struct to implement
  - `src/lua/builtins/db.rs` — existing sqlx/SQLite usage patterns in the codebase

  **API/Type References**:
  - `Cargo.toml:50-54` — sqlx dependency with sqlite feature (already present)
  - `src/search.rs` (T4) — `SearchEngine` trait: `add_document()`, `search()` signatures

  **External References**:
  - SQLite FTS5 docs: https://www.sqlite.org/fts5.html
  - sqlx SQLite: https://docs.rs/sqlx/latest/sqlx/sqlite/index.html

  **WHY Each Reference Matters**:
  - `src/search.rs` trait is the contract this module must satisfy — exact method signatures
  - `db.rs` shows how sqlx is used in this codebase (connection patterns, async handling)
  - FTS5 docs define MATCH syntax and bm25() function parameters

  **Acceptance Criteria**:

  **TDD**:
  - [ ] Test file: `tests/search_fts5.rs` (gated `#[cfg(feature = "db")]`)
  - [ ] `cargo test --test search_fts5` → PASS (tests written first)

  **QA Scenarios (MANDATORY)**:

  ```
  Scenario: FTS5 basic search ranking
    Tool: Bash
    Preconditions: db feature enabled (default)
    Steps:
      1. Create FTS5Index, add grafana/prometheus/loki documents
      2. Search "grafana" → grafana ranks first
      3. Search "monitoring" → relevant modules returned
      4. cargo test --test search_fts5 → all pass
    Expected Result: FTS5 search returns correctly ranked results
    Failure Indicators: Wrong ranking, SQL errors, empty results
    Evidence: .sisyphus/evidence/task-4b-fts5-basic-ranking.txt

  Scenario: FTS5 field boosting via bm25() weights
    Tool: Bash
    Steps:
      1. Doc A: keywords="monitoring", desc="grafana"
      2. Doc B: keywords="grafana", desc="monitoring"
      3. Search "monitoring" → A ranks higher (keywords weight 3.0 > desc 1.0)
    Expected Result: Column weights affect ranking correctly
    Evidence: .sisyphus/evidence/task-4b-fts5-field-boosting.txt

  Scenario: SearchEngine trait compatibility
    Tool: Bash
    Steps:
      1. Use FTS5Index via `&dyn SearchEngine` reference
      2. Same test inputs as T4 BM25 tests → both return same top result
    Expected Result: FTS5Index and BM25Index are interchangeable via trait
    Evidence: .sisyphus/evidence/task-4b-fts5-trait-compat.txt

  Scenario: Edge cases
    Tool: Bash
    Steps:
      1. Empty query → returns empty vec (no panic)
      2. No documents in index → returns empty vec
      3. Special characters in query → handled gracefully (no SQL injection)
    Expected Result: No panics, no SQL errors on edge cases
    Evidence: .sisyphus/evidence/task-4b-fts5-edge-cases.txt
  ```

  **Commit**: YES
  - Message: `feat(search): add FTS5 search backend for db feature`
  - Files: `src/search_fts5.rs`, `src/lib.rs`, `tests/search_fts5.rs`
  - Pre-commit: `cargo test --test search_fts5`

- 5. [ ] Filesystem Module Loader Paths

  **What to do**:
  - Extend `src/lua/mod.rs` to load from `~/.assay/modules/` and `./modules/` in addition to
    existing paths
  - Add two new Lua package.searchers (project modules, global modules)
  - Resolution priority: project > global > embedded > existing fs_searcher
  - Handle missing dirs gracefully (no error if dir doesn't exist)
  - `ASSAY_MODULES_PATH` env var overrides `~/.assay/modules/`
  - TDD: write tests FIRST in `tests/lua_modules.rs`, then implement

  **Must NOT do**:
  - Do NOT change existing register_stdlib_loader or register_fs_loader
  - Do NOT require dirs to exist
  - Do NOT recurse subdirectories
  - Do NOT break ASSAY_LIB_PATH

  **Recommended Agent Profile**:
  - **Category**: `quick` — Small extension to existing module loading pattern
  - **Skills**: `[]`

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with T1, T2, T3, T4)
  - **Blocks**: T6
  - **Blocked By**: None

  **References**:
  - `src/lua/mod.rs:50-83` — stdlib loader pattern to follow
  - `src/lua/mod.rs:86-119` — fs loader pattern to follow
  - `src/lua/mod.rs:14` — LIB_PATH_ENV constant pattern

  **Acceptance Criteria**:
  - [ ] `cargo test --test lua_modules` → PASS

  **QA Scenarios (MANDATORY)**:
  ```
  Scenario: Load module from ./modules/
    Tool: Bash
    Steps:
      1. Create ./modules/testmod.lua with simple function
      2. Run: cargo run -- exec -e 'local m = require("testmod"); m.test()'
      3. Assert: exit code 0
    Evidence: .sisyphus/evidence/task-5-project-modules.txt

  Scenario: ASSAY_MODULES_PATH override
    Tool: Bash
    Steps:
      1. Create /tmp/assay_test/mymod.lua
      2. Run: ASSAY_MODULES_PATH=/tmp/assay_test cargo run -- exec -e 'require("mymod")'
      3. Assert: exit code 0
    Evidence: .sisyphus/evidence/task-5-env-override.txt

  Scenario: Missing dir doesn't error
    Tool: Bash
    Steps:
      1. Run: cargo run -- exec -e 'log.info("ok")'
      2. Assert: exit code 0 (no error about missing ./modules/)
    Evidence: .sisyphus/evidence/task-5-missing-dir.txt
  ```

  **Commit**: YES
  - Message: `feat(modules): extend loader for ~/.assay/modules/ and ./modules/`
  - Files: `src/lua/mod.rs`
  - Pre-commit: `cargo test`

- 6. [ ] Module Discovery + BM25 Index Builder (`src/discovery.rs`)

  **What to do**:
  - Create `src/discovery.rs` with module discovery and search indexing
  - Implement `discover_modules() -> Vec<DiscoveredModule>` scanning embedded, ./modules/,
    ~/.assay/modules/
  - Parse metadata from each via `metadata::parse_metadata()`
  - Track source origin (BuiltIn/Project/Global)
  - Implement `build_index(modules) -> Box<dyn SearchEngine>` using cfg-gated backend selection:
    ```rust
    #[cfg(feature = "db")]
    { FTS5Index::new() }
    #[cfg(not(feature = "db"))]
    { BM25Index::new() }
    ```
  - Field weights: keywords 3.0, module_name 2.0, description 1.0, functions 1.0
  - Add hardcoded builtin entries for: http, json, yaml, toml, fs, crypto, base64, regex, db, ws,
    template, async, assert, log, env, sleep, time
  - Implement `search_modules(query, limit)` convenience function
  - Add `pub mod discovery;` to lib.rs
  - TDD: write tests FIRST in `tests/discovery.rs`, then implement

  **Must NOT do**:
  - Do NOT cache index to disk
  - Do NOT make network calls
  - Do NOT panic on unreadable files (skip with warning)
  - Do NOT deduplicate across sources (all sources indexed)

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high` — Integrates metadata + BM25 + fs scanning + builtins catalog
  - **Skills**: `[]`

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (after Wave 1)
  - **Blocks**: T10, T11, T12
  - **Blocked By**: T1, T3, T4, T4b, T5

  **References**:
  - `src/lua/mod.rs:8` — STDLIB_DIR include_dir!
  - `src/lua/mod.rs:63-67` — iterating embedded files pattern
  - `src/metadata.rs` (T3) — ModuleMetadata types
  - `src/search.rs` (T4) — SearchEngine trait + BM25Index
  - `src/search_fts5.rs` (T4b) — FTS5Index (cfg-gated, used when `db` feature enabled)
  - `AGENTS.md` Built-in Globals table — builtins list

  **Acceptance Criteria**:
  - [ ] `cargo test --test discovery` → PASS

  **QA Scenarios (MANDATORY)**:
  ```
  Scenario: Discover all 23 embedded modules with metadata
    Tool: Bash
    Steps:
      1. Run: cargo run -- exec -e 'local d = require("assay.discovery"); local mods = d.discover_modules(); assert.gt(#mods, 20)'
      2. Assert: exit code 0
    Evidence: .sisyphus/evidence/task-6-discover-embedded.txt

  Scenario: Search quality — grafana query returns grafana first
    Tool: Bash
    Steps:
      1. Run: cargo run -- context "grafana health"
      2. Assert: first result is assay.grafana
    Evidence: .sisyphus/evidence/task-6-search-quality.txt

  Scenario: Builtins appear in search results
    Tool: Bash
    Steps:
      1. Run: cargo run -- context "http request"
      2. Assert: output contains "http.get" or "http.post"
    Evidence: .sisyphus/evidence/task-6-builtins-search.txt
  ```

  **Commit**: YES
  - Message: `feat(discovery): add module discovery and BM25 index builder`
  - Files: `src/discovery.rs`, `src/lib.rs`, `tests/discovery.rs`
  - Pre-commit: `cargo test`

- 7. [ ] `assay exec` Subcommand Handler

  **What to do**:
  - Implement exec subcommand in src/main.rs
  - Two modes: `assay exec -e 'log.info("hello")'` (inline), `assay exec script.lua` (file,
    delegates to run_lua_script)
  - For inline: create VM via lua::create_vm(), execute expression using same LocalSet pattern
  - Error handling via format_lua_error()
  - All builtins available

  **Must NOT do**:
  - Do NOT create new VM implementation
  - Do NOT change run_lua_script()
  - Do NOT add different module loading

  **Recommended Agent Profile**:
  - **Category**: `quick` — Thin handler wiring existing VM to new CLI entry
  - **Skills**: `[]`

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (after Wave 1)
  - **Blocks**: T12
  - **Blocked By**: T2

  **References**:
  - `src/main.rs:89-129` — run_lua_script pattern to follow
  - `src/main.rs:100-108` — VM creation pattern
  - `src/main.rs:112-120` — LocalSet pattern

  **Acceptance Criteria**:
  - [ ] `cargo test` → all pass

  **QA Scenarios (MANDATORY)**:
  ```
  Scenario: Inline execution with log output
    Tool: Bash
    Steps:
      1. Run: cargo run -- exec -e 'log.info("exec-works")'
      2. Assert: exit code 0, output contains "exec-works"
    Evidence: .sisyphus/evidence/task-7-inline-exec.txt

  Scenario: Error handling
    Tool: Bash
    Steps:
      1. Run: cargo run -- exec -e 'error("boom")'
      2. Assert: exit code non-zero, output contains "boom"
    Evidence: .sisyphus/evidence/task-7-error-handling.txt

  Scenario: File execution
    Tool: Bash
    Steps:
      1. Run: cargo run -- exec tests/e2e/check_json.lua
      2. Assert: exit code 0
    Evidence: .sisyphus/evidence/task-7-file-exec.txt
  ```

  **Commit**: YES
  - Message: `feat(cli): add exec subcommand for inline Lua execution`
  - Files: `src/main.rs`
  - Pre-commit: `cargo test`

- 8. [ ] Context Output Formatter (`src/context.rs`)

  **What to do**:
  - Create `src/context.rs` with context output formatter
  - Implement `format_context(results: &[ModuleContextEntry]) -> String` formatting search results
    as prompt-ready text
  - Each module: name, description, auth pattern, env vars, quickref methods
  - Include builtins summary section
  - Include usage example with require() pattern
  - Output matches the "assay context Output Format" spec in Context section
  - Define `ModuleContextEntry` struct
  - Add `pub mod context;` to lib.rs
  - TDD: write tests FIRST in `tests/context.rs`, then implement

  **Must NOT do**:
  - Do NOT make network calls
  - Do NOT include raw source code in output
  - Do NOT exceed 120 chars per line for descriptions

  **Recommended Agent Profile**:
  - **Category**: `quick` — String formatting module
  - **Skills**: `[]`

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (after Wave 1)
  - **Blocks**: T10
  - **Blocked By**: T3

  **References**:
  - Context section "`assay context` Output Format" — authoritative output spec
  - `src/metadata.rs` (T3) — ModuleMetadata/QuickRef types used as input

  **Acceptance Criteria**:
  - [ ] `cargo test --test context` → PASS

  **QA Scenarios (MANDATORY)**:
  ```
  Scenario: Format grafana module entry
    Tool: Bash
    Steps:
      1. Create test with grafana metadata
      2. Call format_context()
      3. Assert: output contains "assay.grafana", method quickrefs, env vars
    Evidence: .sisyphus/evidence/task-8-format-grafana.txt

  Scenario: Empty results
    Tool: Bash
    Steps:
      1. Call format_context with empty vec
      2. Assert: output contains "No matching modules" or similar
    Evidence: .sisyphus/evidence/task-8-empty-results.txt
  ```

  **Commit**: YES
  - Message: `feat(context): add prompt-ready output formatter`
  - Files: `src/context.rs`, `src/lib.rs`, `tests/context.rs`
  - Pre-commit: `cargo test`

- 9. [ ] Backward-Compat Default `assay <file>` Command

  **What to do**:
  - Ensure `assay <file>` (no subcommand, just a file path) works exactly as v0.4.x
  - When CLI receives a positional arg that looks like a file (has .lua/.yaml/.yml extension),
    dispatch to run_lua_script() or run_yaml_checks() directly
  - This may already work from T2's fallback logic — this task verifies and fixes if needed
  - Write tests specifically for backward compatibility

  **Must NOT do**:
  - Do NOT change behavior of run_yaml_checks or run_lua_script
  - Do NOT add new file extensions

  **Recommended Agent Profile**:
  - **Category**: `quick` — Verification + small fix if needed
  - **Skills**: `[]`

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (after Wave 1)
  - **Blocks**: T12
  - **Blocked By**: T2

  **References**:
  - `src/main.rs:53-64` — current extension dispatch
  - T2's backward-compat implementation

  **Acceptance Criteria**:
  - [ ] `cargo test` → all pass

  **QA Scenarios (MANDATORY)**:
  ```
  Scenario: Lua file execution
    Tool: Bash
    Steps:
      1. Run: cargo run -- tests/e2e/check_json.lua
      2. Assert: exit code 0
    Evidence: .sisyphus/evidence/task-9-lua-compat.txt

  Scenario: YAML file execution
    Tool: Bash
    Steps:
      1. Run: cargo run -- tests/e2e/check_yaml.lua (if exists)
      2. Assert: exit code 0
    Evidence: .sisyphus/evidence/task-9-yaml-compat.txt

  Scenario: Unsupported extension
    Tool: Bash
    Steps:
      1. Run: cargo run -- test.txt
      2. Assert: exit code non-zero, error message shown
    Evidence: .sisyphus/evidence/task-9-unsupported-ext.txt
  ```

  **Commit**: YES
  - Message: `feat(cli): preserve backward-compat default run command`
  - Files: `src/main.rs`
  - Pre-commit: `cargo test`

- 10. [ ] `assay context <query>` Subcommand

  **What to do**:
  - THE CROWN JEWEL. Implement the context subcommand handler in src/main.rs
  - Wires together: discovery::discover_modules() → discovery::build_index() → index.search(query) →
    context::format_context()
  - Accepts `--limit N` (default 5)
  - Prints prompt-ready output to stdout
  - Empty query returns full module catalog
  - Handles gracefully: no modules found, no matches
  - Exit 0 always (output is for prompt injection, not for error checking)

  **Must NOT do**:
  - Do NOT make network calls
  - Do NOT read env vars for auth (just document them)
  - Do NOT cache index between runs

  **Recommended Agent Profile**:
  - **Category**: `deep` — Integration of all prior work
  - **Skills**: `[]`

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 3 (after Wave 2)
  - **Blocks**: T12, T13, T14
  - **Blocked By**: T2, T6, T8

  **References**:
  - `src/discovery.rs` (T6)
  - `src/context.rs` (T8)
  - `src/main.rs` (T2 CLI structure)

  **Acceptance Criteria**:
  - [ ] `cargo test` → all pass

  **QA Scenarios (MANDATORY)**:
  ```
  Scenario: Search for grafana
    Tool: Bash
    Steps:
      1. Run: cargo run -- context "grafana health"
      2. Assert: exit code 0, output shows grafana module with quickrefs
    Evidence: .sisyphus/evidence/task-10-grafana-search.txt

  Scenario: Search for builtins
    Tool: Bash
    Steps:
      1. Run: cargo run -- context "http request json"
      2. Assert: exit code 0, output shows http/json builtins
    Evidence: .sisyphus/evidence/task-10-builtins-search.txt

  Scenario: Empty query returns full catalog
    Tool: Bash
    Steps:
      1. Run: cargo run -- context ""
      2. Assert: exit code 0, output shows all modules
    Evidence: .sisyphus/evidence/task-10-full-catalog.txt

  Scenario: No matches
    Tool: Bash
    Steps:
      1. Run: cargo run -- context "nonexistent_xyz"
      2. Assert: exit code 0, output shows empty/helpful message
    Evidence: .sisyphus/evidence/task-10-no-matches.txt
  ```

  **Commit**: YES
  - Message: `feat(cli): add context subcommand for module search`
  - Files: `src/main.rs`
  - Pre-commit: `cargo test`

- 11. [ ] `assay modules` Subcommand

  **What to do**:
  - Implement modules subcommand in src/main.rs
  - Lists all available modules from all sources (embedded, project, global)
  - Shows: module name, source (built-in/project/global), description (first 80 chars)
  - Uses discovery::discover_modules()
  - Output format: table-like text to stdout
  - Group by source
  - Sort alphabetically within groups
  - Include count summary at bottom

  **Must NOT do**:
  - Do NOT include builtins in this listing (those are always available)
  - Do NOT make network calls

  **Recommended Agent Profile**:
  - **Category**: `quick` — Thin handler over discovery
  - **Skills**: `[]`

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 3 (after Wave 2)
  - **Blocks**: T12
  - **Blocked By**: T2, T6

  **References**:
  - `src/discovery.rs` (T6 — discover_modules())
  - `src/main.rs` (T2 CLI)

  **Acceptance Criteria**:
  - [ ] `cargo test` → all pass

  **QA Scenarios (MANDATORY)**:
  ```
  Scenario: List all modules
    Tool: Bash
    Steps:
      1. Run: cargo run -- modules
      2. Assert: exit code 0, output lists 23+ built-in modules
      3. Assert: each shows name and description
      4. Assert: output is sorted
    Evidence: .sisyphus/evidence/task-11-modules-list.txt
  ```

  **Commit**: YES
  - Message: `feat(cli): add modules subcommand for listing`
  - Files: `src/main.rs`
  - Pre-commit: `cargo test`

- 12. [ ] Integration + E2E Tests

  **What to do**:
  - Write integration tests covering all new CLI subcommands end-to-end
  - Test files in `tests/` directory
  - Cover: context search quality (grafana query returns grafana), exec inline + file modes, modules
    listing, backward compatibility (assay <file>.lua), module loading from ./modules/ filesystem
    path
  - Use `std::process::Command` to invoke the binary
  - Test error cases: invalid args, missing files, bad expressions

  **Must NOT do**:
  - Do NOT require external services (all tests self-contained)
  - Do NOT modify any source files

  **Recommended Agent Profile**:
  - **Category**: `deep` — Comprehensive test suite across all features
  - **Skills**: `[]`

  **Parallelization**:
  - **Can Run In Parallel**: NO (depends on all prior tasks)
  - **Parallel Group**: Wave 3 (after Wave 2)
  - **Blocks**: T15
  - **Blocked By**: T7, T9, T10, T11

  **References**:
  - All new src files (T2-T11)
  - `tests/common/mod.rs` — test helper patterns
  - Existing e2e tests in `tests/e2e/`

  **Acceptance Criteria**:
  - [ ] `cargo test` → all pass, test count increased by 10+

  **QA Scenarios (MANDATORY)**:
  ```
  Scenario: All integration tests pass
    Tool: Bash
    Steps:
      1. Run: cargo test
      2. Assert: all tests pass
      3. Assert: test count >= 10 new tests
    Evidence: .sisyphus/evidence/task-12-all-tests-pass.txt
  ```

  **Commit**: YES
  - Message: `test(e2e): add integration tests for new CLI subcommands`
  - Files: `tests/cli_*.rs` or `tests/integration.rs`
  - Pre-commit: `cargo test`

- 13. [ ] SKILL.md for LLM Agent Integration

  **What to do**:
  - Create `SKILL.md` at repo root
  - Teaches LLM agents how to use assay
  - Sections: What is Assay, Quick Start (context + exec workflow), Module Search (assay context),
    Script Execution (assay exec), Available Modules (table with all 23 + builtins), Authentication
    Patterns, Error Handling, Example Workflows (2-3 real-world scenarios)
  - Tone: concise, practical, example-heavy
  - Target audience: AI coding agents

  **Must NOT do**:
  - Do NOT include implementation details
  - Do NOT reference internal Rust code
  - Do NOT use marketing language

  **Recommended Agent Profile**:
  - **Category**: `writing` — Technical documentation
  - **Skills**: `[]`

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 4 (after Wave 3)
  - **Blocks**: T15
  - **Blocked By**: T10

  **References**:
  - `AGENTS.md` — format reference
  - Context section (module/builtin tables)
  - Existing README.md (examples to adapt)

  **Acceptance Criteria**:
  - [ ] SKILL.md exists, contains "assay context" and "assay exec" examples, file size > 2KB

  **QA Scenarios (MANDATORY)**:
  ```
  Scenario: SKILL.md exists and is comprehensive
    Tool: Bash
    Steps:
      1. Run: test -f SKILL.md && wc -l SKILL.md
      2. Assert: file exists, > 50 lines
      3. Run: grep -c "assay context" SKILL.md
      4. Assert: >= 1 match
      5. Run: grep -c "assay exec" SKILL.md
      6. Assert: >= 1 match
    Evidence: .sisyphus/evidence/task-13-skill-md.txt
  ```

  **Commit**: YES
  - Message: `docs(skill): add SKILL.md for LLM agent integration`
  - Files: `SKILL.md`
  - Pre-commit: —

- 14. [ ] README Rewrite for Universal Positioning

  **What to do**:
  - Rewrite README.md for universal API execution engine positioning
  - Keep: installation, usage examples, built-in API reference, development section
  - Update: tagline (not K8s-only), "What is Assay" section (universal focus), add "Module Search"
    section with `assay context` examples, add "LLM Integration" section referencing SKILL.md,
    update architecture diagram to show search/context flow
  - Remove/soften: exclusive K8s focus from intro
  - Keep K8s as a use case, not the identity

  **Must NOT do**:
  - Do NOT remove existing API reference tables
  - Do NOT break any links
  - Do NOT remove installation instructions

  **Recommended Agent Profile**:
  - **Category**: `writing` — Major doc rewrite
  - **Skills**: `[]`

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 4 (after Wave 3)
  - **Blocks**: T15
  - **Blocked By**: T10

  **References**:
  - `README.md` (current content to rewrite)
  - `SKILL.md` (T13 — reference for LLM section)
  - Context section (architecture and positioning info)

  **Acceptance Criteria**:
  - [ ] README.md mentions "context", "exec", "modules" subcommands, K8s is still mentioned but not
        sole focus, architecture diagram updated

  **QA Scenarios (MANDATORY)**:
  ```
  Scenario: README updated with new subcommands
    Tool: Bash
    Steps:
      1. Run: grep -c "assay context" README.md
      2. Assert: >= 1 match
      3. Run: grep -c "assay exec" README.md
      4. Assert: >= 1 match
      5. Run: grep -c "assay modules" README.md
      6. Assert: >= 1 match
    Evidence: .sisyphus/evidence/task-14-readme-updated.txt
  ```

  **Commit**: YES
  - Message: `docs(readme): rewrite for universal API engine positioning`
  - Files: `README.md`
  - Pre-commit: —

- 15. [ ] Version Bump v0.5.0 + Final Verification

  **What to do**:
  - Bump version in Cargo.toml from 0.4.5 to 0.5.0
  - Update description field to reflect universal positioning
  - Update keywords to include "api", "llm", "search"
  - Run full verification:
    `cargo check && cargo clippy -- -D warnings && cargo test && cargo build --release`
  - Verify binary size < 12MB
  - Verify `--version` output

  **Must NOT do**:
  - Do NOT change any functional code
  - Do NOT modify dependencies

  **Recommended Agent Profile**:
  - **Category**: `quick` — Single file change + verification
  - **Skills**: `[]`

  **Parallelization**:
  - **Can Run In Parallel**: NO (final task, depends on all others)
  - **Parallel Group**: Wave 4 (after Wave 3)
  - **Blocks**: F1-F4
  - **Blocked By**: all other tasks

  **References**:
  - `Cargo.toml:1-10` — version + metadata fields

  **Acceptance Criteria**:
  - [ ] `cargo run -- --version` shows 0.5.0
  - [ ] `cargo clippy -- -D warnings` zero warnings
  - [ ] `cargo test` all pass
  - [ ] Binary < 12MB

  **QA Scenarios (MANDATORY)**:
  ```
  Scenario: Version bump and verification
    Tool: Bash
    Steps:
      1. Run: cargo run -- --version
      2. Assert: output contains "0.5.0"
      3. Run: cargo clippy -- -D warnings
      4. Assert: exit code 0 (no warnings)
      5. Run: cargo test
      6. Assert: all tests pass
      7. Run: cargo build --release && ls -la target/release/assay
      8. Assert: binary size < 12MB
    Evidence: .sisyphus/evidence/task-15-version-bump.txt
  ```

  **Commit**: YES
  - Message: `chore: bump version to 0.5.0`
  - Files: `Cargo.toml`
  - Pre-commit: `cargo check && cargo clippy -- -D warnings && cargo test`

---

## Final Verification Wave (MANDATORY — after ALL implementation tasks)

> 4 review agents run in PARALLEL. ALL must APPROVE. Rejection → fix → re-run.

- [ ] F1. **Plan Compliance Audit** — `oracle` Read the plan end-to-end. For each "Must Have":
      verify implementation exists (read file, run command). For each "Must NOT Have": search
      codebase for forbidden patterns — reject with file:line if found. Check evidence files exist
      in `.sisyphus/evidence/`. Compare deliverables against plan. Check backward compatibility by
      running `cargo run -- tests/e2e/check_json.lua`. Output:
      `Must Have [N/N] | Must NOT Have [N/N] | Tasks [N/N] | VERDICT: APPROVE/REJECT`

- [ ] F2. **Code Quality Review** — `unspecified-high` Run
      `cargo check && cargo clippy -- -D warnings && cargo test`. Review all new files for: `unsafe`
      without justification, `unwrap()` in non-test code, dead code, unused imports. Check for AI
      slop: excessive comments, over-abstraction, generic names (data/result/item/temp). Verify
      binary size: `ls -la target/release/assay`. Output:
      `Build [PASS/FAIL] | Clippy [PASS/FAIL] | Tests [N pass/N fail] | Files [N clean/N issues] | VERDICT`

- [ ] F3. **Real Manual QA** — `unspecified-high` Start from clean state (`cargo build --release`).
      Execute EVERY QA scenario from EVERY task — follow exact steps, capture evidence. Test
      cross-task integration: `assay context "grafana"` then `assay exec -e` with the suggested
      code. Test edge cases: empty query, missing module dir, invalid metadata. Save to
      `.sisyphus/evidence/final-qa/`. Output:
      `Scenarios [N/N pass] | Integration [N/N] | Edge Cases [N tested] | VERDICT`

- [ ] F4. **Scope Fidelity Check** — `deep` For each task: read "What to do", read actual diff
      (`git log --oneline`). Verify 1:1 — everything in spec was built, nothing beyond spec was
      built. Check "Must NOT do" compliance. Detect cross-task contamination. Flag unaccounted
      changes. Verify no OpenAPI codegen, no MCP server, no registry code exists. Output:
      `Tasks [N/N compliant] | Contamination [CLEAN/N issues] | Unaccounted [CLEAN/N files] | VERDICT`

---

## Commit Strategy

| Task | Message                                                               | Key Files                                    | Pre-commit                                                 |
| ---- | --------------------------------------------------------------------- | -------------------------------------------- | ---------------------------------------------------------- |
| T1   | `feat(stdlib): add LDoc metadata headers to all 23 modules`           | `stdlib/*.lua`                               | `cargo test`                                               |
| T2   | `refactor(cli): restructure to clap subcommands with backward compat` | `src/main.rs`                                | `cargo check`                                              |
| T3   | `feat(metadata): add module metadata parser with function extraction` | `src/metadata.rs`, `tests/metadata.rs`       | `cargo test`                                               |
| T4   | `feat(search): add zero-dep BM25 search engine with field boosting`   | `src/search.rs`, `tests/search.rs`           | `cargo test`                                               |
| T5   | `feat(modules): extend loader for ~/.assay/modules/ and ./modules/`   | `src/lua/mod.rs`                             | `cargo test`                                               |
| T4b  | `feat(search): add FTS5 search backend for db feature`                | `src/search_fts5.rs`, `tests/search_fts5.rs` | `cargo test --test search_fts5`                            |
| T6   | `feat(discovery): add module discovery and BM25 index builder`        | `src/discovery.rs`, `tests/discovery.rs`     | `cargo test`                                               |
| T7   | `feat(cli): add exec subcommand for inline Lua execution`             | `src/main.rs`                                | `cargo test`                                               |
| T8   | `feat(context): add prompt-ready output formatter`                    | `src/context.rs`, `tests/context.rs`         | `cargo test`                                               |
| T9   | `feat(cli): preserve backward-compat default run command`             | `src/main.rs`                                | `cargo test`                                               |
| T10  | `feat(cli): add context subcommand for module search`                 | `src/main.rs`                                | `cargo test`                                               |
| T11  | `feat(cli): add modules subcommand for listing`                       | `src/main.rs`                                | `cargo test`                                               |
| T12  | `test(e2e): add integration tests for new CLI subcommands`            | `tests/cli_*.rs`                             | `cargo test`                                               |
| T13  | `docs(skill): add SKILL.md for LLM agent integration`                 | `SKILL.md`                                   | —                                                          |
| T14  | `docs(readme): rewrite for universal API engine positioning`          | `README.md`                                  | —                                                          |
| T15  | `chore: bump version to 0.5.0`                                        | `Cargo.toml`                                 | `cargo check && cargo clippy -- -D warnings && cargo test` |

---

## Success Criteria

### Verification Commands

```bash
cargo check                                              # Expected: no errors
cargo clippy -- -D warnings                              # Expected: no warnings
cargo test                                               # Expected: all pass
cargo build --release                                    # Expected: builds OK
ls -la target/release/assay                              # Expected: < 12MB
./target/release/assay --version                         # Expected: assay 0.5.0
./target/release/assay context "grafana health"          # Expected: grafana module info
./target/release/assay exec -e 'log.info("works")'      # Expected: prints "works"
./target/release/assay modules                           # Expected: lists 23+ modules
./target/release/assay tests/e2e/check_json.lua          # Expected: backward compat
```

### Final Checklist

- [ ] All "Must Have" present and verified
- [ ] All "Must NOT Have" absent (searched codebase)
- [ ] All 49+ existing tests still pass
- [ ] All new tests pass
- [ ] Binary size < 12MB
- [ ] Version 0.5.0 in Cargo.toml and --version output
- [ ] SKILL.md exists and is comprehensive
- [ ] README.md updated with universal positioning
- [ ] No new crate dependencies added for search
- [ ] Backward compatibility verified for .lua and .yaml modes
