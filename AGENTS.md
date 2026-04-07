# AGENTS.md

## Skills & Rules

Key coding practices for this project:

- **autonomous-workflow** — Proposal-first development, decision authority, commit hygiene
- **code-quality** — Warnings-as-errors, no underscore prefixes, test coverage, type safety

## Library hygiene: no application-domain leakage

**Assay is a general-purpose library. It must have zero knowledge of any
specific application that uses it.** When writing or modifying any code,
test, comment, doc, changelog entry, commit message, or PR description in
this repo, never reference:

- Specific consumer applications by name (e.g. "Command Center",
  "hydra-login", or any internal product)
- Specific deployments, environments, or company-specific URLs
  (e.g. `*.dev.simons.disw.siemens.com`, internal cluster names)
- Company- or project-specific role names, namespaces, client IDs, or
  resource names that only make sense in one consumer's context
- The user's organisation, team, or internal project naming conventions

Use generic placeholder names instead:

- Client IDs: `example-app`, `demo-client`, `app-1`
- Hostnames: `example.com`, `app.example.com`, `hydra.example.com`
- Role objects: `app:role-a`, `namespace1:role-a`, `app:admin`
- Project IDs: `demo-project`, `project-1`
- Workflow names: `MyWorkflow`, `my-queue`

When motivating a new feature in a CHANGELOG entry, commit message, or PR
description, describe **the OIDC/Kubernetes/HTTP scenario it enables**, not
**the specific consumer that asked for it**. The library should read the
same to a stranger who has never heard of any of assay's consumers as it
does to someone who works on one of them every day.

This applies to all files in the repo: `stdlib/`, `src/`, `tests/`, `*.md`,
`*.html`, `CHANGELOG.md`, and any commit/PR text. The only legitimate
exception is the copyright holder's name in `LICENSE`/`NOTICE`/`CLA.md`.

## What is Assay

General-purpose enhanced Lua runtime. Single ~9 MB static binary with batteries included: HTTP
client/server, JSON/YAML/TOML, crypto, database, WebSocket, filesystem, shell execution, process
management, async, and 33 embedded stdlib modules for infrastructure services (Kubernetes,
Prometheus, Vault, ArgoCD, etc.) and AI agent integrations (OpenClaw, GitHub, Gmail, Google
Calendar).

Use cases:

- **Standalone scripting** — system automation, CI/CD tasks, file processing
- **Embedded runtime** — other Rust services embed assay as a library (`pub mod lua`)
- **Kubernetes Jobs** — replaces 50–250 MB Python/Node/kubectl containers (~9 MB image)
- **Infrastructure automation** — GitOps hooks, health checks, service configuration

- **Repo**: [github.com/developerinlondon/assay](https://github.com/developerinlondon/assay)
- **Image**: `ghcr.io/developerinlondon/assay:latest` (~9 MB compressed)
- **Crate**: [crates.io/crates/assay-lua](https://crates.io/crates/assay-lua)
- **Stack**: Rust (2024 edition), Tokio, Lua 5.5 (mlua), reqwest, clap, axum

## Two Modes

```bash
assay script.lua     # Lua mode — run script with all builtins
assay checks.yaml    # YAML mode — structured checks with retry/backoff/parallel
```

### Example: Kubernetes Job

```yaml
apiVersion: batch/v1
kind: Job
metadata:
  name: configure-service
  annotations:
    argocd.argoproj.io/hook: PostSync
spec:
  template:
    spec:
      containers:
        - name: configure
          image: ghcr.io/developerinlondon/assay:latest
          command: ["assay", "/scripts/configure.lua"]
          volumeMounts:
            - name: scripts
              mountPath: /scripts
          env:
            - name: SERVICE_URL
              value: "http://my-service:8080"
            - name: API_TOKEN
              valueFrom:
                secretKeyRef:
                  name: my-service-credentials
                  key: admin_api_token
      volumes:
        - name: scripts
          configMap:
            name: postsync-scripts
      restartPolicy: Never
```

## Builtins

Available as globals in all `.lua` scripts — no `require` needed:

| Category       | Functions                                                                                                                                                                                                                                                           |
| -------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| HTTP           | `http.get(url, opts?)`, `http.post(url, body, opts?)`, `http.put(url, body, opts?)`, `http.patch(url, body, opts?)`, `http.delete(url, opts?)`, `http.serve(port, routes)` — `http.serve` response handlers accept array header values to emit the same header name multiple times (e.g., multiple `Set-Cookie`) |
| JSON/YAML/TOML | `json.parse(str)`, `json.encode(tbl)`, `yaml.parse(str)`, `yaml.encode(tbl)`, `toml.parse(str)`, `toml.encode(tbl)`                                                                                                                                                 |
| Filesystem     | `fs.read(path)`, `fs.write(path, str)`, `fs.remove(path)`, `fs.list(path)`, `fs.stat(path)`, `fs.mkdir(path)`, `fs.exists(path)`, `fs.copy(src, dst)`, `fs.rename(src, dst)`, `fs.glob(pattern)`, `fs.tempdir()`, `fs.chmod(path, mode)`, `fs.readdir(path, opts?)` |
| Crypto         | `crypto.jwt_sign(claims, key, alg, opts?)`, `crypto.hash(str, alg)`, `crypto.hmac(key, data, alg?, raw?)`, `crypto.random(len)`                                                                                                                                     |
| Base64         | `base64.encode(str)`, `base64.decode(str)`                                                                                                                                                                                                                          |
| Regex          | `regex.match(pat, str)`, `regex.find(pat, str)`, `regex.find_all(pat, str)`, `regex.replace(pat, str, repl)`                                                                                                                                                        |
| Database       | `db.connect(url)`, `db.query(conn, sql, params?)`, `db.execute(conn, sql, params?)`, `db.close(conn)`                                                                                                                                                               |
| WebSocket      | `ws.connect(url)`, `ws.send(conn, msg)`, `ws.recv(conn)`, `ws.close(conn)`                                                                                                                                                                                          |
| Templates      | `template.render(path, vars)`, `template.render_string(tmpl, vars)`                                                                                                                                                                                                 |
| Async          | `async.spawn(fn)`, `async.spawn_interval(fn, ms)`, `handle:await()`, `handle:cancel()`                                                                                                                                                                              |
| Assert         | `assert.eq(a, b, msg?)`, `assert.ne(a, b, msg?)`, `assert.gt(a, b, msg?)`, `assert.lt(a, b, msg?)`, `assert.contains(str, sub, msg?)`, `assert.not_nil(val, msg?)`, `assert.matches(str, pat, msg?)`                                                                |
| Logging        | `log.info(msg)`, `log.warn(msg)`, `log.error(msg)`                                                                                                                                                                                                                  |
| Utilities      | `env.get(key)`, `env.set(key, value)`, `env.list()`, `sleep(secs)`, `time()`                                                                                                                                                                                        |
| Shell          | `shell.exec(cmd, opts?)` — execute commands with timeout, working dir, env                                                                                                                                                                                          |
| Process        | `process.list()`, `process.is_running(name)`, `process.kill(pid, signal?)`, `process.wait_idle(names, timeout, interval)`                                                                                                                                           |
| Disk           | `disk.usage(path)` — returns `{total, used, available, percent}`, `disk.sweep(dir, age_secs)`, `disk.dir_size(path)`                                                                                                                                                |
| OS             | `os.hostname()`, `os.arch()`, `os.platform()`                                                                                                                                                                                                                       |
| Temporal (gRPC) | `temporal.connect(opts)`, `temporal.start(opts)` — native gRPC workflow client (requires `temporal` feature) |

HTTP responses: `{status, body, headers}`. Options: `{headers = {["X-Key"] = "val"}}`.

### `http.serve` Response Shapes

Route handlers return a table. Three shapes are supported:

```lua
-- Body response (default Content-Type: text/plain)
return { status = 200, body = "hello" }

-- JSON response (default Content-Type: application/json)
return { status = 200, json = { ok = true } }

-- SSE streaming response (default Content-Type: text/event-stream)
return {
  status = 200,
  sse = function(send)
    send({ data = "connected" })
    sleep(1)  -- async builtins work inside SSE handlers
    send({ event = "update", data = json.encode({ count = 1 }), id = "1" })
    -- stream closes when function returns
  end
}
```

Custom headers override defaults: `headers = { ["content-type"] = "text/html" }`.

Header values can be either a string or an array of strings. Array values emit the same header
name multiple times — required for `Set-Cookie` with multiple cookies and useful for `Link`,
`Vary`, `Cache-Control`, etc.:

```lua
return {
  status = 200,
  headers = {
    ["Set-Cookie"] = {
      "session=abc; Path=/; HttpOnly",
      "csrf=xyz; Path=/",
    },
  },
  body = "ok",
}
```

SSE `send()` accepts: `event` (string), `data` (string), `id` (string), `retry` (integer). `event`
and `id` must not contain newlines. `data` handles multi-line automatically.

## Stdlib Modules

33 embedded Lua modules loaded via `require("assay.<name>")`:

| Module                | Description                                                                       |
| --------------------- | --------------------------------------------------------------------------------- |
| `assay.prometheus`    | Query metrics, alerts, targets, rules, label values, series                       |
| `assay.alertmanager`  | Manage alerts, silences, receivers, config                                        |
| `assay.loki`          | Push logs, query, labels, series                                                  |
| `assay.grafana`       | Health, dashboards, datasources, annotations                                      |
| `assay.k8s`           | 30+ resource types, CRDs, readiness checks                                        |
| `assay.argocd`        | Apps, sync, health, projects, repositories                                        |
| `assay.kargo`         | Stages, freight, promotions, verification                                         |
| `assay.flux`          | GitRepositories, Kustomizations, HelmReleases                                     |
| `assay.traefik`       | Routers, services, middlewares, entrypoints                                       |
| `assay.vault`         | KV secrets, policies, auth, transit, PKI                                          |
| `assay.openbao`       | Alias for vault (API-compatible)                                                  |
| `assay.certmanager`   | Certificates, issuers, ACME challenges                                            |
| `assay.eso`           | ExternalSecrets, SecretStores, ClusterSecretStores                                |
| `assay.dex`           | OIDC discovery, JWKS, health                                                      |
| `assay.zitadel`       | OIDC identity management with JWT machine auth                                    |
| `assay.kratos`        | Ory Kratos identity — login/registration/recovery/settings flows, identities, sessions, schemas |
| `assay.hydra`         | Ory Hydra OAuth2/OIDC — clients, authorize URLs, tokens, login/consent, introspection, JWKs |
| `assay.keto`          | Ory Keto ReBAC — relation tuples, permission checks, role/group membership, expand |
| `assay.ory`           | Convenience wrapper re-exporting kratos/hydra/keto with `ory.connect(opts)`       |
| `assay.crossplane`    | Providers, XRDs, compositions, managed resources                                  |
| `assay.velero`        | Backups, restores, schedules, storage locations                                   |
| `assay.temporal`      | Workflows, task queues, schedules, signals + native gRPC client (temporal feature) |
| `assay.harbor`        | Projects, repositories, artifacts, vulnerability scanning                         |
| `assay.healthcheck`   | HTTP checks, JSON path, body matching, latency, multi-check                       |
| `assay.s3`            | S3-compatible storage (AWS, R2, MinIO) with Sig V4                                |
| `assay.postgres`      | Postgres-specific helpers                                                         |
| `assay.unleash`       | Feature flags: projects, environments, features, strategies, API tokens           |
| `assay.openclaw`      | OpenClaw AI agent platform — invoke tools, state, diff, approve, LLM tasks        |
| `assay.github`        | GitHub REST API — PRs, issues, actions, repos, GraphQL                            |
| `assay.gmail`         | Gmail REST API with OAuth2 — search, read, reply, send, labels                    |
| `assay.gcal`          | Google Calendar REST API with OAuth2 — events CRUD, calendar list                 |
| `assay.oauth2`        | Google OAuth2 token management — file-based credentials, auto-refresh, persistence |
| `assay.email_triage`  | Email classification — deterministic rules + optional LLM-assisted triage via OpenClaw |

### Client Pattern

Every stdlib module follows the same structure:

```lua
local grafana = require("assay.grafana")
local c = grafana.client("http://grafana:3000", { api_key = "glsa_..." })
local h = c:health()
assert.eq(h.database, "ok")
```

1. `require("assay.<name>")` returns module table `M`
2. `M.client(url, opts?)` creates a client with auth config
3. Client methods use `c:method()` (colon = implicit self)
4. Errors raised via `error()` — use `pcall()` to catch

Auth varies by service: `{ token = "..." }`, `{ api_key = "..." }`,
`{ username = "...", password = "..." }`.

## Tool Mode (OpenClaw Integration)

Assay v0.6.0 adds tool mode for integration with OpenClaw AI agents:

```bash
assay run --mode tool script.lua                    # Run as agent tool, structured JSON output
assay resume --token <token> --approve yes|no       # Resume paused approval gate
```

Tool mode produces structured JSON output suitable for agent consumption. When a script hits an
approval gate (via `openclaw.approve()`), execution pauses and returns a resume token. The agent or
human can then approve or reject via `assay resume`.

```
+-------------------------------------------------------+
| OpenClaw Agent                                        |
|   |                                                   |
|   +--> assay run --mode tool deploy.lua               |
|   |      |                                            |
|   |      +--> approval gate -> pauses, returns token  |
|   |                                                   |
|   +--> human reviews                                  |
|   |                                                   |
|   +--> assay resume --token <t> --approve yes         |
|          |                                            |
|          +--> script resumes from gate                |
+-------------------------------------------------------+
```

### OpenClaw Extension

The `@developerinlondon/assay-openclaw-extension` package (GitHub Packages) registers Assay as an
OpenClaw agent tool:

```bash
# One-time: configure npm to use GitHub Packages for @developerinlondon scope
echo "@developerinlondon:registry=https://npm.pkg.github.com" >> ~/.npmrc

# Install the extension
openclaw plugins install @developerinlondon/assay-openclaw-extension
```

Configuration in OpenClaw plugin config:

| Key              | Default        | Description                              |
| ---------------- | -------------- | ---------------------------------------- |
| `binaryPath`     | PATH lookup    | Explicit path to the `assay` binary      |
| `timeout`        | `20`           | Execution timeout in seconds             |
| `maxOutputSize`  | `524288`       | Maximum stdout collected from Assay      |
| `scriptsDir`     | workspace root | Root directory for Lua scripts           |

See [openclaw-extension/README.md](openclaw-extension/README.md) for full details.

## Adding a New Stdlib Module

No Rust changes needed. Modules are auto-discovered via `include_dir!("$CARGO_MANIFEST_DIR/stdlib")`
in `src/lua/mod.rs`.

### 1. Create `stdlib/<name>.lua`

Follow the client pattern. Reference `grafana.lua` (simple, 110 lines) or `vault.lua`
(comprehensive, 330 lines):

```lua
local M = {}

function M.client(url, opts)
  opts = opts or {}
  local c = {
    url = url:gsub("/+$", ""),
    token = opts.token,
  }

  local function headers(self)
    local h = { ["Content-Type"] = "application/json" }
    if self.token then h["Authorization"] = "Bearer " .. self.token end
    return h
  end

  local function api_get(self, path_str)
    local resp = http.get(self.url .. path_str, { headers = headers(self) })
    if resp.status ~= 200 then
      error("<name>: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_post(self, path_str, payload)
    local resp = http.post(self.url .. path_str, payload, { headers = headers(self) })
    if resp.status ~= 200 and resp.status ~= 201 then
      error("<name>: POST " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  function c:health()
    return api_get(self, "/api/health")
  end

  return c
end

return M
```

Conventions:

- `api_get/api_post/api_put/api_delete` are local helpers, not exported
- Error format: `"<module>: <METHOD> <path> HTTP <status>: <body>"`
- Strip trailing slashes: `url:gsub("/+$", "")`
- All HTTP uses builtins (`http.get`, `json.parse`) — no external requires
- 404 = nil pattern: check `resp.status == 404`, return `nil`
- Idempotent helpers go on `M` (module level), not on the client

### 2. Create `tests/stdlib_<name>.rs`

Wiremock-based tests. One test per client method:

```rust
mod common;

use common::run_lua;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_require_<name>() {
    let script = r#"
        local mod = require("assay.<name>")
        assert.not_nil(mod)
        assert.not_nil(mod.client)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_<name>_health() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "OK"
        })))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local mod = require("assay.<name>")
        local c = mod.client("{}", {{ token = "test-token" }})
        local h = c:health()
        assert.eq(h.status, "OK")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
```

Note: Lua tables in `format!` strings need doubled braces: `{{ key = "value" }}`.

### 3. Update `README.md`

Add the module to the stdlib table.

### 4. Verify

```bash
cargo check && cargo clippy -- -D warnings && cargo test
```

## Directory Structure

```
assay/
├── Cargo.toml
├── Cargo.lock
├── AGENTS.md                 # This file
├── Dockerfile                # Multi-stage: rust builder -> scratch
├── src/
│   ├── main.rs               # CLI entry point (clap)
│   ├── lib.rs                # Library root (pub mod lua)
│   ├── config.rs             # YAML config parser (checks.yaml)
│   ├── runner.rs             # Orchestrator: retries, backoff, timeout
│   ├── output.rs             # Structured JSON results + exit code
│   ├── checks/
│   │   ├── mod.rs            # Check dispatcher
│   │   ├── http.rs           # HTTP check type (YAML mode)
│   │   ├── prometheus.rs     # Prometheus check type (YAML mode)
│   │   └── script.rs         # Lua script check type
│   └── lua/
│       ├── mod.rs            # VM setup, sandbox, stdlib loader (include_dir!)
│       ├── async_bridge.rs   # Async Lua execution, shebang stripping
│       └── builtins/
│           ├── mod.rs        # register_all() — wires builtins into Lua globals
│           ├── http.rs       # http.{get,post,put,patch,delete,serve} + SSE streaming
│           ├── json.rs       # json.{parse,encode}
│           ├── serialization.rs  # yaml + toml parse/encode
│           ├── core.rs       # env, sleep, time, fs, base64, regex, log, async
│           ├── assert.rs     # assert.{eq,ne,gt,lt,contains,not_nil,matches}
│           ├── crypto.rs     # crypto.{jwt_sign,hash,hmac,random}
│           ├── db.rs         # db.{connect,query,execute,close}
│           ├── ws.rs         # ws.{connect,send,recv,close}
│           ├── template.rs   # template.{render,render_string}
│           ├── disk.rs       # disk.{usage} + Lua helpers: disk.sweep, disk.dir_size
│           ├── os_info.rs    # os.{hostname,arch,platform}
│           └── temporal.rs   # temporal.{connect,start} + client methods (gRPC, optional)
├── stdlib/                   # Embedded Lua modules (auto-discovered)
│   ├── vault.lua             # Comprehensive reference (330 lines)
│   ├── grafana.lua           # Simple reference (110 lines)
│   └── ... (22 modules total)
├── tests/
│   ├── common/mod.rs         # Test helpers: run_lua(), create_vm(), eval_lua()
│   ├── stdlib_vault.rs       # One test file per stdlib module
│   └── ...
└── examples/                 # Example scripts and check configs
```

## Design Decisions (FINAL)

| Decision         | Choice   | Reason                                                                   |
| ---------------- | -------- | ------------------------------------------------------------------------ |
| Language runtime | Lua 5.5  | ArgoCD compatible, 30yr ecosystem, native int64, perf irrelevant for I/O |
| Not Luau         | Rejected | Lua 5.1 base, Roblox ecosystem, no int64                                 |
| Not Rhai         | Rejected | 6x slower, no async, no coroutines                                       |
| Not Wasmtime     | Rejected | Requires compile step, bad for script iteration                          |

## Release Process

After merging a feature PR, always create a version bump PR before moving on:

1. **Bump version** in `Cargo.toml` (triggers `Cargo.lock` update on next build)
2. **Update CHANGELOG.md** — add new version section at the top with date and changes
3. **Update AGENTS.md** — if new builtins/functions were added, update the tables
4. **Update all docs** — README.md, SKILL.md, site/modules.html, site/llms.txt, site/llms-full.txt
5. **Run checks**: `cargo clippy --tests -- -D warnings && cargo test`
6. **Create a new branch** (e.g., `chore/bump-0.5.6`), commit, push, open PR
7. **After merge**: tag the release (`git tag v0.5.6 && git push origin v0.5.6`)

Files to update per release:

- `Cargo.toml` — version field
- `CHANGELOG.md` — new version entry
- `AGENTS.md` — if API surface changed
- `README.md` — if API surface changed
- `SKILL.md` — if API surface changed
- `site/modules.html` — if API surface changed
- `site/llms.txt` — if API surface changed
- `site/llms-full.txt` — if API surface changed
- `openclaw-extension/package.json` — version field (auto-synced from git tag by CI, but keep
  in sync manually for local development)

The tag push triggers `.github/workflows/release.yml` which publishes:
- GitHub Release (binaries + checksums)
- crates.io (`assay-lua` crate)
- Docker image (`ghcr.io/developerinlondon/assay`)
- GitHub Packages npm (`@developerinlondon/assay-openclaw-extension`)

## Commands

```bash
cargo check                        # Type check
cargo clippy -- -D warnings        # Lint (warnings = errors)
cargo test                         # Run all tests
cargo build --release              # Release build (~9 MB)
```
