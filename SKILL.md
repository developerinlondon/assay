---
name: assay
description: Infrastructure scripting runtime — 51 modules for Kubernetes, ArgoCD, Vault, Prometheus, HTTP servers, AI agents, databases. Replaces kubectl, Python, Node.js, curl, jq in one 9 MB binary.
metadata:
  author: developerinlondon
  version: "0.6.1"
---

# Assay Skill — LLM Agent Guide

Assay is a single ~9 MB static binary that runs Lua scripts in Kubernetes. It replaces 50-250 MB
Python/Node/kubectl containers in K8s Jobs. One binary, two modes: run a `.lua` script directly, or
run a `.yaml` check config with retry/backoff/structured output.

The image is `ghcr.io/developerinlondon/assay:latest` (~9 MB compressed). Install locally with
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
| `assay modules`             | List all 51 modules (34 stdlib + 17 builtins) |

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

## Builtins (no require needed)

These are always available in every `.lua` script.

### HTTP

| Function                       | Description                                    |
| ------------------------------ | ---------------------------------------------- |
| `http.get(url, opts?)`         | GET request, returns `{status, body, headers}` |
| `http.post(url, body, opts?)`  | POST request (auto-JSON if body is table)      |
| `http.put(url, body, opts?)`   | PUT request                                    |
| `http.patch(url, body, opts?)` | PATCH request                                  |
| `http.delete(url, opts?)`      | DELETE request                                 |
| `http.serve(port, routes)`     | Start HTTP server (async handlers)             |

Options: `{ headers = { ["X-Key"] = "value" } }`

`http.serve` response handlers accept array values for headers to emit the same header name multiple
times — required for `Set-Cookie` with multiple cookies, and useful for `Link`, `Vary`,
`Cache-Control`, etc.:

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

String header values still work as before.

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

| Function                             | Description                                                   |
| ------------------------------------ | ------------------------------------------------------------- |
| `crypto.jwt_sign(claims, key, alg)`  | Sign JWT — alg: HS256, RS256/384/512, ES256/384               |
| `crypto.jwt_decode(token)`           | Decode `{header, claims}` WITHOUT verifying — trusted channel |
| `crypto.hash(str, alg)`              | Hash string (sha256, sha384, sha512, md5)                     |
| `crypto.hmac(key, data, alg?, raw?)` | HMAC (sha256 default, raw=true for binary)                    |
| `crypto.random(len)`                 | Secure random hex string of length `len`                      |

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
| `assert.ne(a, b, msg?)`           | Assert not equal         |
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

### Workflow engine — `assay serve` and `require("assay.workflow")`

Native durable workflow engine built into the `assay` binary (default-on `workflow` feature).
`assay serve` runs the engine; `assay run worker.lua` where the script registers handlers via
`require("assay.workflow")` makes that process a worker. Workflow code runs deterministically and
replays from a persisted event log — worker crashes don't lose work and side effects don't
duplicate.

See `docs/modules/workflow.md` for the full reference.

CLI — `assay <noun> <verb>` with global
`--engine-url`/`--api-key`/`--namespace`/`--output`/`--config` flags (all env-backed:
`ASSAY_ENGINE_URL`, `ASSAY_API_KEY`, `ASSAY_API_KEY_FILE`, `ASSAY_NAMESPACE`, `ASSAY_OUTPUT`,
`ASSAY_CONFIG_FILE`):

| Command                                                         | Description                                                       |
| --------------------------------------------------------------- | ----------------------------------------------------------------- |
| `assay serve [--port N] [--backend ...]`                        | Start the engine (SQLite default; Postgres for multi-instance)    |
| `assay workflow start --type T [--id] [--input] …`              | Start a workflow                                                  |
| `assay workflow list/describe/events/children`                  | Inspect workflows (events supports `--follow` for live streaming) |
| `assay workflow state <id> [<query-name>]`                      | Read the latest `ctx:register_query` snapshot                     |
| `assay workflow signal/cancel/terminate/continue-as-new`        | Mutate workflows                                                  |
| `assay workflow wait <id> [--timeout] [--target]`               | Block for scripts; exit 0/1/2 for COMPLETED / failure / timeout   |
| `assay schedule list/describe/create/patch/pause/resume/delete` | Full cron schedule lifecycle (cron is 6/7-field)                  |
| `assay namespace create/list/describe/delete`                   | Namespace management                                              |
| `assay worker list` / `assay queue stats`                       | Inspect engine state                                              |
| `assay completion <bash                                         | zsh                                                               |

`--input`, `--search-attrs`, and signal payloads accept a literal JSON string, `@file.json`, or `-`
for stdin. Config file auto-discovered at `--config PATH` / `$ASSAY_CONFIG_FILE` /
`$XDG_CONFIG_HOME/assay/config.yaml` / `~/.config/assay/config.yaml` / `/etc/assay/config.yaml`.
Fields: `engine_url`, `api_key`, `api_key_file` (preferred; keeps the secret out of env/argv),
`namespace`, `output`.

Lua client (`require("assay.workflow")`) — two roles in one module: **worker** (register handlers +
block polling) and **management** (inspect/mutate the engine from anywhere, REST parity with the
CLI). Returns parsed JSON on success, nil on a 404 for describe/get_state, raises with HTTP status
on other non-2xx.

| Function                                                                                | Description                                          |
| --------------------------------------------------------------------------------------- | ---------------------------------------------------- |
| `workflow.connect(url, opts?)`                                                          | Verify reachability; opts = `{ token = "Bearer …" }` |
| `workflow.define(name, function(ctx, input) … end)`                                     | Register a workflow handler (runs as a coroutine)    |
| `workflow.activity(name, function(ctx, input) … end)`                                   | Register an activity implementation                  |
| `workflow.listen({queue, namespace?, identity?})`                                                   | Block; poll workflow tasks AND activity tasks. **v0.11.10:** `namespace` scopes the worker (default `"main"`). |
| `workflow.start({workflow_type, workflow_id, namespace?, input?, search_attributes?, task_queue?})` | Start a workflow. **v0.11.10:** `namespace` + `search_attributes` now flow through to the engine.              |
| `workflow.list({namespace?, status?, type?, search_attrs?, limit?, offset?})`           | List + filter workflows                              |
| `workflow.describe(id)` / `workflow.get_events(id)`                                     | Inspect                                              |
| `workflow.get_state(id, name?)`                                                         | Read the latest register_query snapshot (nil on 404) |
| `workflow.list_children(id)`                                                            | List a parent's child workflows                      |
| `workflow.signal(id, name, payload?)`                                                   | Send a signal                                        |
| `workflow.cancel(id)` / `workflow.terminate(id, reason?)`                               | Graceful / hard stop                                 |
| `workflow.continue_as_new(id, input?)`                                                  | Client-side continue-as-new (distinct from `ctx:`)   |
| `workflow.schedules.{create, list, describe, patch, pause, resume, delete}`             | Full schedule management                             |
| `workflow.namespaces.{create, list, describe, stats, delete}`                           | Namespace management                                 |
| `workflow.workers.list(opts?)` / `workflow.queues.stats(opts?)`                         | Engine-state inspection                              |

Workflow handler `ctx`:

| Method                                     | Returns         | Notes                                                                                                        |
| ------------------------------------------ | --------------- | ------------------------------------------------------------------------------------------------------------ |
| `ctx:execute_activity(name, input, opts?)` | activity result | Sync; raises after retries exhausted                                                                         |
| `ctx:execute_parallel(activities)`         | list of results | **v0.11.3**: fan out; handler resumes only when all terminal. Raises if any fail.                            |
| `ctx:sleep(seconds)`                       | nil             | Durable timer; survives worker restart                                                                       |
| `ctx:wait_for_signal(name, opts?)`         | signal payload  | Block until matching signal arrives. **v0.11.9**: `opts.timeout` bounds the wait; returns nil on timeout.    |
| `ctx:start_child_workflow(type, opts)`     | child result    | `opts.workflow_id` required and deterministic                                                                |
| `ctx:side_effect(name, fn)`                | value of fn()   | Run once, cache in event log; use for `crypto.uuid()`, `os.time()`, anything non-deterministic               |
| `ctx:register_query(name, fn)`             | nil             | **v0.11.3**: expose live state via `GET /workflows/{id}/state`. Handler runs on every replay.                |
| `ctx:upsert_search_attributes(patch)`      | nil             | **v0.11.3**: merge into the workflow's indexed metadata; callers filter with `workflow.list({search_attrs})` |
| `ctx:continue_as_new(input)`               | nil (yields)    | **v0.11.3**: close this run, start a fresh one with empty history; same type/namespace/queue                 |

**Dashboard** at `/workflow/` — read-only views in v0.11.2; **v0.11.3** adds tier-1 operator
controls: start-workflow form, per-row signal/cancel/terminate, full schedule CRUD (including
patch/pause/resume/timezone), detail-panel continue-as-new + live `register_query` state, namespace
create/delete, engine version shown in the status bar.

**`GET /api/v1/version`** returns `{ version, build_profile }`. CLI forwards its own
`CARGO_PKG_VERSION` so the field reflects the user-facing binary (e.g. `0.11.3`), not the internal
`assay-workflow` crate version.

**Optional S3 archival** (cargo feature `s3-archival`, default-off). When compiled in and
`ASSAY_ARCHIVE_S3_BUCKET` is set, a background task archives workflows in terminal states older than
`ASSAY_ARCHIVE_RETENTION_DAYS` (default 30) to S3 and stubs the row with `archived_at` +
`archive_uri`. See `docs/modules/workflow.md` for the full list of `ASSAY_ARCHIVE_*` env vars.

**Dashboard whitelabel** (v0.11.10+). Six optional `ASSAY_WHITELABEL_*` env vars rebrand the
embedded `/workflow` dashboard per-deployment — name (`_NAME`), logo image (`_LOGO_URL`), browser
title (`_PAGE_TITLE`), parent-app back-link (`_PARENT_URL` + `_PARENT_NAME`), API Docs link
override / hide (`_API_DOCS_URL`; set to `""` to hide). Every knob defaults to assay's identity;
unset env keeps the standalone experience unchanged. Use when embedding assay inside another
admin UI. Full table in `docs/modules/workflow.md#dashboard-whitelabel`.

## Stdlib Modules Quick Reference

All 35 modules follow `require("assay.<name>")` then `M.client(url, opts)`.

| Module               | Description                                                                                                                                                      |
| -------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `assay.prometheus`   | PromQL queries, alerts, targets, rules, label values, series                                                                                                     |
| `assay.alertmanager` | Manage alerts, silences, receivers, config                                                                                                                       |
| `assay.loki`         | Push logs, query with LogQL, labels, series, tail                                                                                                                |
| `assay.grafana`      | Health, dashboards, datasources, annotations, alert rules, folders                                                                                               |
| `assay.k8s`          | 30+ resource types, CRDs, readiness checks, pod logs, rollouts                                                                                                   |
| `assay.argocd`       | Apps, sync, health, projects, repositories, clusters                                                                                                             |
| `assay.kargo`        | Stages, freight, promotions, warehouses, pipeline status                                                                                                         |
| `assay.flux`         | GitRepositories, Kustomizations, HelmReleases, notifications                                                                                                     |
| `assay.traefik`      | Routers, services, middlewares, entrypoints, TLS status                                                                                                          |
| `assay.vault`        | KV secrets, policies, auth, transit, PKI, token management                                                                                                       |
| `assay.openbao`      | Alias for vault (OpenBao API-compatible)                                                                                                                         |
| `assay.certmanager`  | Certificates, issuers, ACME orders and challenges                                                                                                                |
| `assay.eso`          | ExternalSecrets, SecretStores, ClusterSecretStores sync status                                                                                                   |
| `assay.dex`          | OIDC discovery, JWKS, health, configuration validation                                                                                                           |
| `assay.zitadel`      | OIDC identity management with JWT machine auth                                                                                                                   |
| `assay.ory.kratos`   | Ory Kratos — login/registration/recovery/settings flows, identities, sessions                                                                                    |
| `assay.ory.hydra`    | Ory Hydra OAuth2/OIDC — clients, authorize URLs, tokens, login/consent, JWKs                                                                                     |
| `assay.ory.keto`     | Ory Keto ReBAC — relation tuples, permission checks, expand                                                                                                      |
| `assay.ory.rbac`     | Capability-based RBAC engine over Keto — roles + capabilities, separation of duties                                                                              |
| `assay.ory`          | Convenience wrapper — `ory.connect()` builds kratos/hydra/keto clients together; also re-exports `rbac`                                                          |
| `assay.crossplane`   | Providers, XRDs, compositions, managed resources                                                                                                                 |
| `assay.velero`       | Backups, restores, schedules, storage locations                                                                                                                  |
| `assay.workflow`     | Native durable workflow engine client — connect, define workflows + activities, listen as worker, start/signal/cancel from any client. Pairs with `assay serve`. |
| `assay.harbor`       | Projects, repositories, artifacts, vulnerability scanning                                                                                                        |
| `assay.healthcheck`  | HTTP checks, JSON path, body matching, latency, multi-check                                                                                                      |
| `assay.s3`           | S3-compatible storage (AWS, R2, MinIO) with Sig V4 auth                                                                                                          |
| `assay.postgres`     | Postgres helpers: users, databases, grants, Vault integration                                                                                                    |
| `assay.unleash`      | Feature flags: projects, environments, features, strategies, tokens                                                                                              |
| `assay.openclaw`     | OpenClaw AI agent — invoke tools, state, diff, approve, LLM tasks                                                                                                |
| `assay.gitlab`       | GitLab REST API v4 — projects, repos, commits, MRs, pipelines, registry                                                                                          |
| `assay.github`       | GitHub REST API — PRs, issues, actions, repos, GraphQL                                                                                                           |
| `assay.gmail`        | Gmail REST API with OAuth2 — search, read, reply, send, labels                                                                                                   |
| `assay.gcal`         | Google Calendar REST API with OAuth2 — events CRUD, calendar list                                                                                                |
| `assay.oauth2`       | Google OAuth2 token management — credentials, auto-refresh, persistence                                                                                          |
| `assay.email_triage` | Email classification — deterministic rules + LLM-assisted triage                                                                                                 |

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
local secret = c.kv:get("secret", "myapp/config")
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
local targets = c.targets:list()
local up_count = 0
for _, t in ipairs(targets.activeTargets) do
  if t.health == "up" then up_count = up_count + 1 end
end
assert.gt(up_count, 0, "No Prometheus targets are up")

-- Query a metric
log.info("Active targets: " .. up_count .. ", up query: " .. tostring(c.queries:instant("up")))
```

### Workflow with signal (assay serve + Lua client)

```lua
#!/usr/bin/assay
-- Worker: registers handlers and listens. Run alongside `assay serve`.
local workflow = require("assay.workflow")
workflow.connect("http://localhost:8080")

workflow.define("ProcessOrder", function(ctx, input)
  local stock = ctx:execute_activity("reserve_stock", input)
  -- Pause until a human approves; survives worker restart.
  local approval = ctx:wait_for_signal("approve")
  return ctx:execute_activity("ship", {
    item = input.item,
    qty = input.quantity,
    approver = approval.by,
  })
end)

workflow.activity("reserve_stock", function(ctx, input)
  -- real impl would hit the inventory service
  return { reserved = input.quantity }
end)

workflow.activity("ship", function(ctx, input)
  return { tracking = "TRK-" .. input.item, approver = input.approver }
end)

workflow.listen({ queue = "orders" })
```

Drive it from any HTTP client:

```sh
# Start
curl -X POST http://localhost:8080/api/v1/workflows -d \
  '{"workflow_type":"ProcessOrder","workflow_id":"order-12345",
    "task_queue":"orders","input":{"item":"widget","quantity":3}}'

# Approve (workflow resumes)
assay workflow signal order-12345 approve '{"by":"alice"}'

# Inspect
assay workflow describe order-12345
```

Crash-safety: if the worker dies between `reserve_stock` completing and the `approve` signal
arriving, the engine reassigns the workflow task to another worker after
`ASSAY_WF_DISPATCH_TIMEOUT_SECS` (default 30s); that worker replays from the event log, sees
`reserve_stock`'s result in history, and resumes waiting on the signal. No duplicate stock
reservations. See `docs/modules/workflow.md` for the full replay model.

## Error Handling

Errors from stdlib methods follow the format: `"<module>: <METHOD> <path> HTTP <status>: <body>"`

Use `pcall` to catch errors without crashing the script:

```lua
local vault = require("assay.vault")

local ok, err = pcall(function()
  local c = vault.client("http://vault:8200", { token = env.get("VAULT_TOKEN") })
  return c.kv:get("secret", "myapp/config")
end)

if not ok then
  log.error("Vault read failed: " .. tostring(err))
  -- handle gracefully or re-raise
  error(err)
end
```

For 404 responses, stdlib modules return `nil` rather than raising an error:

```lua
local secret = c.kv:get("secret", "maybe/exists")
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

**Module not found**: All 34 stdlib modules are embedded in the binary. If `require("assay.foo")`
fails, run `assay modules` to see the exact module names.

**Lua 5.5 specifics**: Assay uses Lua 5.5 (not LuaJIT). Integer division is `//`, bitwise ops use
`&`, `|`, `~`, `<<`, `>>`. The `#` operator on tables counts only the sequence part.

**Debugging**: Add `log.info(json.encode(some_table))` to inspect table contents. The `json.encode`
builtin handles nested tables.

**Workflow engine**: Use `assay serve` + `require("assay.workflow")` for durable workflows with
deterministic-replay crash safety. See the "Workflow engine" section above for the API and
`docs/modules/workflow.md` for the full replay model.

## MCP Replacement

Assay replaces MCP (Model Context Protocol) servers with embedded Lua modules. Instead of running
separate Docker containers for each MCP server, you write one Lua script with
`require("assay.<module>")`.

| MCP Server                                | Stars | Assay Module                     | Coverage |
| ----------------------------------------- | ----- | -------------------------------- | -------- |
| modelcontextprotocol/servers (filesystem) | 79K   | `fs.read/write` builtin          | ✅ Full  |
| modelcontextprotocol/servers (fetch)      | 79K   | `http.*` builtins                | ✅ Full  |
| punkpeye/mcp-postgres                     | 3K+   | `assay.postgres`                 | ✅ Full  |
| wong2/mcp-grafana                         | 2K+   | `assay.grafana`                  | ✅ Full  |
| prometheus-community/mcp-prometheus       | 500+  | `assay.prometheus`               | ✅ Full  |
| [42 MCP servers total]                    | —     | See assay.rs/mcp-comparison.html | —        |

Key insight: MCP servers require persistent processes, auth config, and container overhead. Assay
modules are embedded Lua — zero process overhead, same binary, same auth pattern.

Run `assay context "grafana"` to get prompt-ready method signatures for any module.

## AI Agent Integration

Assay integrates with all major AI coding agents via `assay context <query>` (today) or
`assay mcp-serve` (v0.6.0).

### Claude Code

Add to `.mcp.json` (Coming Soon — v0.6.0):

```json
{
  "mcpServers": {
    "assay": {
      "command": "assay",
      "args": ["mcp-serve"]
    }
  }
}
```

Today — add to your AGENTS.md or .cursorrules:

```
Run `assay context <query>` to get accurate Lua method signatures before writing assay scripts.
Example: `assay context "grafana"` returns all grafana client methods with types.
```

### Cursor

Add to `.cursor/mcp.json` (Coming Soon — v0.6.0):

```json
{
  "mcpServers": {
    "assay": { "command": "assay", "args": ["mcp-serve"] }
  }
}
```

### Windsurf

Add to `~/.codeium/windsurf/mcp_config.json` (Coming Soon — v0.6.0):

```json
{
  "mcpServers": {
    "assay": { "command": "assay", "args": ["mcp-serve"] }
  }
}
```

### Cline / OpenCode

Same pattern — `assay mcp-serve` exposes all modules as MCP tools (v0.6.0).

Today: use `assay context <query>` from terminal and paste output into agent context.

## MCP-Serve Vision (v0.6.0)

`assay mcp-serve` will expose all 51 modules (34 stdlib + 17 builtins) as MCP tools over stdio/SSE
transport:

- Each stdlib module becomes an MCP tool (e.g., `grafana_health`, `k8s_pods`)
- Each builtin becomes an MCP tool (e.g., `http_get`, `crypto_jwt_sign`)
- Agents call tools directly — no Lua scripting required for simple queries
- Lua scripting still available for complex multi-step workflows

Until v0.6.0: use `assay context <query>` + paste into agent context window. See
https://assay.rs/agent-guides.html for complete integration examples.
