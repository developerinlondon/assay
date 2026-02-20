# AGENTS.md

## Skills & Rules

For coding practices, commit hygiene, and workflow rules, install from
[agent-skills](https://github.com/developerinlondon/agent-skills):

```bash
npx skills add developerinlondon/agent-skills
```

Key skills that apply to this project:

- **autonomous-workflow** — Proposal-first development, decision authority, commit hygiene
- **code-quality** — Warnings-as-errors, no underscore prefixes, test coverage, type safety

## What is Assay

Lightweight Lua runtime for Kubernetes. Single ~9 MB static binary that replaces 50–250 MB
Python/Node/kubectl containers in K8s Jobs.

- **Repo**: [github.com/developerinlondon/assay](https://github.com/developerinlondon/assay)
- **Image**: `ghcr.io/developerinlondon/assay:latest` (~6 MB compressed)
- **Crate**: [crates.io/crates/assay-lua](https://crates.io/crates/assay-lua)
- **Stack**: Rust (2024 edition), Tokio, Lua 5.5 (mlua), reqwest, clap, axum

## Two Modes

```bash
assay script.lua     # Lua mode — run script with all builtins
assay checks.yaml    # YAML mode — structured checks with retry/backoff/parallel
```

## Using Assay in Kubernetes

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

## Built-in Globals

Available in all `.lua` scripts — no `require` needed:

| Category | Functions |
|----------|-----------|
| HTTP | `http.get(url, opts?)`, `http.post(url, body, opts?)`, `http.put(url, body, opts?)`, `http.patch(url, body, opts?)`, `http.delete(url, opts?)`, `http.serve(port, routes)` |
| JSON/YAML/TOML | `json.parse(str)`, `json.encode(tbl)`, `yaml.parse(str)`, `yaml.encode(tbl)`, `toml.parse(str)`, `toml.encode(tbl)` |
| Filesystem | `fs.read(path)`, `fs.write(path, str)` |
| Crypto | `crypto.jwt_sign(claims, key, alg, opts?)`, `crypto.hash(str, alg)`, `crypto.hmac(key, data, alg?, raw?)`, `crypto.random(len)` |
| Base64 | `base64.encode(str)`, `base64.decode(str)` |
| Regex | `regex.match(pat, str)`, `regex.find(pat, str)`, `regex.find_all(pat, str)`, `regex.replace(pat, str, repl)` |
| Database | `db.connect(url)`, `db.query(conn, sql, params?)`, `db.execute(conn, sql, params?)`, `db.close(conn)` |
| WebSocket | `ws.connect(url)`, `ws.send(conn, msg)`, `ws.recv(conn)`, `ws.close(conn)` |
| Templates | `template.render(path, vars)`, `template.render_string(tmpl, vars)` |
| Async | `async.spawn(fn)`, `async.spawn_interval(fn, ms)`, `handle:await()`, `handle:cancel()` |
| Assert | `assert.eq(a, b, msg?)`, `assert.gt(a, b, msg?)`, `assert.lt(a, b, msg?)`, `assert.contains(str, sub, msg?)`, `assert.not_nil(val, msg?)`, `assert.matches(str, pat, msg?)` |
| Logging | `log.info(msg)`, `log.warn(msg)`, `log.error(msg)` |
| Utilities | `env.get(key)`, `sleep(secs)`, `time()` |

HTTP responses: `{status, body, headers}`. Options: `{headers = {["X-Key"] = "val"}}`.

## Stdlib Modules

23 embedded Lua modules loaded via `require("assay.<name>")`:

| Module | Description |
|--------|-------------|
| `assay.prometheus` | Query metrics, alerts, targets, rules, label values, series |
| `assay.alertmanager` | Manage alerts, silences, receivers, config |
| `assay.loki` | Push logs, query, labels, series |
| `assay.grafana` | Health, dashboards, datasources, annotations |
| `assay.k8s` | 30+ resource types, CRDs, readiness checks |
| `assay.argocd` | Apps, sync, health, projects, repositories |
| `assay.kargo` | Stages, freight, promotions, verification |
| `assay.flux` | GitRepositories, Kustomizations, HelmReleases |
| `assay.traefik` | Routers, services, middlewares, entrypoints |
| `assay.vault` | KV secrets, policies, auth, transit, PKI |
| `assay.openbao` | Alias for vault (API-compatible) |
| `assay.certmanager` | Certificates, issuers, ACME challenges |
| `assay.eso` | ExternalSecrets, SecretStores, ClusterSecretStores |
| `assay.dex` | OIDC discovery, JWKS, health |
| `assay.crossplane` | Providers, XRDs, compositions, managed resources |
| `assay.velero` | Backups, restores, schedules, storage locations |
| `assay.temporal` | Workflows, task queues, schedules |
| `assay.harbor` | Projects, repositories, artifacts, vulnerability scanning |
| `assay.healthcheck` | HTTP checks, JSON path, body matching, latency, multi-check |
| `assay.s3` | S3-compatible storage (AWS, R2, MinIO) with Sig V4 |
| `assay.postgres` | Postgres-specific helpers |
| `assay.zitadel` | OIDC identity management with JWT machine auth |
| `assay.unleash` | Feature flags: projects, environments, features, strategies, API tokens |

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

Auth varies by service: `{ token = "..." }`, `{ api_key = "..." }`, `{ username = "...", password = "..." }`.

## Adding a New Stdlib Module

No Rust changes needed. Modules are auto-discovered via `include_dir!("$CARGO_MANIFEST_DIR/stdlib")`
in `src/lua/mod.rs`.

### 1. Create `stdlib/<name>.lua`

Follow the client pattern. Reference `grafana.lua` (simple, 110 lines) or `vault.lua` (comprehensive,
330 lines):

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
│           ├── http.rs       # http.{get,post,put,patch,delete,serve}
│           ├── json.rs       # json.{parse,encode}
│           ├── serialization.rs  # yaml + toml parse/encode
│           ├── core.rs       # env, sleep, time, fs, base64, regex, log, async
│           ├── assert.rs     # assert.{eq,gt,lt,contains,not_nil,matches}
│           ├── crypto.rs     # crypto.{jwt_sign,hash,hmac,random}
│           ├── db.rs         # db.{connect,query,execute,close}
│           ├── ws.rs         # ws.{connect,send,recv,close}
│           └── template.rs   # template.{render,render_string}
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

| Decision | Choice | Reason |
|----------|--------|--------|
| Language runtime | Lua 5.5 | ArgoCD compatible, 30yr ecosystem, native int64, perf irrelevant for I/O |
| Not Luau | Rejected | Lua 5.1 base, Roblox ecosystem, no int64 |
| Not Rhai | Rejected | 6x slower, no async, no coroutines |
| Not Wasmtime | Rejected | Requires compile step, bad for script iteration |

## Commands

```bash
cargo check                        # Type check
cargo clippy -- -D warnings        # Lint (warnings = errors)
cargo test                         # Run all tests
cargo build --release              # Release build (~9 MB)
```
