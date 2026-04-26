# Assay

**One static binary that replaces Temporal + Kratos + Hydra + Keto.** Plus a full Lua 5.5 runtime
with 51 modules for Kubernetes, monitoring, secrets, and AI agents.

[![CI](https://github.com/developerinlondon/assay/actions/workflows/ci.yml/badge.svg)](https://github.com/developerinlondon/assay/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/assay-lua.svg)](https://crates.io/crates/assay-lua)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)

## What is Assay?

Two binaries, one project. ~9-11 MB static each, `FROM scratch`-shippable, PG18 + SQLite first-
class.

- **`assay`** — Lua 5.5 runtime with 51 stdlib modules (Kubernetes, Prometheus, Vault, GitHub,
  Gmail, OpenClaw, …). Drop-in replacement for 50-250 MB Python/Node/kubectl scripting containers.
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
assay modules                        # List all 51 modules

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
| `assay` runtime               | Python / Node + kubectl   | 11 MB, 5 ms cold start, 51 stdlib modules              |

## Two binaries, two use cases

| Use case                     | Binary         | Install                                       |
| ---------------------------- | -------------- | --------------------------------------------- |
| Scripting / automation       | `assay`        | `cargo install assay-lua` or download release |
| Workflow + auth + IdP server | `assay-engine` | `cargo install assay-engine` or Docker        |

`assay` runs Lua scripts with the full 51-module stdlib; for workflows/auth it talks to a deployed
`assay-engine` over HTTP. `assay-engine` is a standalone HTTP server with workflow + auth +
dashboard, pluggable across PG18 (default) and SQLite — both backends compiled in, runtime-selected
via config.

See [docs/migration-to-0.2.0.md](./docs/migration-to-0.2.0.md) for the upgrade path from v0.1.x.

## Why Assay?

| Runtime          | Compressed |   On-disk | vs Assay | Cold Start | K8s-native |
| ---------------- | ---------: | --------: | :------: | ---------: | :--------: |
| **assay**        |  **11 MB** | **15 MB** |  **1x**  |   **5 ms** |  **Yes**   |
| **assay-engine** |   **9 MB** | **12 MB** |  **1x**  |   **8 ms** |  **Yes**   |
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

34 embedded Lua modules loaded via `require("assay.<name>")`. All follow the client pattern:
`M.client(url, opts)` then `c:method()`.

| Module                          | Description                                                                                           |
| ------------------------------- | ----------------------------------------------------------------------------------------------------- |
| **Monitoring**                  |                                                                                                       |
| `assay.prometheus`              | PromQL queries, alerts, targets, rules, series                                                        |
| `assay.alertmanager`            | Alerts, silences, receivers                                                                           |
| `assay.loki`                    | Log push, query (LogQL), labels, series                                                               |
| `assay.grafana`                 | Health, dashboards, datasources, annotations                                                          |
| **Kubernetes & GitOps**         |                                                                                                       |
| `assay.k8s`                     | 30+ resource types, CRDs, readiness, pod logs                                                         |
| `assay.argocd`                  | Apps, sync, health, projects, repositories                                                            |
| `assay.kargo`                   | Stages, freight, promotions, pipelines                                                                |
| `assay.flux`                    | GitRepositories, Kustomizations, HelmReleases                                                         |
| `assay.traefik`                 | Routers, services, middlewares                                                                        |
| **Security & Identity**         |                                                                                                       |
| `assay.vault` / `assay.openbao` | KV secrets, transit, PKI, policies                                                                    |
| `assay.certmanager`             | Certificates, issuers, ACME                                                                           |
| `assay.eso`                     | ExternalSecrets, SecretStores                                                                         |
| `assay.dex`                     | OIDC discovery, JWKS                                                                                  |
| `assay.zitadel`                 | OIDC identity, JWT machine auth                                                                       |
| `assay.ory.kratos`              | Ory Kratos — login/registration/recovery flows, identities, sessions                                  |
| `assay.ory.hydra`               | Ory Hydra — OAuth2/OIDC clients, authorize, tokens, login/consent/logout                              |
| `assay.ory.keto`                | Ory Keto — ReBAC relation tuples, permission checks, expand                                           |
| `assay.ory.rbac`                | Capability-based RBAC engine over Keto — define roles + capabilities, query users, manage memberships |
| `assay.ory`                     | Ory stack umbrella — `ory.connect()` builds kratos/hydra/keto in one call; also re-exports `rbac`     |
| **Infrastructure**              |                                                                                                       |
| `assay.crossplane`              | Providers, XRDs, compositions                                                                         |
| `assay.velero`                  | Backups, restores, schedules                                                                          |
| `assay.harbor`                  | Projects, repos, vulnerability scanning                                                               |
| **Data & Storage**              |                                                                                                       |
| `assay.postgres`                | User/database management, grants                                                                      |
| `assay.s3`                      | S3-compatible storage with Sig V4 auth                                                                |
| `assay.unleash`                 | Feature flags, environments, strategies                                                               |
| `assay.healthcheck`             | HTTP checks, JSON path, latency                                                                       |
| **AI Agent**                    |                                                                                                       |
| `assay.openclaw`                | Agent tools, state, diff, approve, LLM tasks                                                          |
| `assay.gitlab`                  | Projects, repos, commits, MRs, pipelines, registry                                                    |
| `assay.github`                  | PRs, issues, actions, repos, GraphQL                                                                  |
| `assay.gmail`                   | Search, read, reply, send (OAuth2)                                                                    |
| `assay.gcal`                    | Calendar events CRUD (OAuth2)                                                                         |
| `assay.oauth2`                  | Google OAuth2 token management                                                                        |
| `assay.email_triage`            | Email classification and triage                                                                       |

## Examples

### Kubernetes Health Check

```lua
#!/usr/bin/assay
local k8s = require("assay.k8s")
local c = k8s.client("https://kubernetes.default.svc", {
  token = fs.read("/var/run/secrets/kubernetes.io/serviceaccount/token"),
})

local deploy = c:deployment("default", "my-app")
assert.eq(deploy.status.readyReplicas, deploy.spec.replicas, "Not all replicas ready")
log.info("Deployment ready: " .. deploy.metadata.name)
```

### Web Server with SSE

```lua
#!/usr/bin/assay
http.serve(8080, {
  GET = {
    ["/health"] = function(req)
      return { status = 200, json = { ok = true } }
    end,
    ["/login"] = function(req)
      -- Array header values emit the same header name multiple times.
      -- Required for setting multiple Set-Cookie values in one response.
      return {
        status = 200,
        headers = {
          ["Set-Cookie"] = {
            "session=abc; Path=/; HttpOnly",
            "csrf=xyz; Path=/",
          },
        },
        json = { ok = true },
      }
    end,
    ["/events"] = function(req)
      return {
        sse = function(send)
          send({ data = "connected" })
          for i = 1, 10 do
            sleep(1)
            send({ event = "update", data = json.encode({ count = i }), id = tostring(i) })
          end
        end
      }
    end
  }
})
```

### YAML Check Mode

```yaml
timeout: 120s
retries: 3
backoff: 5s

checks:
  - name: api-health
    type: http
    url: https://api.example.com/health
    expect:
      status: 200
      json: ".status == \"healthy\""

  - name: prometheus-targets
    type: prometheus
    url: http://prometheus:9090
    query: "count(up)"
    expect:
      min: 1
```

```bash
assay checks.yaml   # Exit 0 if all pass, 1 if any fail
```

## OpenClaw Integration

Assay integrates with [OpenClaw](https://openclaw.dev) as an agent tool with human approval gates:

```bash
assay run --mode tool script.lua              # Structured JSON output for agents
assay resume --token <token> --approve yes    # Resume after human approval
```

Install the extension: `openclaw plugins install @developerinlondon/assay-openclaw-extension`

## Module Discovery

Find the right module before writing code:

```bash
assay context "grafana"   # Returns method signatures for LLM prompts
assay context "vault"     # Exact API docs, no hallucination
assay modules             # List all 51 modules
```

Custom modules: place `.lua` files in `./modules/` (project) or `~/.assay/modules/` (global).

## Managing the workflow engine from the CLI

Once `assay serve` is running, the same binary manages it via REST:

```bash
# Start a workflow, wait for it to complete (exit 0/1/2 for scripts).
assay workflow start --type MyFlow --input '{"x":1}' --id wf-42
assay workflow wait wf-42 --timeout 300

# List + filter + inspect.
assay workflow list --status RUNNING --search-attrs '{"env":"prod"}'
assay workflow describe wf-42
assay workflow events wf-42 --follow        # polls until terminal
assay workflow state wf-42 pipeline_stage   # register_query reader

# Signal / cancel / terminate.
assay workflow signal wf-42 approve '{"by":"alice"}'
assay workflow cancel wf-42
assay workflow terminate wf-42 --reason "wrong input"

# Schedules (full lifecycle without a delete-and-recreate cycle).
assay schedule create nightly --type Report --cron '0 0 2 * * *' \
    --timezone Europe/Berlin --input '{"lookback":24}'
assay schedule patch nightly --cron '0 0 3 * * *'
assay schedule pause nightly
assay schedule resume nightly

# Namespaces, workers, queues.
assay namespace create tenant-acme
assay namespace describe main   # live counts
assay worker list
assay queue stats
```

**Configuration** (in precedence order — flag > env > config file > default):

```bash
assay workflow list --engine-url http://engine:8080 --api-key sk_xxxx

# Or via env (fits Kubernetes Secret → env patterns):
export ASSAY_ENGINE_URL=https://assay.example.com
export ASSAY_API_KEY=sk_xxxx
assay workflow list

# Or via a YAML config file — auto-discovered at
# --config FLAG / $ASSAY_CONFIG_FILE / $XDG_CONFIG_HOME/assay/config.yaml /
# ~/.config/assay/config.yaml / /etc/assay/config.yaml.
cat >/etc/assay/config.yaml <<'YAML'
engine_url: https://assay.example.com
api_key_file: /run/secrets/assay-api-key    # preferred — keeps secrets out of env
namespace: main
output: table
YAML
```

**Output formats.** Default is `table` on a TTY, `json` when stdout is piped. Override per-call:

```bash
assay workflow list --output json | jq '.[].id'
assay workflow list --output jsonl | head -5      # streaming-friendly
assay workflow list --output yaml
```

**JSON input anywhere** — literal, `@file`, or `-` for stdin:

```bash
assay workflow start --type MyFlow --input @request.json
echo '{"k":"v"}' | assay workflow signal wf-1 go -
```

**Shell completion.** Writes a script for bash / zsh / fish / powershell / elvish:

```bash
assay completion bash > /etc/bash_completion.d/assay
assay completion zsh  > "${fpath[1]}/_assay"
assay completion fish > ~/.config/fish/completions/assay.fish
```

**Exit codes:** 0 success · 1 HTTP error / unreachable / not-found · 2 `workflow wait` timeout · 64
usage error (bad JSON).

Prefer the Lua stdlib for automation — `local workflow = require("assay.workflow")` mirrors the same
surface programmatically without spawning a subprocess per call. The CLI is for humans at a terminal
and one-shot shell scripts.

## Development

```bash
cargo build --release -p assay-lua      # Lua runtime binary (~11 MB)
cargo build --release -p assay-engine \
    --features backend-postgres,backend-sqlite,auth   # Workflow + auth server (~9 MB)
cargo clippy --workspace -- -D warnings
cargo test --workspace
dprint fmt                              # Format (Rust, Markdown, YAML, JSON, TOML)
```

The workspace splits across six crates: `assay-domain` (shared types + traits), `assay-workflow`
(Temporal-replacement engine), `assay-auth` (Ory-replacement IdP), `assay-dashboard` (the SPA),
`assay-engine` (composer + binary), and `assay-lua` / `assay` (the Lua runtime). Both
`backend-
postgres` and `backend-sqlite` features are additive — both compile in by default and the
active backend is picked at runtime via `engine.toml`.

## License

Assay is licensed under the [Apache License, Version 2.0](LICENSE). You can use, modify, and
redistribute it freely — including in commercial and proprietary products — as long as you preserve
the copyright notice and the license text.

## Contributing

Pull requests are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for the workflow and
[CLA.md](CLA.md) for the Contributor License Agreement that all contributors are required to sign
(it lets the project owner relicense or incorporate contributions into commercial editions in the
future, while you keep the copyright on your contribution).

## Links

- **Website**: https://assay.rs
- **Crate**: https://crates.io/crates/assay-lua
- **Docker**: `ghcr.io/developerinlondon/assay:latest`
- **Changelog**: https://assay.rs/changelog.html
- **Module Reference**: https://assay.rs/modules.html
- **Comparison**: https://assay.rs/comparison.html
- **Agent Guides**: https://assay.rs/agent-guides.html
- **LLM Context**: https://assay.rs/llms.txt
