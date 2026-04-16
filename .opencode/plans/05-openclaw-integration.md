# Plan 05: OpenClaw Integration — Full Lobster Replacement (v0.6.0)

**Status**: Approved — implementation in progress **Created**: 2026-04-05 **Branch**:
`feat/openclaw-integration` **Version**: 0.5.6 → 0.6.0 (minor release — new capability domain)

## Context

Assay is replacing Lobster as the workflow runtime for OpenClaw. Lobster is a Node.js YAML pipeline
runner (~80MB runtime). Assay replaces it with a 9MB static binary, proper control flow (Lua),
parallel execution, error handling, and 27 stdlib modules Lobster has no equivalent for.

This plan covers two directions of integration:

- **Assay → OpenClaw**: Lua modules that call the OpenClaw HTTP API (stdlib)
- **OpenClaw → Assay**: TypeScript plugin that spawns the `assay` binary (extension)

## Architecture

```
+------------------------------------------------------------------+
|               OpenClaw Server (TypeScript)                        |
|                                                                  |
|  extensions/lobster/  <-- existing, spawns lobster (DEPRECATED)   |
|                                                                  |
|  User installs @assay/openclaw-extension via:                    |
|    openclaw plugins install @assay/openclaw-extension             |
|                                                                  |
|  POST /tools/invoke   <-- HTTP gateway (already exists)          |
+--------+--------------------------+------------------------------+
         | spawns                   | HTTP responses
         v                         |
+------------------+    +-----------+------------------------------+
| assay binary     |    | assay script (standalone mode)            |
| (tool mode)      |    |                                          |
|                  |    | local oc = require("assay.openclaw")      |
| --mode tool      |    | c:invoke("message","send",{...})          |
| JSON envelope    |    |    --- HTTP POST ---> OpenClaw API        |
| on stdout        |    |                                          |
| resume support   |    | c:state_set("key", data)                  |
+------------------+    |    --- local fs ---> ~/.assay/state/      |
                        +------------------------------------------+
```

```
+------------------------------------------------------------------+
|                    WHAT LIVES WHERE                               |
|                                                                  |
|  assay repo (Rust + Lua + TS extension):                         |
|  +-- src/                     Rust runtime                       |
|  |   +-- tool_mode.rs         NEW: --mode tool envelope output   |
|  |   +-- resume.rs            NEW: assay resume --token X        |
|  +-- stdlib/                  Lua modules                        |
|  |   +-- openclaw.lua         DONE: OpenClaw API client          |
|  |   +-- github.lua           DONE: GitHub REST API              |
|  |   +-- gmail.lua            DONE: Gmail REST + OAuth2          |
|  |   +-- gcal.lua             DONE: Google Calendar + OAuth2     |
|  |   +-- oauth2.lua           TODO: Shared token management      |
|  |   +-- email_triage.lua     TODO: Email classification         |
|  +-- openclaw-extension/      NEW: TypeScript OpenClaw plugin    |
|  |   +-- package.json         @assay/openclaw-extension          |
|  |   +-- openclaw.plugin.json Plugin manifest                    |
|  |   +-- index.ts             register(api) entry point          |
|  |   +-- src/assay-tool.ts    Spawn assay, parse JSON envelope   |
|  |   +-- SKILL.md             Agent-facing docs                  |
|  |   +-- README.md            User-facing docs                   |
|  +-- tests/                   Wiremock-based Rust tests           |
|  +-- examples/                Example Lua scripts                |
|                                                                  |
|  openclaw-ts repo: NO CHANGES (users install extension via npm)  |
+------------------------------------------------------------------+
```

## What's Already Done

Commit `c77b505` on `feat/openclaw-integration` (20 files, +2173/-100):

| Item                       | Status | Detail                             |
| -------------------------- | ------ | ---------------------------------- |
| `stdlib/openclaw.lua`      | Done   | 166 lines, 12 methods, 7 tests     |
| `stdlib/github.lua`        | Done   | 179 lines, 12 methods, 9 tests     |
| `stdlib/gmail.lua`         | Done   | 204 lines, 5 methods, 6 tests      |
| `stdlib/gcal.lua`          | Done   | 170 lines, 6 methods, 6 tests      |
| Cargo.toml version bump    | Done   | 0.5.6 → 0.6.0                      |
| CHANGELOG.md               | Done   | v0.6.0 section                     |
| README.md                  | Done   | Module count 23→27, new table rows |
| SKILL.md                   | Done   | Module count 23→27, new table rows |
| AGENTS.md                  | Done   | Description + module table updated |
| llms.txt (root + site)     | Done   | New AI Agent & Workflow section    |
| site/llms-full.txt         | Done   | Full code examples for 4 modules   |
| site/modules.html          | Done   | New HTML section with examples     |
| examples/openclaw-health   | Done   | 22 lines                           |
| examples/github-pr-monitor | Done   | 37 lines                           |

## What Remains

### Sprint 1: Stdlib Completion (Lua only, no Rust)

- [ ] **1.1** Extract `stdlib/oauth2.lua` (~120 lines) from duplicated inline code in gmail.lua and
      gcal.lua. Functions: `M.from_file(creds_path, token_path)`, `client:access_token()`,
      `client:refresh()`, `client:save()`. Default paths: `~/.config/gog/credentials.json` and
      `~/.config/gog/token.json`. Support overrides via opts table.
- [ ] **1.2** Refactor `gmail.lua` and `gcal.lua` to `require("assay.oauth2")` internally instead of
      inline token refresh. No API changes to gmail/gcal — purely internal refactor.
- [ ] **1.3** Create `stdlib/email_triage.lua` (~100 lines). Functions: `M.categorize(emails, opts)`
      — deterministic rules (subject keywords, noreply detection), returns
      `{needs_reply={}, needs_action={}, fyi={}}`. `M.categorize_llm(emails, openclaw_client, opts)`
      — uses OpenClaw LLM task for smarter classification + optional draft replies.
- [ ] **1.4** Add tests for `oauth2.lua` — wiremock tests for token refresh, expired token, file
      persistence, error handling (~6 tests)
- [ ] **1.5** Add tests for `email_triage.lua` — unit tests for deterministic categorization,
      wiremock test for LLM-assisted mode (~5 tests)
- [ ] **1.6** Fill test gaps in existing modules (~12 missing method tests): - openclaw: `notify`,
      `cron_add`, `spawn`, `approve` - github: `pr_reviews`, `pr_merge`, `issue_get`,
      `issue_comment`, `run_get` - gmail: `reply` - gcal: `event_update`, token refresh
- [ ] **1.7** Add example scripts: `examples/gmail-digest.lua`, `examples/gcal-daily-agenda.lua`,
      `examples/email-triage.lua`

### Sprint 2: Rust Tool-Mode Envelope

Assay needs a "tool mode" so OpenClaw can spawn it and read structured JSON from stdout.

- [ ] **2.1** Add `--mode tool` CLI flag (clap). When set, assay wraps script output in a JSON
      envelope on stdout:
      `json
      {
        "ok": true,
        "status": "ok",
        "output": "<script return value as JSON>",
        "requiresApproval": null
      }`
      Error case: `{ "ok": false, "status": "error", "error": "<message>" }`
- [ ] **2.2** Also support `ASSAY_MODE=tool` env var (same behavior as `--mode tool`). Env var takes
      precedence if both are set.
- [ ] **2.3** In tool mode, suppress all log.info/log.warn output to stderr (not stdout). Only the
      final JSON envelope goes to stdout.
- [ ] **2.4** Add stdout size cap (512KB, matching Lobster's limit) and execution timeout
      (configurable via `--timeout <secs>`, default 20s).
- [ ] **2.5** Tests for tool mode: envelope format, error envelope, timeout, stdout cap, stderr
      separation (~8 tests)

### Sprint 3: Resume/Halt Mechanism (Rust)

For approval gates to work when Assay runs as an OpenClaw tool, the binary needs to halt
mid-execution and resume later.

- [ ] **3.1** Add `assay resume --token <token> --approve yes|no` CLI subcommand.
- [ ] **3.2** When `openclaw.approve()` is called in tool mode, the script halts and emits:
      `json
      {
        "ok": true,
        "status": "needs_approval",
        "requiresApproval": {
          "prompt": "Deploy to production?",
          "context": { ... },
          "resumeToken": "<base64-encoded state>"
        }
      }`
- [ ] **3.3** State serialization: save Lua VM state to `~/.assay/state/resume/<token>.json`. State
      includes: script path, environment, approval context, and a continuation marker. (Full VM
      serialization is not feasible — the resume re-runs the script with the approval result
      injected, matching Lobster's pattern.)
- [ ] **3.4** On `assay resume --token X --approve yes`, load state, re-execute script with
      `ASSAY_APPROVAL_RESULT=yes` env var. The `openclaw.approve()` function checks this var and
      returns the result immediately instead of halting.
- [ ] **3.5** Resume token expiry: state files expire after 1 hour (configurable via
      `--resume-ttl <secs>`). Expired tokens return error envelope.
- [ ] **3.6** Tests for resume: approve/reject flow, expired token, invalid token, state cleanup (~7
      tests)

### Sprint 4: OpenClaw Extension (TypeScript, in assay repo)

The TypeScript plugin that lets OpenClaw spawn and communicate with the assay binary. Lives in
`assay/openclaw-extension/` and is published to npm as `@assay/openclaw-extension`.

- [ ] **4.1** Create `openclaw-extension/package.json`:
      `json
      {
        "name": "@assay/openclaw-extension",
        "version": "0.6.0",
        "description": "Assay workflow runtime for OpenClaw",
        "openclaw": { "extensions": ["./index.ts"] },
        "peerDependencies": { "openclaw": "*" }
      }`
- [ ] **4.2** Create `openclaw-extension/openclaw.plugin.json`:
      `json
      {
        "id": "assay",
        "name": "Assay Workflow Runtime",
        "description": "Run Lua workflow scripts via Assay",
        "configSchema": {
          "type": "object",
          "properties": {
            "binaryPath": { "type": "string", "description": "Path to assay binary" },
            "timeout": { "type": "number", "default": 20 },
            "maxOutputSize": { "type": "number", "default": 524288 },
            "stateDir": { "type": "string" },
            "scriptsDir": { "type": "string" }
          }
        }
      }`
- [ ] **4.3** Create `openclaw-extension/index.ts` — `register(api)` that calls `api.registerTool()`
      with optional flag, skip when sandboxed.
- [ ] **4.4** Create `openclaw-extension/src/assay-tool.ts` (~250 lines): - `run` action: spawn
      `assay run --mode tool <script>`, parse JSON envelope from stdout - `resume` action: spawn
      `assay resume --token <token> --approve yes|no` - Sandboxed CWD (relative paths only, no path
      traversal) - Timeout + stdout cap enforcement - Tolerant JSON parser (skip debug output before
      final JSON, match Lobster's pattern)
- [ ] **4.5** Create `openclaw-extension/SKILL.md` (~100 lines) — agent-facing documentation: when
      to use assay vs shell, available modules, script patterns, approval workflow. Must list all 29
      stdlib modules (27 existing + oauth2 + email_triage).
- [ ] **4.6** Create `openclaw-extension/README.md` — user-facing: installation, configuration,
      security model, examples.
- [ ] **4.7** Tests for the tool: mock assay binary spawning, envelope parsing, timeout, sandbox
      path validation, resume flow (~8 tests)

### Sprint 5: Documentation & Release

- [ ] **5.1** Update CHANGELOG.md with complete v0.6.0 notes covering all new modules, tool mode,
      resume support, and OpenClaw extension.
- [ ] **5.2** Update README.md — add OpenClaw integration section, update module count to 29, add
      `openclaw-extension` section.
- [ ] **5.3** Update AGENTS.md — module count 27→29, add tool mode and extension docs.
- [ ] **5.4** Update SKILL.md — module count 27→29.
- [ ] **5.5** Update site/modules.html — add oauth2 and email_triage to the Agent & Workflow
      section.
- [ ] **5.6** Update llms.txt, site/llms.txt, site/llms-full.txt — add oauth2 and email_triage docs.
- [ ] **5.7** Run full verification: `cargo clippy --tests -- -D warnings && cargo test`
- [ ] **5.8** Verify openclaw-extension builds: `cd openclaw-extension && npm install && npm test`

## Key Design Decisions

| Decision               | Choice                     | Why                                        |
| ---------------------- | -------------------------- | ------------------------------------------ |
| Version                | 0.6.0 (minor)              | New capability domain, 6+ new modules      |
| email_triage location  | stdlib module              | Reusable, not just an example              |
| OAuth2 credentials     | gog config files default   | Existing infra, overridable via opts       |
| State directory        | ~/.assay/state/            | Overridable via ASSAY_STATE_DIR env var    |
| Extension packaging    | In assay repo              | Versions with assay, team controls it      |
| Extension distribution | npm as @assay/openclaw-ext | Users: openclaw plugins install ...        |
| YAML workflow compat   | Skipped                    | Clean break — Lua only, no .lobster shim   |
| Shell-free             | All HTTP-native            | No subprocess overhead, proper errors      |
| Resume mechanism       | Re-run with env var        | Full VM serialization not feasible in Lua  |
| Tool mode output       | JSON envelope on stdout    | Matches Lobster protocol for compatibility |

## Extension Versioning Strategy

The `openclaw-extension/` directory has its own `package.json` with semver. The version tracks
Assay's version (both start at 0.6.0) but can diverge if the extension needs a hotfix without a
runtime change. The SKILL.md is the coupling point — it must accurately reflect which Assay
version's modules are available.

```
assay v0.6.0  <-->  @assay/openclaw-extension v0.6.0   (initial release)
assay v0.6.1  <-->  @assay/openclaw-extension v0.6.1   (both get bugfixes)
assay v0.7.0  <-->  @assay/openclaw-extension v0.7.0   (new modules added)
assay v0.7.0  <-->  @assay/openclaw-extension v0.7.1   (extension-only fix)
```

## Dependency Graph

```
stdlib/oauth2.lua          (standalone)
stdlib/openclaw.lua        (standalone)
stdlib/github.lua          (standalone)
stdlib/gmail.lua       --> oauth2.lua (internal require, lazy)
stdlib/gcal.lua        --> oauth2.lua (internal require, lazy)
stdlib/email_triage.lua -> gmail.lua + openclaw.lua (lazy require)

src/tool_mode.rs       --> src/main.rs (CLI integration)
src/resume.rs          --> src/tool_mode.rs (reuses envelope format)

openclaw-extension/    --> assay binary (spawns it as subprocess)
                       --> openclaw/plugin-sdk (peerDependency)
```

## Effort Estimate

| Sprint                       | Effort   | Blocked by |
| ---------------------------- | -------- | ---------- |
| Sprint 1: Stdlib completion  | ~4h      | Nothing    |
| Sprint 2: Rust tool-mode     | ~3h      | Nothing    |
| Sprint 3: Resume mechanism   | ~3h      | Sprint 2   |
| Sprint 4: OpenClaw extension | ~3h      | Sprint 2   |
| Sprint 5: Docs & release     | ~2h      | All above  |
| **Total**                    | **~15h** |            |

Sprints 1 and 2 can run in parallel. Sprint 3 and 4 depend on Sprint 2. Sprint 5 is last.
