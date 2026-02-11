# Assay

Lightweight Lua runtime for Kubernetes. Verification, scripting, and web services.

[![CI](https://github.com/developerinlondon/assay/actions/workflows/ci.yml/badge.svg)](https://github.com/developerinlondon/assay/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/assay.svg)](https://crates.io/crates/assay)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

## What is Assay?

Assay is a single ~9 MB binary that replaces 50-250 MB Python/Node/kubectl containers in Kubernetes
Jobs. It provides a full-featured Lua runtime with built-in HTTP client/server, database access,
WebSocket, JWT signing, templates, and 19 embedded Kubernetes-native libraries.

One binary, auto-detected behavior:

```bash
assay checks.yaml    # YAML → check orchestration (retry, backoff, structured output)
assay script.lua     # Lua → run it (all builtins, script decides what to do)
```

Scripts that call `http.serve()` become web services. Scripts that call `http.get()` and exit are
jobs. Same binary, same builtins.

## Why Assay?

Container image size comparison (compressed pull):

```
+------------------------------------------------------------------+
| Docker image size comparison (compressed pull)                   |
|                                                                  |
| Assay Full       ## 6 MB                                         |
| Python alpine    ########## 17 MB                                |
| bitnami/kubectl  #################### 35 MB                      |
| Python slim      ########################## 43 MB                |
| Node.js alpine   ################################## 57 MB        |
| alpine/k8s       ######################################## 60 MB  |
| Deno             ############################################ 75 |
| Node.js slim     ############################################### |
| Bun              ############################################### |
| postman/newman   ############################################### |
+------------------------------------------------------------------+
```

| Runtime         | Compressed |   On-disk | vs Assay | Sandbox | K8s-native |
| --------------- | ---------: | --------: | :------: | :-----: | :--------: |
| **Assay**       |   **6 MB** | **13 MB** |  **1x**  | **Yes** |  **Yes**   |
| Python alpine   |      17 MB |     50 MB |    3x    |   No    |     No     |
| bitnami/kubectl |      35 MB |     90 MB |    6x    |   No    |  Partial   |
| Python slim     |      43 MB |    130 MB |    9x    |   No    |     No     |
| Node.js alpine  |      57 MB |    180 MB |   12x    |   No    |     No     |
| alpine/k8s      |      60 MB |    150 MB |   10x    |   No    |  Partial   |

## Installation

### Pre-built Binary (fastest)

Download from [GitHub Releases](https://github.com/developerinlondon/assay/releases/latest):

```bash
# Linux (x86_64, static — runs on any distro, no dependencies)
curl -L -o assay https://github.com/developerinlondon/assay/releases/latest/download/assay-linux-x86_64
chmod +x assay
sudo mv assay /usr/local/bin/

# macOS (Apple Silicon)
curl -L -o assay https://github.com/developerinlondon/assay/releases/latest/download/assay-darwin-aarch64
chmod +x assay
sudo mv assay /usr/local/bin/
```

### Docker

```bash
docker pull ghcr.io/developerinlondon/assay:latest
docker run --rm ghcr.io/developerinlondon/assay:latest --version
```

### Cargo

```bash
cargo install assay
```

### From Source

```bash
git clone https://github.com/developerinlondon/assay.git
cd assay
cargo build --release
./target/release/assay --version
```

## Usage

### Two Modes

Assay auto-detects behavior by file extension:

#### 1. YAML Check Mode (Orchestration)

Run structured verification checks with retry, backoff, and JSON output:

```yaml
# checks.yaml
timeout: 120s
retries: 3
backoff: 5s
parallel: false

checks:
  - name: grafana-healthy
    type: http
    url: http://grafana.monitoring:80/api/health
    expect:
      status: 200
      json: ".database == \"ok\""

  - name: prometheus-targets
    type: prometheus
    url: http://prometheus.monitoring:9090
    query: "count(up)"
    expect:
      min: 1

  - name: custom-check
    type: script
    file: verify.lua
```

```bash
assay checks.yaml
```

#### 2. Lua Script Mode (Direct Execution)

Run Lua scripts with all builtins available:

```lua
#!/usr/bin/assay
-- HTTP health check with JWT auth
local token = crypto.jwt_sign({
  iss = "assay",
  sub = "health-check",
  exp = time() + 300
}, env.get("JWT_SECRET"), "HS256")

local resp = http.get("https://api.example.com/health", {
  headers = { Authorization = "Bearer " .. token }
})

assert.eq(resp.status, 200, "API health check failed")
log.info("API healthy: " .. resp.body)
```

```bash
chmod +x script.lua
./script.lua
```

### Shebang Support

Assay supports shebang for executable Lua scripts:

```lua
#!/usr/bin/assay
log.info("Hello from Assay!")
```

```bash
chmod +x hello.lua
./hello.lua
```

## Built-in API Reference

All builtins are available to `.lua` scripts. YAML check mode uses a sandboxed subset (http, json,
yaml, assert, log, env, sleep, time, base64).

### HTTP Client

| Function                                 | Description                                |
| ---------------------------------------- | ------------------------------------------ |
| `http.get(url, opts?)`                   | GET request, returns `{status, body, ...}` |
| `http.post(url, body, opts?)`            | POST request (auto-JSON if table)          |
| `http.put(url, body, opts?)`             | PUT request                                |
| `http.patch(url, body, opts?)`           | PATCH request                              |
| `http.delete(url, opts?)`                | DELETE request                             |
| `http.serve(port, routes)`               | Start HTTP server (blocking)               |
| `opts.headers = {["X-Key"] = "value"}`   | Custom headers                             |
| `routes = {GET = {["/path"] = handler}}` | Route table for server                     |

### Serialization

| Function             | Description                     |
| -------------------- | ------------------------------- |
| `json.parse(str)`    | Parse JSON string to Lua table  |
| `json.encode(table)` | Encode Lua table to JSON string |
| `yaml.parse(str)`    | Parse YAML string to Lua table  |
| `yaml.encode(table)` | Encode Lua table to YAML string |
| `toml.parse(str)`    | Parse TOML string to Lua table  |
| `toml.encode(table)` | Encode Lua table to TOML string |
| `base64.encode(str)` | Base64 encode                   |
| `base64.decode(str)` | Base64 decode                   |

### Filesystem

| Function            | Description          |
| ------------------- | -------------------- |
| `fs.read(path)`     | Read file to string  |
| `fs.write(path, s)` | Write string to file |

### Cryptography

| Function                            | Description                                |
| ----------------------------------- | ------------------------------------------ |
| `crypto.jwt_sign(claims, key, alg)` | Sign JWT (HS256/384/512, RS256/384/512)    |
| `crypto.hash(str, alg)`             | Hash string (sha256, sha384, sha512, etc.) |
| `crypto.random(len)`                | Secure random string (hex)                 |

### Regular Expressions

| Function                         | Description             |
| -------------------------------- | ----------------------- |
| `regex.match(pattern, str)`      | Test if pattern matches |
| `regex.find(pattern, str)`       | Find first match        |
| `regex.find_all(pattern, str)`   | Find all matches        |
| `regex.replace(pattern, str, r)` | Replace matches         |

### Database (SQL)

| Function                         | Description                                 |
| -------------------------------- | ------------------------------------------- |
| `db.connect(url)`                | Connect to database (Postgres/MySQL/SQLite) |
| `db.query(conn, sql, params?)`   | Execute query, return rows                  |
| `db.execute(conn, sql, params?)` | Execute statement, return affected count    |
| `db.close(conn)`                 | Close connection                            |

Supported URLs:

- `postgres://user:pass@host:5432/dbname`
- `mysql://user:pass@host:3306/dbname`
- `sqlite:///path/to/file.db`

### WebSocket

| Function             | Description                 |
| -------------------- | --------------------------- |
| `ws.connect(url)`    | Connect to WebSocket server |
| `ws.send(conn, msg)` | Send message                |
| `ws.recv(conn)`      | Receive message (blocking)  |
| `ws.close(conn)`     | Close connection            |

### Templates (Jinja2-compatible)

| Function                          | Description            |
| --------------------------------- | ---------------------- |
| `template.render(path, vars)`     | Render template file   |
| `template.render_string(tmpl, v)` | Render template string |

### Async

| Function                       | Description                          |
| ------------------------------ | ------------------------------------ |
| `async.spawn(fn)`              | Spawn async task, returns handle     |
| `async.spawn_interval(fn, ms)` | Spawn recurring task, returns handle |
| `handle:await()`               | Wait for task completion             |
| `handle:cancel()`              | Cancel recurring task                |

### Assertions

| Function                          | Description         |
| --------------------------------- | ------------------- |
| `assert.eq(a, b, msg?)`           | Assert equal        |
| `assert.gt(a, b, msg?)`           | Assert greater than |
| `assert.lt(a, b, msg?)`           | Assert less than    |
| `assert.contains(str, sub, msg?)` | Assert substring    |
| `assert.not_nil(val, msg?)`       | Assert not nil      |
| `assert.matches(str, pat, msg?)`  | Assert regex match  |

### Logging

| Function         | Description |
| ---------------- | ----------- |
| `log.info(msg)`  | Info log    |
| `log.warn(msg)`  | Warning log |
| `log.error(msg)` | Error log   |

### Utilities

| Function       | Description              |
| -------------- | ------------------------ |
| `env.get(key)` | Get environment variable |
| `sleep(secs)`  | Sleep for seconds        |
| `time()`       | Unix timestamp (seconds) |

## Stdlib Modules

Assay embeds 19 Lua modules for Kubernetes-native operations. Use `require("assay.<module>")`:

| Module               | Description                                                 |
| -------------------- | ----------------------------------------------------------- |
| `assay.prometheus`   | Query metrics, alerts, targets, rules, label values, series |
| `assay.alertmanager` | Manage alerts, silences, receivers, config                  |
| `assay.loki`         | Push logs, query, labels, series                            |
| `assay.grafana`      | Health checks, dashboards, datasources                      |
| `assay.k8s`          | 30+ resource types, CRDs, readiness checks                  |
| `assay.argocd`       | Apps, sync, health, projects, repositories                  |
| `assay.kargo`        | Stages, freight, promotions, verification                   |
| `assay.flux`         | GitRepositories, Kustomizations, HelmReleases               |
| `assay.traefik`      | Routers, services, middlewares, entrypoints                 |
| `assay.vault`        | KV secrets, policies, auth, transit, PKI                    |
| `assay.openbao`      | Alias for vault (OpenBao API-compatible)                    |
| `assay.certmanager`  | Certificates, issuers, ACME challenges                      |
| `assay.eso`          | ExternalSecrets, SecretStores, ClusterSecretStores          |
| `assay.dex`          | OIDC discovery, JWKS, health                                |
| `assay.crossplane`   | Providers, XRDs, compositions, managed resources            |
| `assay.velero`       | Backups, restores, schedules, storage locations             |
| `assay.temporal`     | Workflows, task queues, schedules                           |
| `assay.harbor`       | Projects, repositories, artifacts, vulnerability scanning   |
| `assay.healthcheck`  | HTTP checks, JSON path, body matching, latency, multi-check |

Example:

```lua
local prom = require("assay.prometheus")
local result = prom.query("http://prometheus:9090", "up")
log.info("Targets up: " .. tostring(result))
```

## Examples

### HTTP Health Check

```lua
#!/usr/bin/assay
local resp = http.get("http://grafana.monitoring:80/api/health")
assert.eq(resp.status, 200, "Grafana not responding")

local data = json.parse(resp.body)
assert.eq(data.database, "ok", "Grafana database unhealthy")
log.info("Grafana healthy: version=" .. data.version)
```

### JWT Authentication to API

```lua
#!/usr/bin/assay
-- Read RSA private key from file
local key = fs.read("/secrets/jwt-key.pem")

-- Sign JWT with RS256
local token = crypto.jwt_sign({
  iss = "assay-client",
  sub = "admin@example.com",
  exp = time() + 3600
}, key, "RS256")

-- Call API with JWT
local resp = http.get("https://api.example.com/users", {
  headers = { Authorization = "Bearer " .. token }
})

assert.eq(resp.status, 200, "API call failed")
local users = json.parse(resp.body)
log.info("Found " .. #users .. " users")
```

### Database Query

```lua
#!/usr/bin/assay
local pg = db.connect("postgres://user:pass@postgres:5432/mydb")

-- Parameterized query (safe from SQL injection)
local rows = db.query(pg, "SELECT id, name FROM users WHERE active = $1", {true})

for _, row in ipairs(rows) do
  log.info("User: " .. row.name .. " (ID: " .. row.id .. ")")
end

db.close(pg)
```

### Web Server

```lua
#!/usr/bin/assay
-- Simple API server
http.serve(8080, {
  GET = {
    ["/health"] = function(req)
      return { status = 200, body = "ok" }
    end,
    ["/api/time"] = function(req)
      return {
        status = 200,
        json = { timestamp = time(), zone = "UTC" }
      }
    end
  },
  POST = {
    ["/api/echo"] = function(req)
      local data = json.parse(req.body)
      return { status = 200, json = data }
    end
  }
})
```

### Prometheus Verification

```lua
#!/usr/bin/assay
local prom = require("assay.prometheus")

-- Check Prometheus is up
local targets = prom.targets("http://prometheus.monitoring:9090")
local up_count = 0
for _, target in ipairs(targets.activeTargets) do
  if target.health == "up" then
    up_count = up_count + 1
  end
end

assert.gt(up_count, 0, "No Prometheus targets are up")
log.info("Prometheus targets up: " .. up_count)

-- Query metrics
local result = prom.query("http://prometheus.monitoring:9090", "up")
log.info("Query result: " .. tostring(result))
```

## YAML Check Mode

YAML check mode provides structured orchestration with retry, backoff, and parallel execution:

```yaml
# Global config
timeout: 120s # Max time for all checks
retries: 3 # Retry failed checks
backoff: 5s # Wait between retries
parallel: false # Run checks sequentially (true = parallel)

checks:
  # HTTP check with JSON path assertion
  - name: api-health
    type: http
    url: https://api.example.com/health
    expect:
      status: 200
      json: ".status == \"healthy\""

  # Prometheus query check
  - name: high-cpu
    type: prometheus
    url: http://prometheus:9090
    query: "avg(rate(cpu_usage[5m]))"
    expect:
      max: 0.8 # Alert if CPU > 80%

  # Custom Lua script check
  - name: database-check
    type: script
    file: verify-db.lua
    env:
      DB_URL: postgres://user:pass@postgres:5432/mydb
```

Check types:

- `type: http` — HTTP request with status/body/JSON assertions
- `type: prometheus` — PromQL query with min/max assertions
- `type: script` — Custom Lua script (sandboxed builtins)

Output is structured JSON:

```json
{
  "passed": 2,
  "failed": 1,
  "total": 3,
  "results": [
    {
      "name": "api-health",
      "status": "passed",
      "duration_ms": 45
    },
    {
      "name": "high-cpu",
      "status": "failed",
      "error": "expected max 0.8, got 0.92",
      "duration_ms": 120
    }
  ]
}
```

Exit code: 0 if all checks pass, 1 if any fail.

## Development

### Build

```bash
cargo build --release
```

### Test

```bash
cargo test
```

### Lint

```bash
cargo clippy -- -D warnings
```

### Format

```bash
dprint fmt
```

### Run Examples

Self-contained scripts (no external services needed):

```bash
cargo run -- tests/e2e/check_json.lua
cargo run -- tests/e2e/check_yaml.lua
cargo run -- tests/e2e/check_toml.lua
cargo run -- tests/e2e/check_base64.lua
cargo run -- tests/e2e/check_crypto.lua
cargo run -- tests/e2e/check_regex.lua
cargo run -- tests/e2e/check_fs.lua
cargo run -- tests/e2e/check_template.lua
```

Kubernetes examples (require services running in-cluster):

```bash
cargo run -- examples/checks.yaml
cargo run -- examples/grafana-health.lua
cargo run -- examples/prometheus-scrape.lua
cargo run -- examples/loki-test.lua
```

## Architecture

```
+------------------------------------------------------------------+
| Assay v0.1.0 (~9 MB static MUSL binary, Alpine container)       |
|                                                                  |
| CLI (auto-detected by file extension):                           |
|   assay config.yaml           (.yaml -> check orchestration)     |
|   assay script.lua            (.lua  -> run script)              |
|                                                                  |
| Shebang support:                                                 |
|   #!/usr/bin/assay            (works like #!/usr/bin/python3)    |
|                                                                  |
| Rust Core:                                                       |
|   Config parser (YAML) -> Runner (retry/backoff/timeout)         |
|   -> Structured JSON output -> Exit code (0/1)                   |
|                                                                  |
| Lua Runtime (mlua + Lua 5.5):                                    |
|   - 64 MB memory limit per VM                                    |
|   - Fresh VM per check (YAML mode)                               |
|   - Single VM per script (Lua mode)                              |
|   - Async support via tokio LocalSet                             |
|                                                                  |
| Rust Builtins (all available to .lua scripts):                   |
|   http.{get,post,put,patch,delete,serve}                         |
|   ws.{connect,send,recv,close}                                   |
|   json.{parse,encode}  yaml.{parse,encode}  toml.{parse,encode}  |
|   fs.{read,write}  base64.{encode,decode}                        |
|   crypto.{jwt_sign,hash,random}  regex.{match,find,replace}      |
|   db.{connect,query,execute,close}  (postgres, mysql, sqlite)    |
|   template.{render,render_string}                                |
|   assert.{eq,gt,lt,contains,not_nil,matches}                     |
|   log.{info,warn,error}  env.get  sleep  time                    |
|   async.{spawn,spawn_interval}                                   |
|                                                                  |
| Lua Stdlib (embedded .lua files via include_dir!):               |
|   Monitoring: prometheus, alertmanager, loki, grafana             |
|   K8s/GitOps: k8s, argocd, kargo, flux, traefik                 |
|   Security:   vault, openbao, certmanager, eso, dex              |
|   Infra:      crossplane, velero, temporal, harbor               |
|   Utilities:  healthcheck                                        |
+------------------------------------------------------------------+
```

## Use Cases

- **ArgoCD/Kargo Hooks**: PostSync verification, PreSync validation, health checks
- **Kubernetes Jobs**: Database migrations, API configuration, secret rotation
- **Lightweight Web Services**: Webhook receivers, API proxies, mock servers, dashboards
- **Platform Automation**: Operational tasks, cross-service connectivity checks, report generation
- **Verification**: E2E tests, smoke tests, integration tests

## Why Lua 5.5?

Assay uses Lua 5.5 (released Dec 2025) over LuaJIT for:

- **Global declarations**: Catches accidental globals (reduces bugs)
- **Named vararg tables**: Cleaner function signatures
- **Incremental major GC**: Smoother latency for long-running servers
- **Native int64**: Better for timestamps, IDs
- **MUSL static linking**: No assembler issues

Our scripts are I/O bound (HTTP calls, database queries). LuaJIT's 5-10x CPU speedup provides
negligible benefit (<1% of total job time).

## License

MIT

## Contributing

Contributions welcome! Please open an issue or PR on GitHub.

## Links

- **Repository**: https://github.com/developerinlondon/assay
- **Crates.io**: https://crates.io/crates/assay
- **Docker**: https://github.com/developerinlondon/assay/pkgs/container/assay
- **Issues**: https://github.com/developerinlondon/assay/issues
