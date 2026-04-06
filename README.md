# Assay

Replaces your entire infrastructure scripting toolchain. One 9 MB binary, 46 built-in modules.

[![CI](https://github.com/developerinlondon/assay/actions/workflows/ci.yml/badge.svg)](https://github.com/developerinlondon/assay/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/assay-lua.svg)](https://crates.io/crates/assay-lua)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

## What is Assay?

A single ~9 MB static binary that replaces 50-250 MB Python/Node/kubectl containers in Kubernetes.
Full-featured Lua 5.5 runtime with HTTP client/server, database, WebSocket, JWT, templates, native
Temporal gRPC workflows, and 29 embedded stdlib modules for Kubernetes, monitoring, security, and
AI agent integrations.

```bash
assay script.lua     # Run Lua with all builtins
assay checks.yaml    # Structured checks with retry/backoff/JSON output
assay exec -e 'log.info("hello")'   # Inline evaluation
assay context "grafana"              # LLM-ready module docs
assay modules                        # List all 46+ modules
```

Scripts that call `http.serve()` become web services. Scripts that call `http.get()` and exit are
jobs. Same binary, same builtins.

## Why Assay?

| Runtime         | Compressed |   On-disk | vs Assay | Cold Start | K8s-native |
| --------------- | ---------: | --------: | :------: | ---------: | :--------: |
| **Assay**       |   **9 MB** | **15 MB** |  **1x**  |   **5 ms** |  **Yes**   |
| Python alpine   |      17 MB |     50 MB |    2x    |     300 ms |     No     |
| bitnami/kubectl |      35 MB |     90 MB |    4x    |     200 ms |  Partial   |
| Node.js alpine  |      57 MB |    180 MB |    6x    |     500 ms |     No     |
| Deno            |      75 MB |    200 MB |    8x    |      50 ms |     No     |
| Bun             |     115 MB |    250 MB |   13x    |      30 ms |     No     |
| postman/newman  |     128 MB |    350 MB |   14x    |     800 ms |     No     |

## Installation

```bash
# Pre-built binary (Linux x86_64 static)
curl -L -o assay https://github.com/developerinlondon/assay/releases/latest/download/assay-linux-x86_64
chmod +x assay && sudo mv assay /usr/local/bin/

# macOS (Apple Silicon)
curl -L -o assay https://github.com/developerinlondon/assay/releases/latest/download/assay-darwin-aarch64
chmod +x assay && sudo mv assay /usr/local/bin/

# Docker
docker pull ghcr.io/developerinlondon/assay:latest

# Cargo
cargo install assay-lua
```

## Built-in API Reference

All builtins are available globally in `.lua` scripts — no `require` needed.

### HTTP & Networking

| Function | Description |
| --- | --- |
| `http.get(url, opts?)` | GET request, returns `{status, body, headers}` |
| `http.post(url, body, opts?)` | POST (auto-JSON if body is table) |
| `http.put/patch/delete(url, ...)` | PUT, PATCH, DELETE |
| `http.serve(port, routes)` | HTTP server with async handlers + SSE streaming |
| `ws.connect(url)` | WebSocket client (`send`, `recv`, `close`) |

### Serialization

| Function | Description |
| --- | --- |
| `json.parse(str)` / `json.encode(tbl)` | JSON |
| `yaml.parse(str)` / `yaml.encode(tbl)` | YAML |
| `toml.parse(str)` / `toml.encode(tbl)` | TOML |
| `base64.encode(str)` / `base64.decode(str)` | Base64 |

### Filesystem & System

| Function | Description |
| --- | --- |
| `fs.read(path)` / `fs.write(path, s)` | Read/write files |
| `fs.exists(path)` / `fs.mkdir(path)` / `fs.glob(pattern)` | File operations |
| `shell.exec(cmd, opts?)` | Execute shell commands |
| `process.list()` / `process.kill(pid)` | Process management |
| `disk.usage(path)` / `disk.sweep(dir, age)` | Disk info and cleanup |
| `os.hostname()` / `os.arch()` / `os.platform()` | OS information |
| `env.get(key)` / `env.set(key, val)` | Environment variables |
| `sleep(secs)` / `time()` | Pause execution, Unix timestamp |

### Cryptography & Regex

| Function | Description |
| --- | --- |
| `crypto.jwt_sign(claims, key, alg, opts?)` | Sign JWT (HS256, RS256/384/512, ES256/384) |
| `crypto.hash(str, alg)` | SHA-256, SHA-384, SHA-512, SHA3 |
| `crypto.hmac(key, data, alg?, raw?)` | HMAC (all 8 hash algorithms) |
| `crypto.random(len)` | Secure random hex string |
| `regex.match/find/find_all/replace` | Regular expressions |

### Database, Templates & Async

| Function | Description |
| --- | --- |
| `db.connect(url)` | Postgres, MySQL, SQLite |
| `db.query(conn, sql, params?)` | Execute query, return rows |
| `template.render(path, vars)` | Jinja2-compatible templates |
| `async.spawn(fn)` / `async.spawn_interval(secs, fn)` | Async tasks with handles |

### Assertions & Logging

| Function | Description |
| --- | --- |
| `assert.eq/ne/gt/lt/contains/not_nil/matches` | Test assertions |
| `log.info/warn/error(msg)` | Structured logging |

### Temporal Workflow Engine (native gRPC)

| Function | Description |
| --- | --- |
| `temporal.connect({ url, namespace? })` | Connect to Temporal gRPC frontend |
| `temporal.start({ url, ..., workflow_type, workflow_id, input? })` | One-shot: connect + start |
| `client:start_workflow(opts)` | Start a workflow execution |
| `client:signal_workflow(opts)` | Signal a running workflow |
| `client:query_workflow(opts)` | Query workflow state |
| `client:describe_workflow(id)` | Get status, timestamps, history length |
| `client:get_result({ workflow_id })` | Block until workflow completes |
| `client:cancel_workflow(id)` | Graceful cancellation |
| `client:terminate_workflow(id)` | Force terminate |

## Stdlib Modules

29 embedded Lua modules loaded via `require("assay.<name>")`. All follow the client pattern:
`M.client(url, opts)` then `c:method()`.

| Module | Description |
| --- | --- |
| **Monitoring** | |
| `assay.prometheus` | PromQL queries, alerts, targets, rules, series |
| `assay.alertmanager` | Alerts, silences, receivers |
| `assay.loki` | Log push, query (LogQL), labels, series |
| `assay.grafana` | Health, dashboards, datasources, annotations |
| **Kubernetes & GitOps** | |
| `assay.k8s` | 30+ resource types, CRDs, readiness, pod logs |
| `assay.argocd` | Apps, sync, health, projects, repositories |
| `assay.kargo` | Stages, freight, promotions, pipelines |
| `assay.flux` | GitRepositories, Kustomizations, HelmReleases |
| `assay.traefik` | Routers, services, middlewares |
| **Security & Identity** | |
| `assay.vault` / `assay.openbao` | KV secrets, transit, PKI, policies |
| `assay.certmanager` | Certificates, issuers, ACME |
| `assay.eso` | ExternalSecrets, SecretStores |
| `assay.dex` | OIDC discovery, JWKS |
| `assay.zitadel` | OIDC identity, JWT machine auth |
| **Infrastructure** | |
| `assay.crossplane` | Providers, XRDs, compositions |
| `assay.velero` | Backups, restores, schedules |
| `assay.harbor` | Projects, repos, vulnerability scanning |
| `assay.temporal` | Workflows, task queues, schedules (HTTP REST) |
| **Data & Storage** | |
| `assay.postgres` | User/database management, grants |
| `assay.s3` | S3-compatible storage with Sig V4 auth |
| `assay.unleash` | Feature flags, environments, strategies |
| `assay.healthcheck` | HTTP checks, JSON path, latency |
| **AI Agent** | |
| `assay.openclaw` | Agent tools, state, diff, approve, LLM tasks |
| `assay.github` | PRs, issues, actions, repos, GraphQL |
| `assay.gmail` | Search, read, reply, send (OAuth2) |
| `assay.gcal` | Calendar events CRUD (OAuth2) |
| `assay.oauth2` | Google OAuth2 token management |
| `assay.email_triage` | Email classification and triage |

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

### Temporal Workflow

```lua
#!/usr/bin/assay
local client = temporal.connect({
  url = "temporal-frontend:7233",
  namespace = "production",
})

local handle = client:start_workflow({
  task_queue = "promotions",
  workflow_type = "PromoteToEnv",
  workflow_id = "promote-prod-v1.2.0",
  input = { version = "v1.2.0", target = "prod" },
})
log.info("Started: " .. handle.run_id)

local info = client:describe_workflow("promote-prod-v1.2.0")
log.info("Status: " .. info.status)
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
assay modules             # List all 46+ modules
```

Custom modules: place `.lua` files in `./modules/` (project) or `~/.assay/modules/` (global).

## Development

```bash
cargo build --release     # Release build (~9 MB)
cargo clippy -- -D warnings
cargo test
dprint fmt                # Format (Rust, Markdown, YAML, JSON, TOML)
```

## License

MIT

## Links

- **Website**: https://assay.rs
- **Crate**: https://crates.io/crates/assay-lua
- **Docker**: `ghcr.io/developerinlondon/assay:latest`
- **Changelog**: https://assay.rs/changelog.html
- **Module Reference**: https://assay.rs/modules.html
- **Comparison**: https://assay.rs/comparison.html
- **Agent Guides**: https://assay.rs/agent-guides.html
- **LLM Context**: https://assay.rs/llms.txt
