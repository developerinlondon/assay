# AGENTS.md

## Skills & Rules

Key coding practices for this project:

- **autonomous-workflow** — Proposal-first development, decision authority, commit hygiene
- **code-quality** — Warnings-as-errors, no underscore prefixes, test coverage, type safety

## Library hygiene: no application-domain leakage

**Assay is a general-purpose library. It must have zero knowledge of any specific application that
uses it.** When writing or modifying any code, test, comment, doc, changelog entry, commit message,
or PR description in this repo, never reference:

- Specific consumer applications by name (e.g. "Command Center", "hydra-login", or any internal
  product)
- Specific deployments, environments, or company-specific URLs (e.g.
  `*.dev.simons.disw.siemens.com`, internal cluster names)
- Company- or project-specific role names, namespaces, client IDs, or resource names that only make
  sense in one consumer's context
- The user's organisation, team, or internal project naming conventions

Use generic placeholder names instead:

- Client IDs: `example-app`, `demo-client`, `app-1`
- Hostnames: `example.com`, `app.example.com`, `hydra.example.com`
- Role objects: `app:role-a`, `namespace1:role-a`, `app:admin`
- Project IDs: `demo-project`, `project-1`
- Workflow names: `MyWorkflow`, `my-queue`

When motivating a new feature in a CHANGELOG entry, commit message, or PR description, describe
**the OIDC/Kubernetes/HTTP scenario it enables**, not **the specific consumer that asked for it**.
The library should read the same to a stranger who has never heard of any of assay's consumers as it
does to someone who works on one of them every day.

This applies to all files in the repo: `stdlib/`, `src/`, `tests/`, `*.md`, `*.html`,
`CHANGELOG.md`, and any commit/PR text. The only legitimate exception is the copyright holder's name
in `LICENSE`/`NOTICE`/`CLA.md`.

## Release docs checklist

Every release (patch, minor, or major) must update **all** of the following before the PR merges —
never ship a release that only touches source + CHANGELOG. The site, llms.txt, and sub-crate
manifests drift silently if you forget them, and agents downstream (including future-you) see
stale information.

### Version bumps

- `Cargo.toml` (workspace root) — bump `[package].version` for the `assay-lua` binary crate.
- **`crates/*/Cargo.toml`** — each sub-crate follows **independent semver tied to its own public
  Rust API**, not the binary's version. Today that's `crates/assay-workflow/Cargo.toml`. Skip this
  and the crates.io publish step of the release workflow fails at tag push time (the crate version
  already exists on the index).
- When a sub-crate version bumps, the workspace root `Cargo.toml` dependency spec must bump to
  match (e.g. `assay-workflow = { ..., version = "0.2" }`). Cargo's lockfile will regenerate on the
  next build; commit the resulting `Cargo.lock` update.

### Pre-1.0 semver: patch bumps by default

**Every release bumps the patch digit. Never jump the minor without an explicit conversation.**

Rationale: assay is pre-1.0. Both the binary (`assay-lua`) and the engine library
(`assay-workflow`) are young and do not have a stable promised API yet. A minor bump on a 0.x
crate conventionally signals a breaking change to downstream Cargo consumers — which matters
once there are downstream consumers. While there aren't, a patch bump is both sufficient and
keeps Cargo dep specs (`version = "0.1"` = `^0.1`, `version = "0.11"` = `^0.11`) stable across
the upgrade.

- `assay-lua` (binary): always patch bump (`0.11.4 → 0.11.5`) unless the user says otherwise.
- `assay-workflow` (engine sub-crate): always patch bump (`0.1.2 → 0.1.3`) unless the user says
  otherwise.
- **Don't pick a minor bump unilaterally.** If a change looks like it justifies one (e.g. a
  public enum becomes a struct, a trait signature changes), surface the tradeoff to the user and
  ask before touching the version field. Discussion first, edit second.

The binary version and sub-crate version are independent. It's normal for `assay-lua 0.11.5` to
ship `assay-workflow 0.1.3`; they have unrelated bump cadences.

### Non-source files

- `CHANGELOG.md` — new section at the top; describe the OIDC/Kubernetes/HTTP scenario enabled, not
  a specific consumer.
- `docs/modules/*.md` — any module whose surface changed.
- `README.md`, `SKILL.md`, `AGENTS.md`, `skills/assay/SKILL.md` — auth / CLI / API tables if
  touched.
- `llms.txt` (root) — one-liner module descriptors.
- `site/pages/index.html` — the release banner (`v0.x.y` tag + copy) and any version badge on
  feature cards (e.g. the Workflow Engine card).
- `site/static/llms.txt` — site's static teaser; keep it reasonably fresh (not auto-generated from
  the root `llms.txt`).
- Any site page that references the previous version in prose or code (e.g. the `mise` /
  `crates.io` install snippets on `index.html`).

### Verification before opening the PR

```sh
# No stale version references in shipping content:
grep -rn "v0.PREVIOUS" . --include="*.md" --include="*.html" --include="*.toml" --include="*.txt"

# Every crate manifest version field matches what you intend to publish:
grep -E '^version = ' Cargo.toml crates/*/Cargo.toml

# Dep spec in the root Cargo.toml tracks the sub-crate bump:
grep -A0 'assay-workflow' Cargo.toml | grep 'version ='
```

Only matches that should remain for the first grep are historical CHANGELOG entries and
"introduced in vX.Y.Z" feature markers. The second and third greps exist because v0.11.4's
crates.io publish failed when the sub-crate version was missed — do not skip them.

## PR merge process

Follow this sequence for every PR merge into `main`, even as an admin who could bypass branch
protection. Branch protection is the enforcement layer, the process below is the discipline
layer — protection is the last line of defence, not the only one.

### 1. Wait for CI to settle fully

```sh
gh pr checks <PR> --watch
```

**Do not use `--required`.** On repos without required-check protection, `--required` prints "no
required checks reported" and exits immediately, letting a merge-chain proceed while checks are
still running. Even on protected repos, `--required` only watches the required contexts — other
CI in the same workflow may still be running. Plain `--watch` blocks on every check.

### 2. Never merge through red

If any check ends non-green, inspect before deciding:

```sh
gh run view <run-id>
gh api repos/developerinlondon/assay/actions/jobs/<job-id>/logs
```

- **Infrastructure flakes** — runner `No space left on device`, `Runner failed to dispatch`,
  transient network timeouts pulling crates, GitHub outages — re-run the failed job on a fresh
  runner:

  ```sh
  gh run rerun <run-id> --failed
  gh pr checks <PR> --watch   # wait again
  ```

  Merging through an infra flake is gambling that the flake hides nothing. Occasionally it does.

- **Code failures** — test assertion, clippy error, compile error — fix in a new commit on the
  same branch, push, wait for CI. Never merge a PR with a known code failure on the promise of a
  "follow-up fix" — the follow-up lives in the same PR.

### 3. Squash-merge with branch delete

```sh
gh pr merge <PR> --squash --delete-branch
```

Prefer `--squash` over `--merge` so `main` history stays one-commit-per-PR and greppable. Let
`--delete-branch` clean up the remote branch; don't leave stale feature branches around.

### 4. Verify the post-merge CI run

The `push` trigger on `ci.yml` fires a fresh CI run on the merge commit itself. Occasionally a
merge commit behaves differently than the PR HEAD (e.g. silent conflict with concurrent merges,
unrelated upstream drift). Confirm it lands green before triggering anything downstream (tag
push, release workflow, deploy):

```sh
gh run list --workflow=ci.yml --branch main --limit 3
```

If the first entry isn't `success`, treat `main` as broken and fix before anything else. Don't
tag a release off a main that isn't green.

### Why this exists

v0.11.5 was merged with one CI check still in progress because the merge-chain used
`gh pr checks 46 --watch --required` on a repo that had no required checks configured. The
watch command exited immediately, the auto-merge fired, and the still-running ubuntu check job
later errored on a runner infrastructure flake (`No space left on device`). The post-merge CI
on `main` was green, so nothing broke — but the only reason was luck. Branch protection on
`main` was added retroactively (see "PR merge process → 1" above for the correct watch
invocation that would have caught this).

## What is Assay

General-purpose enhanced Lua runtime. Single ~11 MB static binary with batteries included: HTTP
client/server, JSON/YAML/TOML, crypto, database, WebSocket, filesystem, shell execution, process
management, async, and 34 embedded stdlib modules for infrastructure services (Kubernetes,
Prometheus, Vault, ArgoCD, etc.) and AI agent integrations (OpenClaw, GitHub, Gmail, Google
Calendar).

Use cases:

- **Standalone scripting** — system automation, CI/CD tasks, file processing
- **Embedded runtime** — other Rust services embed assay as a library (`pub mod lua`)
- **Kubernetes Jobs** — replaces 50–250 MB Python/Node/kubectl containers (~11 MB image)
- **Infrastructure automation** — GitOps hooks, health checks, service configuration

- **Repo**: [github.com/developerinlondon/assay](https://github.com/developerinlondon/assay)
- **Image**: `ghcr.io/developerinlondon/assay:latest` (~11 MB compressed)
- **Crate**: [crates.io/crates/assay-lua](https://crates.io/crates/assay-lua)
- **Stack**: Rust (2024 edition), Tokio, Lua 5.5 (mlua), reqwest, clap, axum

## Three Modes

```bash
assay script.lua     # Lua mode — run script with all builtins
assay checks.yaml    # YAML mode — structured checks with retry/backoff/parallel
assay serve          # Workflow engine mode — REST+SSE API, dashboard, CLI
```

### Workflow engine mode — short overview

Full reference: `docs/modules/workflow.md`. One binary runs the engine (`assay serve`), the CLI that
drives it (`assay workflow/schedule/namespace/worker/queue/completion …`), and the Lua client
(`require("assay.workflow")`) for worker processes and management scripts.

```bash
assay serve --backend postgres://... --port 8080       # engine (multi-instance via Postgres)
assay workflow start --type Deploy --input @req.json   # start a run; --input takes @file/-/literal
assay workflow wait <id> --timeout 300                 # block for scripts; exit 0/1/2
assay schedule create nightly --cron "0 0 2 * * *" --timezone Europe/Berlin  --type Report
assay completion bash > /etc/bash_completion.d/assay   # generate shell completion
```

Global CLI flags (all env-backed and config-file-backed): `--engine-url`, `--api-key`,
`--namespace`, `--output`, `--config`. YAML config file auto-discovered at `--config PATH` /
`$ASSAY_CONFIG_FILE` / `$XDG_CONFIG_HOME/assay/config.yaml` / `~/.config/assay/config.yaml` /
`/etc/assay/config.yaml`. `api_key_file:` keeps the secret out of env/argv. Output formats: table
(TTY default), json (pipe default), jsonl, yaml.

Lua surface (worker + management in one module):

```lua
local workflow = require("assay.workflow")
workflow.connect("http://assay:8080", { token = env.get("ASSAY_API_KEY") })

-- Worker: define handlers, listen. ctx gets execute_activity / execute_parallel (v0.11.3) /
-- sleep / wait_for_signal / start_child_workflow / side_effect / register_query (v0.11.3) /
-- upsert_search_attributes (v0.11.3) / continue_as_new (v0.11.3).
workflow.define("Pipeline", function(ctx, input) ... end)
workflow.activity("step", function(ctx, input) ... end)
workflow.listen({ queue = "default" })  -- blocks

-- Management: workflow.list/describe/get_events/get_state/list_children/continue_as_new,
-- workflow.schedules.{create,list,describe,patch,pause,resume,delete},
-- workflow.namespaces.{create,list,describe,stats,delete},
-- workflow.workers.list, workflow.queues.stats.
```

**Dashboard** at `/workflow/` — reads + tier-1 operator controls (start, signal, cancel, terminate,
continue-as-new, live `register_query` state, schedule CRUD + pause/resume, namespace
create/delete). Engine version shown in the status bar, fetched from `/api/v1/version`.

**Optional S3 archival** (cargo feature `s3-archival`, default-off). Enabled when
`ASSAY_ARCHIVE_S3_BUCKET` is set. Bundles completed workflows to S3 after
`ASSAY_ARCHIVE_RETENTION_DAYS` and stubs the row with `archive_uri`.

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

| Category       | Functions                                                                                                                                                                                                                                                                                                                |
| -------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| HTTP           | `http.get(url, opts?)`, `http.post(url, body, opts?)`, `http.put(url, body, opts?)`, `http.patch(url, body, opts?)`, `http.delete(url, opts?)`, `http.serve(port, routes)` — `http.serve` response handlers accept array header values to emit the same header name multiple times (e.g., multiple `Set-Cookie`)         |
| JSON/YAML/TOML | `json.parse(str)`, `json.encode(tbl)`, `yaml.parse(str)`, `yaml.encode(tbl)`, `toml.parse(str)`, `toml.encode(tbl)`                                                                                                                                                                                                      |
| Filesystem     | `fs.read(path)`, `fs.read_bytes(path)`, `fs.write(path, str)`, `fs.write_bytes(path, data)`, `fs.remove(path)`, `fs.list(path)`, `fs.stat(path)`, `fs.mkdir(path)`, `fs.exists(path)`, `fs.copy(src, dst)`, `fs.rename(src, dst)`, `fs.glob(pattern)`, `fs.tempdir()`, `fs.chmod(path, mode)`, `fs.readdir(path, opts?)` |
| Crypto         | `crypto.jwt_sign(claims, key, alg, opts?)`, `crypto.jwt_decode(token)` (decode without verifying), `crypto.hash(str, alg)`, `crypto.hmac(key, data, alg?, raw?)`, `crypto.random(len)`                                                                                                                                   |
| Base64         | `base64.encode(str)`, `base64.decode(str)`                                                                                                                                                                                                                                                                               |
| Regex          | `regex.match(pat, str)`, `regex.find(pat, str)`, `regex.find_all(pat, str)`, `regex.replace(pat, str, repl)`                                                                                                                                                                                                             |
| Database       | `db.connect(url)`, `db.query(conn, sql, params?)`, `db.execute(conn, sql, params?)`, `db.close(conn)`                                                                                                                                                                                                                    |
| WebSocket      | `ws.connect(url)`, `ws.send(conn, msg)`, `ws.recv(conn)`, `ws.close(conn)`                                                                                                                                                                                                                                               |
| Templates      | `template.render(path, vars)`, `template.render_string(tmpl, vars)`                                                                                                                                                                                                                                                      |
| Async          | `async.spawn(fn)`, `async.spawn_interval(fn, ms)`, `handle:await()`, `handle:cancel()`                                                                                                                                                                                                                                   |
| Assert         | `assert.eq(a, b, msg?)`, `assert.ne(a, b, msg?)`, `assert.gt(a, b, msg?)`, `assert.lt(a, b, msg?)`, `assert.contains(str, sub, msg?)`, `assert.not_nil(val, msg?)`, `assert.matches(str, pat, msg?)`                                                                                                                     |
| Logging        | `log.info(msg)`, `log.warn(msg)`, `log.error(msg)`                                                                                                                                                                                                                                                                       |
| Utilities      | `env.get(key)`, `env.set(key, value)`, `env.list()`, `sleep(secs)`, `time()`                                                                                                                                                                                                                                             |
| Shell          | `shell.exec(cmd, opts?)` — execute commands with timeout, working dir, env                                                                                                                                                                                                                                               |
| Process        | `process.list()`, `process.is_running(name)`, `process.kill(pid, signal?)`, `process.wait_idle(names, timeout, interval)`                                                                                                                                                                                                |
| Disk           | `disk.usage(path)` — returns `{total, used, available, percent}`, `disk.sweep(dir, age_secs)`, `disk.dir_size(path)`                                                                                                                                                                                                     |
| OS             | `os.hostname()`, `os.arch()`, `os.platform()`                                                                                                                                                                                                                                                                            |
| Markdown       | `markdown.to_html(source)` — convert Markdown to HTML (tables, strikethrough, task lists)                                                                                                                                                                                                                                |

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

Header values can be either a string or an array of strings. Array values emit the same header name
multiple times — required for `Set-Cookie` with multiple cookies and useful for `Link`, `Vary`,
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

SSE `send()` accepts: `event` (string), `data` (string), `id` (string), `retry` (integer). `event`
and `id` must not contain newlines. `data` handles multi-line automatically.

## Stdlib Modules

35 embedded Lua modules loaded via `require("assay.<name>")`:

| Module               | Description                                                                                                                                                                                                                                                                     |
| -------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `assay.prometheus`   | Query metrics, alerts, targets, rules, label values, series                                                                                                                                                                                                                     |
| `assay.alertmanager` | Manage alerts, silences, receivers, config                                                                                                                                                                                                                                      |
| `assay.loki`         | Push logs, query, labels, series                                                                                                                                                                                                                                                |
| `assay.grafana`      | Health, dashboards, datasources, annotations                                                                                                                                                                                                                                    |
| `assay.k8s`          | 30+ resource types, CRDs, readiness checks                                                                                                                                                                                                                                      |
| `assay.argocd`       | Apps, sync, health, projects, repositories                                                                                                                                                                                                                                      |
| `assay.kargo`        | Stages, freight, promotions, verification                                                                                                                                                                                                                                       |
| `assay.flux`         | GitRepositories, Kustomizations, HelmReleases                                                                                                                                                                                                                                   |
| `assay.traefik`      | Routers, services, middlewares, entrypoints                                                                                                                                                                                                                                     |
| `assay.vault`        | KV secrets, policies, auth, transit, PKI                                                                                                                                                                                                                                        |
| `assay.openbao`      | Alias for vault (API-compatible)                                                                                                                                                                                                                                                |
| `assay.certmanager`  | Certificates, issuers, ACME challenges                                                                                                                                                                                                                                          |
| `assay.eso`          | ExternalSecrets, SecretStores, ClusterSecretStores                                                                                                                                                                                                                              |
| `assay.dex`          | OIDC discovery, JWKS, health                                                                                                                                                                                                                                                    |
| `assay.zitadel`      | OIDC identity management with JWT machine auth                                                                                                                                                                                                                                  |
| `assay.ory.kratos`   | Ory Kratos identity — login/registration/recovery/settings flows, identities, sessions, schemas                                                                                                                                                                                 |
| `assay.ory.hydra`    | Ory Hydra OAuth2/OIDC — clients, authorize URLs, tokens, login/consent, introspection, JWKs                                                                                                                                                                                     |
| `assay.ory.keto`     | Ory Keto ReBAC — relation tuples, permission checks, role/group membership, expand                                                                                                                                                                                              |
| `assay.ory.rbac`     | Capability-based RBAC engine over Keto — define roles + capabilities, query users, manage memberships, separation of duties                                                                                                                                                     |
| `assay.ory`          | Convenience wrapper re-exporting kratos/hydra/keto/rbac with `ory.connect(opts)`                                                                                                                                                                                                |
| `assay.crossplane`   | Providers, XRDs, compositions, managed resources                                                                                                                                                                                                                                |
| `assay.velero`       | Backups, restores, schedules, storage locations                                                                                                                                                                                                                                 |
| `assay.harbor`       | Projects, repositories, artifacts, vulnerability scanning                                                                                                                                                                                                                       |
| `assay.workflow`     | Workflow engine client — define workflows/activities + listen as worker; or full management surface (list/start/signal/cancel/terminate/state/events/children/continue-as-new plus `.schedules`/`.namespaces`/`.workers`/`.queues` sub-tables). See `docs/modules/workflow.md`. |
| `assay.healthcheck`  | HTTP checks, JSON path, body matching, latency, multi-check                                                                                                                                                                                                                     |
| `assay.s3`           | S3-compatible storage (AWS, R2, MinIO) with Sig V4                                                                                                                                                                                                                              |
| `assay.postgres`     | Postgres-specific helpers                                                                                                                                                                                                                                                       |
| `assay.unleash`      | Feature flags: projects, environments, features, strategies, API tokens                                                                                                                                                                                                         |
| `assay.openclaw`     | OpenClaw AI agent platform — invoke tools, state, diff, approve, LLM tasks                                                                                                                                                                                                      |
| `assay.gitlab`       | GitLab REST API v4 — projects, repos, commits, MRs, pipelines, issues, registry                                                                                                                                                                                                 |
| `assay.github`       | GitHub REST API — PRs, issues, actions, repos, GraphQL                                                                                                                                                                                                                          |
| `assay.gmail`        | Gmail REST API with OAuth2 — search, read, reply, send, labels                                                                                                                                                                                                                  |
| `assay.gcal`         | Google Calendar REST API with OAuth2 — events CRUD, calendar list                                                                                                                                                                                                               |
| `assay.oauth2`       | Google OAuth2 token management — file-based credentials, auto-refresh, persistence                                                                                                                                                                                              |
| `assay.email_triage` | Email classification — deterministic rules + optional LLM-assisted triage via OpenClaw                                                                                                                                                                                          |

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

| Key             | Default        | Description                         |
| --------------- | -------------- | ----------------------------------- |
| `binaryPath`    | PATH lookup    | Explicit path to the `assay` binary |
| `timeout`       | `20`           | Execution timeout in seconds        |
| `maxOutputSize` | `524288`       | Maximum stdout collected from Assay |
| `scriptsDir`    | workspace root | Root directory for Lua scripts      |

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

### 3. Add documentation

Add a `docs/modules/<name>.md` file with method signatures and examples. This is the single source
of truth — `site/build.lua` generates the website pages and `llms-full.txt` from these files. No
need to manually update `README.md`, `SKILL.md`, or the website.

### 4. Verify

```bash
cargo check && cargo clippy -- -D warnings && cargo test
assay site/build.lua   # verify docs build
```

## Directory Structure

```
assay/
├── Cargo.toml
├── Cargo.lock
├── CHANGELOG.md              # Release notes (manual, rendered to website by build.lua)
├── AGENTS.md                 # This file — agent instructions
├── SKILL.md                  # LLM agent integration guide
├── README.md                 # Human-facing README
├── Dockerfile                # Multi-stage: rust builder -> scratch
├── src/
│   ├── main.rs               # CLI entry point (clap)
│   ├── lib.rs                # Library root (pub mod lua)
│   └── lua/
│       ├── mod.rs            # VM setup, sandbox, stdlib loader (include_dir!)
│       └── builtins/
│           ├── mod.rs        # register_all() — wires builtins into Lua globals
│           ├── http.rs       # http.{get,post,put,patch,delete,serve} + wildcard routes
│           ├── core.rs       # env, sleep, time, fs (read/write + read_bytes/write_bytes), base64, regex, log, async
│           ├── markdown.rs   # markdown.to_html() via pulldown-cmark
│           └── ...           # json, yaml, toml, crypto, db, ws, template, assert, etc.
├── stdlib/                   # Embedded Lua modules (auto-discovered)
│   ├── vault.lua             # Comprehensive reference (330 lines)
│   ├── grafana.lua           # Simple reference (110 lines)
│   └── ...                   # 34 modules total
├── docs/
│   └── modules/              # Module documentation (single source of truth)
│       ├── ory.md
│       └── ...               # 35 markdown files
├── site/                     # Website source (tracked in git)
│   ├── build.lua             # Assay builds its own docs (replaces bash/npm)
│   ├── serve.lua             # Dev server using http.serve() with wildcard routes
│   ├── pages/                # HTML templates with __PLACEHOLDER__ markers
│   ├── partials/             # header.html, footer.html (nav, theme toggle, search)
│   └── static/               # style.css, _headers, _redirects, llms.txt
├── build/                    # Build output (gitignored)
│   └── site/                 # Generated website, deployed to Cloudflare Pages
├── tests/
│   ├── common/mod.rs         # Test helpers: run_lua(), create_vm(), eval_lua()
│   ├── stdlib_vault.rs       # One test file per stdlib module
│   ├── http_serve.rs         # HTTP server tests (incl. wildcard routes)
│   ├── markdown.rs           # Markdown builtin tests
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

### When making changes

1. **Code changes** — implement in `src/` or `stdlib/`, add tests in `tests/`
2. **Document** — add/update `docs/modules/<name>.md` (single source of truth)
3. **CHANGELOG.md** — add entry under the current version section
4. **AGENTS.md** — update builtins table if API surface changed
5. **Verify**: `cargo clippy --tests -- -D warnings && cargo test && assay site/build.lua`

The following are **auto-generated** — do NOT edit manually:

- `build/site/` — entire website (generated by `assay site/build.lua`)
- Module count on the website (computed from `src/lua/builtins/mod.rs` + `stdlib/`)
- `build/site/llms-full.txt` (concatenated from `docs/modules/*.md`)
- `build/site/modules/*.html` (generated from `docs/modules/*.md`)
- `build/site/modules.html` (index page)
- Changelog page (rendered from `CHANGELOG.md`)

### Releasing a version

1. **Bump version** in `Cargo.toml`
2. **Finalize CHANGELOG.md** — ensure version header has correct date
3. **Run checks**: `cargo clippy --tests -- -D warnings && cargo test`
4. **Merge PR** to main
5. **Tag the release**: `git tag v0.9.0 && git push origin v0.9.0`

The tag push triggers `.github/workflows/release.yml` which publishes:

- GitHub Release (binaries + checksums)
- crates.io (`assay-lua` crate)
- Docker image (`ghcr.io/developerinlondon/assay`)

The merge to main triggers `.github/workflows/deploy.yml` which:

- Builds assay (`cargo build --release`)
- Builds the website (`assay site/build.lua`)
- Indexes for search (`pagefind --site build/site`)
- Deploys to Cloudflare Pages (`wrangler pages deploy build/site/`)

### Local development

```bash
cargo build --release               # build assay
assay site/build.lua                 # build website
npx pagefind --site build/site       # index for search (optional)
assay site/serve.lua                 # serve at http://localhost:3000
```

## Commands

```bash
cargo check                        # Type check
cargo clippy -- -D warnings        # Lint (warnings = errors)
cargo test                         # Run all tests
cargo build --release              # Release build (~11 MB)
```
