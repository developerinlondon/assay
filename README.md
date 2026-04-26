# Assay

**One static binary that replaces Temporal + Kratos + Hydra + Keto.** Plus a full Lua 5.5 runtime
with 65 modules for Kubernetes, monitoring, secrets, and AI agents.

[![CI](https://github.com/developerinlondon/assay/actions/workflows/ci.yml/badge.svg)](https://github.com/developerinlondon/assay/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/assay-lua.svg)](https://crates.io/crates/assay-lua)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)

## What is Assay?

Two binaries, one project. `FROM scratch`-shippable, PG18 + SQLite first-class. Sizes today: `assay`
~12 MB, `assay-engine` ~19 MB (the engine grew with the auth + IdP work in v0.2.0).

- **`assay`** — Lua 5.5 runtime with 45 stdlib modules (Kubernetes, Prometheus, Vault, GitHub,
  Gmail, OpenClaw, Tailscale, …). Drop-in replacement for 50-250 MB Python/Node/kubectl scripting
  containers.
- **`assay-engine`** — durable **workflow engine** (Temporal-replacement: deterministic-replay
  activities, signals, timers, child workflows, schedules, search attributes) **+ full IdP**
  (Kratos + Hydra + Keto replacement: OIDC client + provider, passkey, JWT/JWKS rotation, biscuit
  capability tokens, Argon2 password, Zanzibar ReBAC, session management, admin HTTP API, dashboard
  panes for everything).

```bash
# Lua runtime
assay script.lua     # Run Lua with all builtins
assay checks.yaml    # Structured checks with retry/backoff/JSON output
assay exec -e 'log.info("hello")'   # Inline evaluation
assay context "grafana"              # LLM-ready module docs
assay modules                        # List all 65 modules

# Workflow + auth + dashboard server (one process)
assay-engine serve --config engine.toml
#   workflow API  → /api/v1/workflows + dashboard at /workflow/
#   auth/IdP API  → /auth/* (OIDC discovery at /.well-known/openid-configuration)
#   admin SPA     → /auth/console
```

Scripts that call `http.serve()` become web services. Scripts that call `http.get()` and exit are
jobs. The runtime talks to a deployed `assay-engine` over HTTP via the `assay.workflow` and
`assay.auth` stdlib modules — same binary, same builtins.

## Replaces what?

| Component                     | Replaces                  | Notes                                                  |
| ----------------------------- | ------------------------- | ------------------------------------------------------ |
| `assay-engine` workflow       | **Temporal**              | Same `define`/`execute_activity`/`wait_for_signal` API |
| `assay-engine` auth (session) | **Ory Kratos** (sessions) | Cookie + CSRF + Argon2id                               |
| `assay-engine` auth (passkey) | **Ory Kratos** (WebAuthn) | `webauthn-rs`-backed register + auth ceremonies        |
| `assay-engine` auth (OIDC OP) | **Ory Hydra**             | RFC 7009 revoke, RFC 7662 introspect, JWKS rotation    |
| `assay-engine` auth Zanzibar  | **Ory Keto / SpiceDB**    | Recursive-CTE walk on PG18 + SQLite                    |
| `assay-engine` auth biscuit   | (Ory has nothing)         | Datalog-attenuable capability tokens — built-in        |
| `assay-engine` dashboard      | Ory Console + Temporal UI | Single SPA, auth panes appear when auth is on          |
| `assay` runtime               | Python / Node + kubectl   | 12 MB, 5 ms cold start, 45 stdlib modules              |

## Two binaries, two use cases

| Use case                     | Binary         | Install                                       |
| ---------------------------- | -------------- | --------------------------------------------- |
| Scripting / automation       | `assay`        | `cargo install assay-lua` or download release |
| Workflow + auth + IdP server | `assay-engine` | `cargo install assay-engine` or Docker        |

`assay` runs Lua scripts with the full 45-module stdlib; for workflows/auth it talks to a deployed
`assay-engine` over HTTP. `assay-engine` is a standalone HTTP server with workflow + auth +
dashboard, pluggable across PG18 (default) and SQLite — both backends compiled in, runtime-selected
via config.

See [docs/migration-to-0.2.0.md](./docs/migration-to-0.2.0.md) for the upgrade path from v0.1.x.

## Why Assay?

| Runtime          | Compressed |   On-disk | vs Assay | Cold Start | K8s-native |
| ---------------- | ---------: | --------: | :------: | ---------: | :--------: |
| **assay**        |  **~9 MB** | **12 MB** |  **1x**  |   **5 ms** |  **Yes**   |
| **assay-engine** | **~14 MB** | **19 MB** |  **1x**  |   **8 ms** |  **Yes**   |
| Python alpine    |      17 MB |     50 MB |    2x    |     300 ms |     No     |
| bitnami/kubectl  |      35 MB |     90 MB |    4x    |     200 ms |  Partial   |
| Node.js alpine   |      57 MB |    180 MB |    6x    |     500 ms |     No     |
| Deno             |      75 MB |    200 MB |    8x    |      50 ms |     No     |
| Bun              |     115 MB |    250 MB |   13x    |      30 ms |     No     |
| postman/newman   |     128 MB |    350 MB |   14x    |     800 ms |     No     |

For comparison, the stack `assay-engine` replaces — Temporal server + UI + Kratos + Hydra + Keto +
their Postgres deps — typically lands at **800 MB-1.5 GB compressed** across 5+ containers.

## Installation

```bash
# Pre-built binaries (Linux x86_64 static, both binaries)
curl -L -o assay https://github.com/developerinlondon/assay/releases/latest/download/assay-linux-x86_64
curl -L -o assay-engine https://github.com/developerinlondon/assay/releases/latest/download/assay-engine-linux-x86_64
chmod +x assay assay-engine && sudo mv assay assay-engine /usr/local/bin/

# macOS (Apple Silicon)
curl -L -o assay https://github.com/developerinlondon/assay/releases/latest/download/assay-darwin-aarch64
curl -L -o assay-engine https://github.com/developerinlondon/assay/releases/latest/download/assay-engine-darwin-aarch64
chmod +x assay assay-engine && sudo mv assay assay-engine /usr/local/bin/

# Docker
docker pull ghcr.io/developerinlondon/assay:latest         # runtime
docker pull ghcr.io/developerinlondon/assay-engine:latest  # engine

# Cargo
cargo install assay-lua      # the `assay` runtime binary
cargo install assay-engine   # the workflow + auth server
```

## Auth + IdP quick-start

Once `assay-engine` is running with the auth module enabled, every IdP capability is reachable over
HTTP and from Lua via the `assay.auth` stdlib module:

```bash
# engine.toml — minimum viable v0.2.0 with auth on
cat > engine.toml <<'TOML'
auto_enable_modules = ["auth"]

[server]
bind_addr = "0.0.0.0:3000"
public_url = "https://auth.example.com"

[backend]
type = "postgres"
url = "postgres://postgres:postgres@localhost/assay"

[auth]
issuer = "https://auth.example.com/auth"
admin_api_keys = ["sk_admin_replace_me"]
TOML

assay-engine serve --config engine.toml
#   /auth/console                          → admin SPA
#   /.well-known/openid-configuration      → OIDC discovery (Hydra-equivalent)
#   /auth/login, /auth/passkey/*           → user-facing auth flows
#   /auth/admin/auth/*                     → admin HTTP API (api-key gated)
```

```lua
-- Use the assay-auth stdlib module from the assay (Lua) runtime
local auth = require("assay.auth")
local c = auth.client({ engine_url = "http://localhost:3000" })

local sess = c:login("alice@example.com", "hunter2")
local me   = c:whoami()
local ok   = c.zanzibar:check("doc", "doc-42", "read", "user", me.id)

-- Federated SSO (e.g. Google)
local redirect = c.oidc:start("google")        -- returns redirect URL
-- ...user round-trips through Google...
local sess2    = c.oidc:complete("google", code, state)

-- Issue a Datalog-attenuable biscuit capability token
local pem = c.biscuit:public_pem()             -- cache the engine's root pubkey
```

Hook `assay-engine` up to any OIDC consumer (Immich, Grafana, ArgoCD, Nextcloud, …) by registering a
client via `c.oidc_clients:create({...})` or the dashboard's OIDC Clients pane. The engine ships RFC
7009 token revocation, RFC 7662 introspection, JWKS rotation, back-channel logout, and PKCE-enforced
authorization-code flow out of the box — full Hydra parity in one process.

## Builtins API Reference

All 17 Rust builtins are available globally in `.lua` scripts — no `require` needed.

### HTTP & Networking

| Function                          | Description                                                                                                                                                                   |
| --------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `http.get(url, opts?)`            | GET request, returns `{status, body, headers}`                                                                                                                                |
| `http.post(url, body, opts?)`     | POST (auto-JSON if body is table)                                                                                                                                             |
| `http.put/patch/delete(url, ...)` | PUT, PATCH, DELETE                                                                                                                                                            |
| `http.serve(port, routes)`        | HTTP server with async handlers + SSE streaming (header values can be strings or arrays — array values emit the header multiple times for `Set-Cookie`, `Link`, `Vary`, etc.) |
| `ws.connect(url)`                 | WebSocket client (`send`, `recv`, `close`)                                                                                                                                    |

### Serialization

| Function                                    | Description |
| ------------------------------------------- | ----------- |
| `json.parse(str)` / `json.encode(tbl)`      | JSON        |
| `yaml.parse(str)` / `yaml.encode(tbl)`      | YAML        |
| `toml.parse(str)` / `toml.encode(tbl)`      | TOML        |
| `base64.encode(str)` / `base64.decode(str)` | Base64      |

### Filesystem & System

| Function                                                  | Description                     |
| --------------------------------------------------------- | ------------------------------- |
| `fs.read(path)` / `fs.write(path, s)`                     | Read/write files                |
| `fs.exists(path)` / `fs.mkdir(path)` / `fs.glob(pattern)` | File operations                 |
| `shell.exec(cmd, opts?)`                                  | Execute shell commands          |
| `process.list()` / `process.kill(pid)`                    | Process management              |
| `disk.usage(path)` / `disk.sweep(dir, age)`               | Disk info and cleanup           |
| `os.hostname()` / `os.arch()` / `os.platform()`           | OS information                  |
| `env.get(key)` / `env.set(key, val)`                      | Environment variables           |
| `sleep(secs)` / `time()`                                  | Pause execution, Unix timestamp |

### Cryptography & Regex

| Function                                   | Description                                |
| ------------------------------------------ | ------------------------------------------ |
| `crypto.jwt_sign(claims, key, alg, opts?)` | Sign JWT (HS256, RS256/384/512, ES256/384) |
| `crypto.hash(str, alg)`                    | SHA-256, SHA-384, SHA-512, SHA3            |
| `crypto.hmac(key, data, alg?, raw?)`       | HMAC (all 8 hash algorithms)               |
| `crypto.random(len)`                       | Secure random hex string                   |
| `regex.match/find/find_all/replace`        | Regular expressions                        |

### Database, Templates & Async

| Function                                             | Description                 |
| ---------------------------------------------------- | --------------------------- |
| `db.connect(url)`                                    | Postgres, MySQL, SQLite     |
| `db.query(conn, sql, params?)`                       | Execute query, return rows  |
| `template.render(path, vars)`                        | Jinja2-compatible templates |
| `async.spawn(fn)` / `async.spawn_interval(secs, fn)` | Async tasks with handles    |

### Assertions & Logging

| Function                                      | Description        |
| --------------------------------------------- | ------------------ |
| `assert.eq/ne/gt/lt/contains/not_nil/matches` | Test assertions    |
| `log.info/warn/error(msg)`                    | Structured logging |

## Stdlib Modules

36 embedded Lua modules loaded via `require("assay.<name>")`. Most follow the client pattern:
`M.client(url, opts)` then `c:method()`. A few utilities (`ansi`, `url`, `version`) are pure
functions and can be called directly off the module table.

The table below is generated by `assay site/build.lua` from the `category:` frontmatter in each
`docs/modules/<slug>.md`. Edit the frontmatter / docs, not the table.

<!-- BEGIN STDLIB TABLE -->
<!-- Generated by site/build.lua from docs/modules/*.md frontmatter — do not edit by hand. -->

| Module | Description |
| --- | --- |
| **Monitoring & Observability** | |
| `assay.alertmanager` |  |
| `assay.grafana` |  |
| `assay.loki` |  |
| `assay.prometheus` |  |
| **Kubernetes & GitOps** | |
| `assay.argocd` |  |
| `assay.flux` |  |
| `assay.k8s` |  |
| `assay.kargo` |  |
| `assay.traefik` |  |
| **Security & Identity** | |
| `assay.certmanager` |  |
| `assay.dex` |  |
| `assay.eso` |  |
| `assay.openbao` |  |
| `assay.ory` |  |
| `assay.vault` |  |
| `assay.zitadel` |  |
| **Infrastructure** | |
| `assay.apt` |  |
| `assay.crossplane` |  |
| `assay.harbor` |  |
| `assay.tailscale` |  |
| `assay.velero` |  |
| **Data & Storage** | |
| `assay.postgres` |  |
| `assay.s3` |  |
| **Feature Flags & Health** | |
| `assay.healthcheck` |  |
| `assay.unleash` |  |
| **Text, URLs & Versions** | |
| `assay.ansi` |  |
| `assay.url` |  |
| `assay.version` |  |
| **AI Agents & Workflow** | |
| `assay.ai-agents` |  |
| `assay.github` |  |
| `assay.gitlab` |  |
| `assay.workflow` |  |

<!-- END STDLIB TABLE --><!-- END STDLIB TABLE -->