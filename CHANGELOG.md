# Changelog

All notable changes to Assay are documented here.

## [0.4.3] - 2026-02-13

### Added

- **crypto.hmac**: HMAC builtin supporting all 8 hash algorithms (SHA-224/256/384/512,
  SHA3-224/256/384/512). Binary-safe key/data via `mlua::String`. Supports `raw` output mode for
  key chaining (required by AWS Sig V4). Manual RFC 2104 implementation using existing sha2/sha3
  crates — zero new dependencies.
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
