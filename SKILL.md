# Assay Skill — LLM Agent Guide

Assay is a single ~9 MB static binary that runs Lua scripts in Kubernetes. It replaces 50-250 MB
Python/Node/kubectl containers in K8s Jobs. One binary, two modes: run a `.lua` script directly, or
run a `.yaml` check config with retry/backoff/structured output.

The image is `ghcr.io/developerinlondon/assay:latest` (~6 MB compressed). Install locally with
`cargo install assay-lua` or download from GitHub Releases.

## Quick Start

```bash
# Run a Lua script
assay script.lua

# Run YAML check orchestration
assay checks.yaml

# Test Lua inline (great for quick experiments)
assay exec -e 'log.info("hello from assay")'

# Discover modules by keyword
assay context "vault"

# List all available modules
assay modules
```

## CLI Commands

| Command                     | What it does                                  |
| --------------------------- | --------------------------------------------- |
| `assay script.lua`          | Auto-detect and run Lua script                |
| `assay checks.yaml`         | Auto-detect and run YAML check config         |
| `assay run script.lua`      | Explicit run (same as auto-detect)            |
| `assay exec -e 'lua code'`  | Evaluate Lua inline                           |
| `assay exec script.lua`     | Run Lua file via exec subcommand              |
| `assay context "<keyword>"` | Find modules matching keyword, shows quickref |
| `assay modules`             | List all 40 modules (23 stdlib + 17 builtins) |

## Discovering Modules

When you need to interact with a service, use `assay context` to find the right module:

```
1. Run: assay context "<what you need>"
2. Read the output — it shows matching modules and their methods
3. Use require("assay.<module>") in your script
4. Call client methods shown in the quickref
```

Example:

```bash
$ assay context "grafana"
# Assay Module Context

## Matching Modules

### assay.grafana
Grafana monitoring and dashboards. Health, datasources, annotations, alerts, folders.
Methods:
  c:health() -> {database, version, commit} | Check Grafana health
  c:datasources() -> [{id, name, type, url}] | List all datasources
  ...
```

The output is prompt-ready Markdown. Paste it into your context or read it to know exactly which
methods exist and what they return.

## Writing Lua Scripts

All stdlib modules follow the same three-step pattern:

```lua
-- 1. Require the module
local grafana = require("assay.grafana")

-- 2. Create a client
local c = grafana.client("http://grafana:3000", { api_key = "glsa_..." })

-- 3. Call methods
local h = c:health()
assert.eq(h.database, "ok", "Grafana database unhealthy")
log.info("Grafana version: " .. h.version)
```

Auth options vary by service:

```lua
-- Token auth
local c = vault.client(url, { token = "hvs...." })

-- API key auth
local c = grafana.client(url, { api_key = "glsa_..." })

-- Username/password
local c = grafana.client(url, { username = "admin", password = "secret" })
```

## Built-in Functions (no require needed)

These are always available in every `.lua` script.

### HTTP

| Function                       | Description                                    |
| ------------------------------ | ---------------------------------------------- |
| `http.get(url, opts?)`         | GET request, returns `{status, body, headers}` |
| `http.post(url, body, opts?)`  | POST request (auto-JSON if body is table)      |
| `http.put(url, body, opts?)`   | PUT request                                    |
| `http.patch(url, body, opts?)` | PATCH request                                  |
| `http.delete(url, opts?)`      | DELETE request                                 |
| `http.serve(port, routes)`     | Start HTTP server (blocking)                   |

Options: `{ headers = { ["X-Key"] = "value" } }`

### Serialization

| Function             | Description                     |
| -------------------- | ------------------------------- |
| `json.parse(str)`    | Parse JSON string to Lua table  |
| `json.encode(tbl)`   | Encode Lua table to JSON string |
| `yaml.parse(str)`    | Parse YAML string to Lua table  |
| `yaml.encode(tbl)`   | Encode Lua table to YAML string |
| `toml.parse(str)`    | Parse TOML string to Lua table  |
| `toml.encode(tbl)`   | Encode Lua table to TOML string |
| `base64.encode(str)` | Base64 encode                   |
| `base64.decode(str)` | Base64 decode                   |

### Filesystem

| Function            | Description          |
| ------------------- | -------------------- |
| `fs.read(path)`     | Read file to string  |
| `fs.write(path, s)` | Write string to file |

### Cryptography

| Function                             | Description                                     |
| ------------------------------------ | ----------------------------------------------- |
| `crypto.jwt_sign(claims, key, alg)`  | Sign JWT — alg: HS256, RS256/384/512, ES256/384 |
| `crypto.hash(str, alg)`              | Hash string (sha256, sha384, sha512, md5)       |
| `crypto.hmac(key, data, alg?, raw?)` | HMAC (sha256 default, raw=true for binary)      |
| `crypto.random(len)`                 | Secure random hex string of length `len`        |

### Regex

| Function                         | Description              |
| -------------------------------- | ------------------------ |
| `regex.match(pattern, str)`      | Test if pattern matches  |
| `regex.find(pattern, str)`       | Find first match         |
| `regex.find_all(pattern, str)`   | Find all matches (array) |
| `regex.replace(pattern, str, r)` | Replace matches          |

### Database

| Function                         | Description                                   |
| -------------------------------- | --------------------------------------------- |
| `db.connect(url)`                | Connect (Postgres, MySQL, SQLite)             |
| `db.query(conn, sql, params?)`   | Execute query, return rows as array of tables |
| `db.execute(conn, sql, params?)` | Execute statement, return affected row count  |
| `db.close(conn)`                 | Close connection                              |

URLs: `postgres://user:pass@host:5432/db`, `mysql://...`, `sqlite:///path/to/file.db`

### WebSocket and Templates

| Function                          | Description                   |
| --------------------------------- | ----------------------------- |
| `ws.connect(url)`                 | Connect to WebSocket server   |
| `ws.send(conn, msg)`              | Send message                  |
| `ws.recv(conn)`                   | Receive message (blocking)    |
| `ws.close(conn)`                  | Close connection              |
| `template.render(path, vars)`     | Render Jinja2 template file   |
| `template.render_string(tmpl, v)` | Render Jinja2 template string |

### Async

| Function                       | Description                        |
| ------------------------------ | ---------------------------------- |
| `async.spawn(fn)`              | Spawn async task, returns handle   |
| `async.spawn_interval(fn, ms)` | Spawn recurring task every `ms` ms |
| `handle:await()`               | Wait for task completion           |
| `handle:cancel()`              | Cancel recurring task              |

### Assertions

| Function                          | Description              |
| --------------------------------- | ------------------------ |
| `assert.eq(a, b, msg?)`           | Assert equal             |
| `assert.gt(a, b, msg?)`           | Assert greater than      |
| `assert.lt(a, b, msg?)`           | Assert less than         |
| `assert.contains(str, sub, msg?)` | Assert substring present |
| `assert.not_nil(val, msg?)`       | Assert not nil           |
| `assert.matches(str, pat, msg?)`  | Assert regex match       |

### Logging and Utilities

| Function         | Description              |
| ---------------- | ------------------------ |
| `log.info(msg)`  | Info log                 |
| `log.warn(msg)`  | Warning log              |
| `log.error(msg)` | Error log                |
| `env.get(key)`   | Get environment variable |
| `sleep(secs)`    | Sleep for N seconds      |
| `time()`         | Unix timestamp (integer) |

## Stdlib Modules Quick Reference

All 23 modules follow `require("assay.<name>")` then `M.client(url, opts)`.

| Module               | Description                                                         |
| -------------------- | ------------------------------------------------------------------- |
| `assay.prometheus`   | PromQL queries, alerts, targets, rules, label values, series        |
| `assay.alertmanager` | Manage alerts, silences, receivers, config                          |
| `assay.loki`         | Push logs, query with LogQL, labels, series, tail                   |
| `assay.grafana`      | Health, dashboards, datasources, annotations, alert rules, folders  |
| `assay.k8s`          | 30+ resource types, CRDs, readiness checks, pod logs, rollouts      |
| `assay.argocd`       | Apps, sync, health, projects, repositories, clusters                |
| `assay.kargo`        | Stages, freight, promotions, warehouses, pipeline status            |
| `assay.flux`         | GitRepositories, Kustomizations, HelmReleases, notifications        |
| `assay.traefik`      | Routers, services, middlewares, entrypoints, TLS status             |
| `assay.vault`        | KV secrets, policies, auth, transit, PKI, token management          |
| `assay.openbao`      | Alias for vault (OpenBao API-compatible)                            |
| `assay.certmanager`  | Certificates, issuers, ACME orders and challenges                   |
| `assay.eso`          | ExternalSecrets, SecretStores, ClusterSecretStores sync status      |
| `assay.dex`          | OIDC discovery, JWKS, health, configuration validation              |
| `assay.crossplane`   | Providers, XRDs, compositions, managed resources                    |
| `assay.velero`       | Backups, restores, schedules, storage locations                     |
| `assay.temporal`     | Workflows, task queues, schedules, signals                          |
| `assay.harbor`       | Projects, repositories, artifacts, vulnerability scanning           |
| `assay.healthcheck`  | HTTP checks, JSON path, body matching, latency, multi-check         |
| `assay.s3`           | S3-compatible storage (AWS, R2, MinIO) with Sig V4 auth             |
| `assay.postgres`     | Postgres helpers: users, databases, grants, Vault integration       |
| `assay.zitadel`      | OIDC identity management with JWT machine auth                      |
| `assay.unleash`      | Feature flags: projects, environments, features, strategies, tokens |

## Common Patterns

### HTTP Health Check

```lua
#!/usr/bin/assay
local resp = http.get("http://grafana.monitoring:80/api/health")
assert.eq(resp.status, 200, "Grafana not responding")

local data = json.parse(resp.body)
assert.eq(data.database, "ok", "Grafana database unhealthy")
log.info("Grafana healthy: version=" .. data.version)
```

### JWT Auth and API Call

```lua
#!/usr/bin/assay
-- Read RSA private key from mounted secret
local key = fs.read("/secrets/jwt-key.pem")

local token = crypto.jwt_sign({
  iss = "assay-client",
  sub = "admin@example.com",
  exp = time() + 3600
}, key, "RS256")

local resp = http.get("https://api.example.com/users", {
  headers = { Authorization = "Bearer " .. token }
})

assert.eq(resp.status, 200, "API call failed")
local users = json.parse(resp.body)
log.info("Found " .. #users .. " users")
```

### Vault Secret Retrieval

```lua
#!/usr/bin/assay
local vault = require("assay.vault")

local token = env.get("VAULT_TOKEN")
local c = vault.client("http://vault:8200", { token = token })

-- Read KV v2 secret
local secret = c:kv_get("secret", "myapp/config")
assert.not_nil(secret, "Secret not found")

log.info("db_password: " .. secret.data.db_password)
```

### Kubernetes Pod Readiness Check

```lua
#!/usr/bin/assay
local k8s = require("assay.k8s")

local c = k8s.client("https://kubernetes.default.svc", {
  token = fs.read("/var/run/secrets/kubernetes.io/serviceaccount/token"),
  ca_cert = fs.read("/var/run/secrets/kubernetes.io/serviceaccount/ca.crt"),
})

-- Wait for deployment to be ready
local deploy = c:deployment("my-namespace", "my-app")
assert.not_nil(deploy, "Deployment not found")
assert.eq(deploy.status.readyReplicas, deploy.spec.replicas, "Not all replicas ready")
log.info("Deployment ready: " .. deploy.metadata.name)
```

### Prometheus Metric Query

```lua
#!/usr/bin/assay
local prom = require("assay.prometheus")

local c = prom.client("http://prometheus.monitoring:9090")

-- Check targets are up
local targets = c:targets()
local up_count = 0
for _, t in ipairs(targets.activeTargets) do
  if t.health == "up" then up_count = up_count + 1 end
end
assert.gt(up_count, 0, "No Prometheus targets are up")

-- Query a metric
log.info("Active targets: " .. up_count .. ", up query: " .. tostring(c:query("up")))
```

## Error Handling

Errors from stdlib methods follow the format: `"<module>: <METHOD> <path> HTTP <status>: <body>"`

Use `pcall` to catch errors without crashing the script:

```lua
local vault = require("assay.vault")

local ok, err = pcall(function()
  local c = vault.client("http://vault:8200", { token = env.get("VAULT_TOKEN") })
  return c:kv_get("secret", "myapp/config")
end)

if not ok then
  log.error("Vault read failed: " .. tostring(err))
  -- handle gracefully or re-raise
  error(err)
end
```

For 404 responses, stdlib modules return `nil` rather than raising an error:

```lua
local secret = c:kv_get("secret", "maybe/exists")
if secret == nil then
  log.warn("Secret not found, using defaults")
else
  log.info("Secret found")
end
```

## YAML Check Mode

For structured orchestration with retry, backoff, and JSON output:

```yaml
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

Check types: `http`, `prometheus`, `script`. Exit code 0 = all pass, 1 = any fail.

## Tips for LLM Agents

**Finding the right module**: Run `assay context "<service name>"` before writing any script. The
output shows exact method signatures and return types. Don't guess.

**Testing snippets**: Use `assay exec -e 'log.info(json.encode({a=1}))'` to test individual
expressions before putting them in a full script.

**In-cluster auth**: K8s service account tokens are at
`/var/run/secrets/kubernetes.io/serviceaccount/token`. Read them with `fs.read()`.

**Environment variables**: Pass secrets via env vars, read with `env.get("MY_SECRET")`. Never
hardcode credentials in scripts.

**Shebang scripts**: Add `#!/usr/bin/assay` as the first line and `chmod +x script.lua` to run
scripts directly without the `assay` prefix.

**Module not found**: All 23 stdlib modules are embedded in the binary. If `require("assay.foo")`
fails, run `assay modules` to see the exact module names.

**Lua 5.5 specifics**: Assay uses Lua 5.5 (not LuaJIT). Integer division is `//`, bitwise ops use
`&`, `|`, `~`, `<<`, `>>`. The `#` operator on tables counts only the sequence part.

**Debugging**: Add `log.info(json.encode(some_table))` to inspect table contents. The `json.encode`
builtin handles nested tables.
