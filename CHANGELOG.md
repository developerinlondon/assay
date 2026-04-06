# Changelog

All notable changes to Assay are documented here.

## [0.6.1] - 2026-04-06

### Fixed

- **http.serve async handlers**: Route handlers are now async (`call_async`), allowing
  them to call `http.get`, `sleep`, and any other async builtins. Previously, calling
  an async function from a route handler would crash with "attempt to yield from outside
  a coroutine". This was the only remaining sync call site for user Lua functions.

## [0.6.0] - 2026-04-05

### Added

- **6 new stdlib modules** (23 -> 29 total):
  - **assay.openclaw** — OpenClaw AI agent platform integration. Invoke tools, send messages,
    manage persistent state with JSON files, diff detection, approval gates, cron jobs, sub-agent
    spawning, and LLM task execution. Auto-discovers `$OPENCLAW_URL`/`$CLAWD_URL`.
  - **assay.github** — GitHub REST API client (no `gh` CLI dependency). Pull requests (view, list,
    reviews, merge), issues (list, get, create, comment), repositories, Actions workflow runs, and
    GraphQL queries. Bearer token auth via `$GITHUB_TOKEN`.
  - **assay.gmail** — Gmail REST API client with OAuth2 token auto-refresh. Search, read, reply,
    send emails, and list labels. Uses Google OAuth2 credentials and token files.
  - **assay.gcal** — Google Calendar REST API client with OAuth2 token auto-refresh. Events CRUD
    (list, get, create, update, delete) and calendar list. Same auth pattern as gmail.
  - **assay.oauth2** — Google OAuth2 token management. File-based credentials loading, automatic
    access token refresh via refresh_token grant, token persistence, and auth header generation.
    Used internally by gmail and gcal modules. Default paths: `~/.config/gog/credentials.json`
    and `~/.config/gog/token.json`.
  - **assay.email_triage** — Email classification and triage. Deterministic rule-based
    categorization of emails into needs_reply, needs_action, and fyi buckets. Optional
    LLM-assisted triage via OpenClaw for smarter classification. Subject and sender pattern
    matching for automated mail detection.
- **Tool mode**: `assay run --mode tool` for OpenClaw integration. Runs Lua scripts as
  deterministic tools invoked by AI agents, with structured JSON output.
- **Resume mechanism**: `assay resume --token <token> --approve yes|no` for resuming paused
  workflows after human approval gates.
- **OpenClaw extension**: `@developerinlondon/assay-openclaw-extension` package (GitHub Packages).
  Registers Assay as an OpenClaw agent tool with configurable script directory, timeout, output
  size limits, and approval-based resume flow.
  Install via `openclaw plugins install @developerinlondon/assay-openclaw-extension`.

### Architecture

- **Shell-free design**: All 6 new modules use native HTTP APIs exclusively. No shell commands,
  no CLI dependencies (no `gh`, no `gcloud`, no `oauth2l`). Pure Lua over Assay HTTP builtins.

## [0.5.6] - 2026-04-03

### Added

- **SSE streaming** for `http.serve` via `{ sse = function(send) ... end }` return shape. SSE
  handler runs async so `sleep()` and other async builtins work inside the producer. `send` callback
  uses async channel send with proper backpressure handling. Custom headers take precedence over SSE
  defaults (Content-Type, Cache-Control, Connection).
- **assert.ne(a, b, msg?)** — inequality assertion for the test framework.

### Fixed

- **Content-Type precedence**: User-provided `Content-Type` header no longer overwritten by defaults
  (`text/plain` / `application/json`) in `http.serve` responses.
- **SSE newline validation**: `event` and `id` fields reject values containing newlines or carriage
  returns to prevent SSE field injection.

## [0.5.5] - 2026-03-13

### Added

- **follow_redirects** option for YAML HTTP checks. Set `follow_redirects: false` to disable
  automatic redirect following, allowing verification of auth-protected endpoints that return 302
  redirects to identity providers. Defaults to `true` for backward compatibility.
- **follow_redirects** option for Lua `http.client()` builder. Create clients with
  `http.client({ follow_redirects = false })` for the same no-redirect behavior in scripts.

## [0.5.4] - 2026-03-12

### Fixed

- **unleash.ensure_token**: Send `tokenName` instead of `username` in create token API payload. The
  Unleash API expects `tokenName` — sending `username` caused HTTP 400 (BadDataError). Function now
  accepts both `opts.tokenName` and `opts.username` for backward compatibility. Existing token
  matching also checks `t.tokenName` with fallback to `t.username`.

## [0.5.3] - 2026-03-12

### Added

- **disk builtins**: `disk.usage(path)` and `disk.mounts()` for filesystem disk information
- **os builtins**: `os.info()` returning name, version, arch, hostname, uptime
- **Expanded fs builtins**: `fs.exists`, `fs.is_dir`, `fs.is_file`, `fs.list`, `fs.mkdir`,
  `fs.remove`, `fs.rename`, `fs.copy`, `fs.stat`, `fs.glob`, `fs.temp_dir`
- **Expanded env builtins**: `env.set`, `env.unset`, `env.list`, `env.home`, `env.cwd`

### Fixed

- Cross-platform casts in `disk.rs` (`u32` on macOS, `u64` on Linux)

## [0.5.2] - 2026-03-11

### Added

- **shell builtins**: `shell.run(cmd)`, `shell.output(cmd)`, `shell.which(name)`, `shell.pipe(cmds)`
- **process builtins**: `process.spawn(cmd, opts)`, `process.kill(pid)`, `process.pid()`,
  `process.list()`, `process.sleep(secs)`
- **Expanded fs builtins**: `fs.read_bytes`, `fs.write_bytes`, `fs.append`, `fs.symlink`,
  `fs.readlink`, `fs.canonicalize`, `fs.metadata`

### Fixed

- `http.serve` port race condition — use ephemeral ports with `_SERVER_PORT` global
- Symlink safety, timeout validation, pipe drain, PID validation hardening

## [0.5.1] - 2026-02-23

### Added

- **Website**: Static site at assay.rs on Cloudflare Pages with homepage, module reference, AI agent
  integration guides, and MCP comparison page mapping 42 servers
- **llms.txt**: LLM agent context traversal files (`llms.txt` and `llms-full.txt`)
- **Enriched search keywords**: All 23 stdlib modules and builtins enriched with `@keywords`
  metadata for improved discovery

### Changed

- Updated README with website links
- Updated SKILL.md with MCP comparison and agent integration guidance

## [0.5.0] - 2026-02-23

### Added

- **CLI subcommands**: `assay exec` for inline Lua execution, `assay context` for prompt-ready
  module output, `assay modules` for listing all available modules
- **Module discovery**: LDoc metadata parser with auto-function extraction from all 23 stdlib
  modules
- **Search engine**: Zero-dependency BM25 search with FTS5 backend for `db` feature
- **Filesystem module loader**: Project/global/builtin priority for `require()` resolution
- **LDoc metadata headers**: All 23 stdlib modules annotated with `@module`, `@description`,
  `@keywords`, `@quickref`

### Changed

- CLI restructured to clap subcommands with backward compatibility
- Feature flags added for optional `db`, `server`, and `cli` dependencies

## [0.4.4] - 2026-02-20

### Added

- **Unleash stdlib module** (`assay.unleash`): Feature flag management client for Unleash. Projects
  (CRUD, list), environments (enable/disable per project), features (CRUD, archive, toggle on/off),
  strategies (list, add), API tokens (CRUD). Idempotent helpers: `ensure_project`,
  `ensure_environment`, `ensure_token`.

## [0.4.3] - 2026-02-13

### Added

- **crypto.hmac**: HMAC builtin supporting all 8 hash algorithms (SHA-224/256/384/512,
  SHA3-224/256/384/512). Binary-safe key/data via `mlua::String`. Supports `raw` output mode for key
  chaining (required by AWS Sig V4). Manual RFC 2104 implementation using existing sha2/sha3 crates
  — zero new dependencies.
- **S3 stdlib module** (`assay.s3`): Pure Lua S3 client with AWS Signature V4 request signing. Works
  with any S3-compatible endpoint (AWS, iDrive e2, Cloudflare R2, MinIO). Operations: create/delete
  bucket, list buckets, put/get/delete/list/head/copy objects, bucket_exists. Path-style URLs
  default. Epoch-to-UTC date math (no os.date dependency). Simple XML response parsing via Lua
  patterns.
- 15 new tests (7 HMAC + 8 S3 stdlib)

### Changed

- **Modular builtins**: Split monolithic `builtins.rs` (1788 lines) into `src/lua/builtins/`
  directory with 10 focused modules: http, json, serialization, assert, crypto, db, ws, template,
  core, mod. Zero behavior change — pure refactoring for maintainability.

## [0.4.2] - 2026-02-13

### Fixed

- **zitadel.find_app**: Improved with name query filter and resilient 409 conflict handling

## [0.4.1] - 2026-02-13

### Fixed

- **zitadel.create_oidc_app**: Handle 409 conflict responses gracefully

## [0.4.0] - 2026-02-13

### Added

- **Zitadel stdlib module** (`assay.zitadel`): OIDC identity management with JWT machine auth
- **Postgres stdlib module** (`assay.postgres`): Postgres-specific helpers
- **Vault enhancements**: Additional vault helper functions
- **healthcheck.wait**: Wait helper for health check polling

### Fixed

- Use merge-patch content-type in `k8s.patch`

## [0.3.3] - 2026-02-12

### Added

- **Filesystem require fallback**: External Lua libraries can be loaded via filesystem `require()`

### Fixed

- Load K8s CA cert for in-cluster HTTPS API calls

## [0.3.2] - 2026-02-11

### Added

- **crypto.jwt_sign**: `kid` (Key ID) header support for JWT signing

### Fixed

- Release workflow: Filter artifact download to exclude Docker metadata

## [0.3.1] - 2026-02-11

- Publish crate as `assay-lua` on crates.io (binary still installs as `assay`)
- Add release pipeline: pre-built binaries (Linux x86_64 static, macOS Apple Silicon), Docker,
  crates.io
- Add prerequisite docs to K8s-dependent examples
- Fix flaky sleep timing test

## [0.3.0] - 2026-02-11

First feature-complete release. Assay is now a general-purpose Lua runtime for Kubernetes — covering
verification, scripting, automation, and lightweight web services in a single ~9 MB binary.

### Added

- **Direct Lua execution**: `assay script.lua` with auto-detection by file extension
- **Shebang support**: `#!/usr/bin/assay` for executable Lua scripts
- **HTTP server**: `http.serve(port, routes)` — Lua scripts become web services
- **Database access**: `db.connect/query/execute` — PostgreSQL, MySQL/MariaDB, SQLite via sqlx
- **WebSocket client**: `ws.connect/send/recv/close` via tokio-tungstenite
- **Template engine**: `template.render/render_string` via minijinja (Jinja2-compatible)
- **Filesystem write**: `fs.write(path, content)` complements existing `fs.read`
- **YAML builtins**: `yaml.parse/encode` for YAML processing in Lua scripts
- **TOML builtins**: `toml.parse/encode` for TOML processing in Lua scripts
- **Async primitives**: `async.spawn(fn)` and `async.spawn_interval(ms, fn)` with handles
- **Crypto hash**: `crypto.hash(algo, data)` — SHA-256, SHA-384, SHA-512, SHA3-256, SHA3-512
- **Crypto random**: `crypto.random(length)` — cryptographically secure random hex strings
- **JWT signing**: `crypto.jwt_sign(claims, key, algo)` — RS256/RS384/RS512
- **Regex**: `regex.match/find/find_all/replace` via regex-lite
- **Base64**: `base64.encode/decode`
- **19 stdlib modules**: prometheus, alertmanager, loki, grafana, k8s, argocd, kargo, flux, traefik,
  vault, openbao, certmanager, eso, dex, crossplane, velero, temporal, harbor, healthcheck
- **E2E dogfood tests**: Assay testing itself via YAML check mode
- **CI**: GitHub Actions with clippy + tests on Linux (x86_64) and macOS (Apple Silicon)
- **491 tests**, 0 clippy warnings

### Changed

- CLI changed from `assay --config file.yaml` to `assay <file>` (positional arg, auto-detect)
- Lua upgraded from 5.4 to 5.5 (global declarations, incremental major GC, compact arrays)
- HTTP builtins DRYed (collapsed 4x duplicated method registrations into generic loop)

## [0.0.1] - 2026-02-09

Initial release. YAML-based check orchestration for ArgoCD PostSync verification.

### Added

- YAML config with timeout, retries, backoff, parallel execution
- Check types: `type: http`, `type: prometheus`, `type: script` (Lua)
- Built-in retry with exponential backoff
- Structured JSON output with pass/fail per check
- K8s-native exit codes (0 = all passed, 1 = any failed)
- HTTP client builtins: `http.get/post/put/patch`
- JSON builtins: `json.parse/encode`
- Assert builtins: `assert.eq/gt/lt/contains/not_nil/matches`
- Logging builtins: `log.info/warn/error`
- Environment: `env.get`, `sleep`, `time`
- Prometheus stdlib module
- Docker image: Alpine 3.21 + ~5 MB binary
