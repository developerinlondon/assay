# Assay

**One static binary that replaces Temporal + Kratos + Hydra + Keto.** Plus a full Lua 5.5 runtime
with 66 modules for Kubernetes, monitoring, secrets, and AI agents.

[![CI](https://github.com/developerinlondon/assay/actions/workflows/ci.yml/badge.svg)](https://github.com/developerinlondon/assay/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/assay-lua.svg)](https://crates.io/crates/assay-lua)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)

## What is Assay?

Two binaries, one project. `FROM scratch`-shippable, PG18 + SQLite first-class. Sizes today: `assay`
~12 MB, `assay-engine` ~19 MB (the engine grew with the auth + IdP work in v0.2.0).

- **`assay`** — Lua 5.5 runtime with 63 stdlib modules (Kubernetes, Prometheus, Vault, GitHub,
  Gmail, OpenClaw, Tailscale, …). Drop-in replacement for 50-250 MB Python/Node/kubectl scripting
  containers.
- **`assay-engine`** — durable **workflow engine** (Temporal-replacement: deterministic-replay
  activities, signals, timers, child workflows, schedules, search attributes) **+ full IdP**
  (Kratos + Hydra + Keto replacement: OIDC client + provider, passkey, JWT/JWKS rotation, biscuit
  capability tokens, Argon2 password, Zanzibar ReBAC, session management, admin HTTP API, dashboard
  panes for everything). Default builds include runtime-gated S3 history archival.

```bash
# Lua runtime
assay script.lua     # Run Lua with all builtins
assay checks.yaml    # Structured checks with retry/backoff/JSON output
assay exec -e 'log.info("hello")'   # Inline evaluation
assay context "grafana"              # LLM-ready module docs
assay modules                        # List all 65 modules
assay modules --json                 # Same list as JSON, with full per-module metadata

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
| `assay` runtime               | Python / Node + kubectl   | 12 MB, 5 ms cold start, 63 stdlib modules              |

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

## Read-only mode

For semi-trusted script contexts (agent-generated scripts, review pipelines, dry-run diagnostics),
the runtime can execute scripts with every mutating builtin disabled. Activate with the global
`--readonly` flag or `ASSAY_READONLY=1` (or `true`):

```bash
assay run --readonly script.lua        # also: exec, YAML check mode, tool mode
ASSAY_READONLY=1 assay script.lua      # env activation, same effect
```

Read paths work unchanged: `http.get` (including `http.client(...)` wrappers), `fs.read` /
`fs.list` / `fs.stat`, `env.get`, `db.query`, `systemd`/`apt` status and list helpers, and all
pure builtins (`json`, `crypto`, `regex`, ...). Mutating builtins stay registered but raise a
clear error instead of executing:

```
readonly: http.post blocked (write operations are disabled in read-only mode)
```

Blocked surfaces: HTTP write verbs + `http.serve` + `http.download`, `ws.connect`, all of
`shell.*` / `process.*` / `machinectl.*`, `fs` write ops, `env.set`, `db.execute`, `oci`
mutators, `systemd` unit/machine lifecycle actions, `apt` mutators, `tar.create` /
`tar.extract` / `compress.untar`, `io.popen`, and `io.open` write modes. `assay modules` notes
when the mode is active, and tool-mode envelopes carry `"readonly": true`. For nil-ing out
additional globals entirely, combine with `ASSAY_BLOCK_GLOBALS`.

## Capability policy

Read-only and approval mode decide whether a *mutating* operation runs. A policy file decides what
is *reachable at all* — which modules a script may `require`, which environment keys it may read,
and which HTTP hosts, methods, and paths it may call. The two compose, and a policy applies in
every mode. Point `ASSAY_POLICY_FILE` at a YAML file:

```yaml
version: 1
modules:
  allow: [assay.openstack, assay.json]
env:
  allow: [OS_PROJECT_NAME]
http:
  max_response_bytes: 262144
  redact: [password, token]
  rules:
    - hosts: ["*.identity.example.com"]
      methods: [GET]
      paths: ["/v3/*"]
    - hosts: ["*.identity.example.com"]
      methods: [POST]
      paths: ["/v3/auth/tokens"]
      classify: read
```

`classify: read` marks a target that authenticates with a POST — an OpenStack token issue, an STS
presign — as the read it actually is, so it proceeds under `--readonly` instead of being refused
for its verb. With no policy loaded nothing changes.

A policy can also declare credentials, so a script authenticates without being able to read the
secret:

```yaml
credentials:
  inventory-ro:
    username: ASSAY_INVENTORY_USER
    password: ASSAY_INVENTORY_PASSWORD
```

```lua
local c = credential.get("inventory-ro")   -- opaque placeholders, not secrets
require("assay.openstack").client(url, { username = c.username, password = c.password })
```

The real values are substituted into the request body and headers by the HTTP layer, after the
policy has allowed the target — so printing or encoding a handle yields the placeholder, and
modules taking `username`/`password` need no changes. See [`docs/policy.md`](docs/policy.md) for
the residual-risk note.

## Approval mode

Where read-only mode hard-blocks every write, approval mode enforces a per-operation human (or
supervisor) decision on the same catalog of mutating builtins. It is aimed at supervised remediation
contexts where each write must be individually authorized. Activate with the global
`--approval-mode` flag or `ASSAY_APPROVAL=1` (approval mode wins if `--readonly` is also set):

```bash
assay run --mode tool --approval-mode remediate.lua   # also: run, exec, YAML check mode
```

When a script reaches a mutating operation, the run suspends and raises the existing tool-mode
approval flow — `status:"needs_approval"` with a resume token — carrying a descriptor of the pending
operation:

```json
{
  "status": "needs_approval",
  "requiresApproval": {
    "op": "http.post",
    "summary": "https://api.example.com/deploy",
    "index": 0,
    "resumeToken": "…"
  }
}
```

The supervisor inspects the descriptor and decides:

```bash
assay resume --token <token> --approve yes   # permit this one operation, re-run, suspend at the next
assay resume --token <token> --approve no    # fail it with "approval: <op> denied"
```

Each `yes` re-runs the script from the top: previously-approved operations execute and the run
re-suspends at the next unapproved one, so grants are single-shot and per-operation. Read paths
(`http.get`, `fs.read`, `env.get`, `db.query`, status/list helpers) run freely without prompting.
Each grant is bound to the exact call it was issued for: a SHA-256 digest over the operation, its
URL, and its arguments, reported in the approval descriptor as `digest` alongside the header
*names* in play (never their values). A replay that reaches the same index with a different target
or a different body is refused with `approval: ... changed since approval` rather than executing
what nobody approved. A grant carrying no digest — resume state written by an older version — is
refused too, so the check fails closed.

Because approvals are matched by the sequence index of mutating operations, a read that changes
control flow between operations across re-runs can shift indices — the same replay limitation
workflow engines have; suitable for supervised single-writer scripts. The digest turns a shifted
index into a hard failure instead of a misapplied grant.

## HTTP API server

`mcp-serve` speaks stdio, which suits a client that spawns the runtime as a child process. A host
that wants the runtime in a *separate trust domain* — its own process, its own credentials, reached
over the network — can serve the same gated execution over HTTP:

```bash
ASSAY_API_TOKENS="$(openssl rand -hex 32)" \
ASSAY_POLICY_FILE=/etc/assay/policy.yaml \
  assay api-serve --bind 0.0.0.0:8080
```

`POST /v1/run` and `POST /v1/resume` behind a bearer token, plus an unauthenticated `GET /healthz`.
Both return the tool-mode envelope. `unrestricted` is refused unless the server opts in, exactly as
over MCP, and the server **refuses to start with no tokens configured** rather than quietly serving
an ungated runtime. Pair it with a policy so the transport is not the only control — see
[`docs/api-server.md`](docs/api-server.md).

## MCP server

`assay mcp-serve` runs a Model Context Protocol server over stdio so AI coding agents (Claude Code,
Cursor, Windsurf, Cline, and any MCP client) can drive the runtime directly. It speaks JSON-RPC 2.0
with newline-delimited messages per the MCP stdio transport. Point the client at the binary:

```json
{
  "mcpServers": {
    "assay": { "command": "assay", "args": ["mcp-serve"] }
  }
}
```

Rather than one tool per module — which would balloon the advertised schema as modules are added —
the server exposes exactly **two tools** and lets Lua compose the modules:

- **`assay_run`** — run a Lua script (all embedded modules and builtins available) and return the
  tool-mode JSON envelope. Every run is **gated**: `mode` accepts only `readonly` (default) or
  `approval` — unrestricted execution is not offered, so a caller can never opt out of the gate.
  Optional `timeout_secs` and `args`. Approval gates suspend and return a `requiresApproval` resume
  token, resumable with `assay resume`.
- **`assay_context`** — search the embedded modules and return prompt-ready Markdown docs (method
  signatures, env vars), the same output as `assay context --no-builtins`. The builtins reference is
  omitted by default because the calling agent's harness already carries it; pass
  `include_builtins: true` to append it.

A script that errors — including a write blocked by read-only mode — comes back as an MCP result
with `isError: true`; `needs_approval` is not an error. The server implements `initialize`,
`tools/list`, `tools/call`, and `ping`, and shuts down cleanly on EOF.

## Claude Code plugin

Install assay as a Claude Code plugin — the `assay_run` + `assay_context` tools plus a usage skill, wrapping the MCP server:

```bash
claude plugin marketplace add developerinlondon/assay
claude plugin install assay
```

The plugin (`plugin/`) declares the MCP server as `assay mcp-serve`, so the `assay` binary must be on `PATH` (install via `cargo install assay-lua` or a release binary). Every run is read-only or approval-gated; unrestricted execution is never exposed.

## Auth + IdP quick-start

Once `assay-engine` is running with the auth module enabled, every IdP capability is reachable over
HTTP and from Lua via the `assay.auth` stdlib module:

```bash
# engine.toml — minimum viable v0.2.0 with auth on (0.3.1 adds env-var
# substitution for `${VAR}` / `${VAR:-default}` in any string field).
cat > engine.toml <<'TOML'
auto_enable_modules = ["auth"]

[server]
bind_addr = "0.0.0.0:3000"
public_url = "${ENGINE_PUBLIC_URL:-https://engine.example.com}"

[backend]
type = "postgres"
url = "${DATABASE_URL}"

[auth]
public_url = "${AUTH_PUBLIC_URL:-https://auth.example.com}"
admin_api_keys = ["${ADMIN_API_KEY}"]

[auth.recovery]
enabled = true

[auth.recovery.smtp]
host = "${SMTP_HOST}"
port = 587
username = "${SMTP_USERNAME}"
password = "${SMTP_PASSWORD}"
from = "Example Auth <noreply@example.com>"
starttls = true
TOML

# Inject the secrets via the environment — never bake them into the file.
export DATABASE_URL='postgres://postgres:postgres@localhost/assay'
export ADMIN_API_KEY='sk_admin_replace_me'
# Also inject SMTP_HOST, SMTP_USERNAME, and SMTP_PASSWORD when recovery is enabled.
assay-engine serve --config engine.toml
#   /auth/console                          → admin SPA
#   /.well-known/openid-configuration      → OIDC discovery (Hydra-equivalent)
#   /auth/login, /auth/recovery             → password auth and recovery pages
#   /auth/passkey/*                         → passkey flows
#   /auth/admin/auth/*                     → admin HTTP API (api-key gated)
```

When `auth.public_url` is omitted, auth continues to use `server.public_url`. Set it only when the
same engine is intentionally exposed through a separate browser-facing auth hostname; an explicit
`auth.issuer` still takes precedence over both defaults.

Same `engine.toml` works under Kubernetes — keep the TOML in a ConfigMap, project the secrets in via
env from a Secret:

```yaml
spec:
  containers:
    - name: engine
      image: ghcr.io/developerinlondon/assay-engine:0.3.1
      args: ["serve", "--config", "/etc/assay/engine.toml"]
      env:
        - name: DATABASE_URL
          valueFrom: { secretKeyRef: { name: engine-db, key: url } }
        - name: ADMIN_API_KEY
          valueFrom: { secretKeyRef: { name: engine-admin, key: api-key } }
      volumeMounts:
        - { name: cfg, mountPath: /etc/assay, readOnly: true }
  volumes:
    - name: cfg
      configMap: { name: engine-toml }
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
| `ws.connect(url, opts?)`          | WebSocket client (`send`, `send_binary`, `recv`, `protocol`, `close`); `opts` = `{subprotocols, headers, insecure}`                                                            |

### Serialization

| Function                                    | Description |
| ------------------------------------------- | ----------- |
| `json.parse(str)` / `json.encode(tbl)`      | JSON        |
| `yaml.parse(str)` / `yaml.parse_all(str)` / `yaml.encode(tbl)` | YAML stream/documents |
| `toml.parse(str)` / `toml.encode(tbl)`      | TOML        |
| `base64.encode(str)` / `base64.decode(str)` | Base64      |

### Filesystem & System

| Function                                                             | Description                                      |
| -------------------------------------------------------------------- | ------------------------------------------------ |
| `fs.read(path)` / `fs.write(path, s)`                                | Read/write files                                 |
| `fs.exists(path)` / `fs.mkdir(path)` / `fs.glob(pattern)`            | File operations                                  |
| `shell.exec(cmd, opts?)`                                             | Execute shell commands                           |
| `process.list()` / `process.kill(pid)`                               | Process management                               |
| `disk.usage(path)` / `disk.sweep(dir, age)`                          | Disk info and cleanup                            |
| `os.hostname()` / `os.arch()` / `os.platform()`                      | OS information                                   |
| `linux.cpu_stat()` / `meminfo()` / `loadavg()` / `proc_stat(pid)`    | `/proc` + `/sys/...` readers (Linux-only)        |
| `cgroup.cpu_stat(path)` / `memory(path)` / `pids(path)`              | cgroup v2 unified-hierarchy readers (Linux-only) |
| `systemd.list_units(filter?)` / `list_machines()` / `journal({...})` | D-Bus client + journal reader (Linux-only)       |
| `env.get(key)` / `env.set(key, val)`                                 | Environment variables                            |
| `sleep(secs)` / `time()`                                             | Pause execution, Unix timestamp                  |

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
| `assay.sonarqube` |  |
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
| `assay.rauthy` |  |
| `assay.vault` |  |
| `assay.zitadel` |  |
| **Infrastructure** | |
| `assay.apt_index` | Debian/Ubuntu apt Packages-index reader (require("assay.apt")) |
| `assay.crossplane` |  |
| `assay.fs_snapshot` | btrfs / zfs subvolume snapshot wrapper for crash-consistent backup capture (require("assay.fs_snapshot"), v0.15.7+) |
| `assay.harbor` |  |
| `assay.infoblox` |  |
| `assay.openstack` |  |
| `assay.pkg` | Package manager framework — catalog, templates, targets, plan/reconcile (v0.15.5+) |
| `assay.rustic` | rustic backup CLI wrapper — snapshots, backup, restore, init, check, forget (require("assay.rustic"), v0.15.7+) |
| `assay.servicenow` |  |
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
| `assay.clickup` | ClickUp REST API — tasks, lists, spaces, goals, custom fields, time tracking, and Docs |
| `assay.excalidash` | ExcaliDash REST API — Excalidraw drawings, collections, version history, and sharing |
| `assay.github` |  |
| `assay.gitlab` |  |
| `assay.huly` | Huly transactor REST API — document queries, transactions, fulltext search, and tracker helpers |
| `assay.n8n` | n8n public REST API — workflows, executions, credentials, projects, variables, plus idempotent reconcilers (v0.18.0+) |
| `assay.plane` | Plane REST API — projects, work items, cycles, modules, states, labels, comments, and links |
| `assay.workflow` |  |
| **AI & Agents** | |
| `assay.neutron` |  |
| **Cloud & AWS** | |
| `assay.aws-ec2` |  |
| `assay.aws-ecr` |  |
| `assay.aws-s3` |  |
| `assay.aws-sigv4` |  |
| **Container & Registry** | |
| `assay.oci` |  |
| **Filesystem & Archives** | |
| `assay.tar` |  |
| **Linux & systemd** | |
| `assay.cron` |  |
| `assay.system` |  |
| **Stdlib** | |
| `assay.shell` |  |
<!-- END STDLIB TABLE -->