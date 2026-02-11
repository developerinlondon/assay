# Plan 02: Assay Runtime Evolution

Status: APPROVED Created: 2026-02-10 Decision: Evolve Assay into a general-purpose Lua runtime for
Kubernetes

## Summary

Assay v0.2.0 is a 5.1 MB verification runner for K8s PostSync hooks. This plan evolves it into a
full-featured Lua runtime for Kubernetes — covering verification, scripting, automation, and
lightweight web services — in a single ~9 MB binary that replaces 50-250 MB Python/Node/kubectl
containers.

One binary, auto-detected behavior:

```
assay config.yaml           # YAML → check orchestration (retry, backoff, structured output)
assay script.lua            # Lua → run it (all builtins, script decides what to do)
assay --sandbox script.lua  # Lua → restricted builtins (future: untrusted user code)
```

## Naming

The tool is evolving beyond "verification runner" into a general-purpose K8s Lua runtime. No
production users exist yet — renaming cost is zero. Options:

| Name         | Meaning                                  | Pros                                   | Cons                       |
| ------------ | ---------------------------------------- | -------------------------------------- | -------------------------- |
| **Assay**    | "to test/examine" (also "an attempt")    | Unique, has domain (assay.rs), on GHCR | Name suggests testing only |
| **Luma**     | "Lua" + "machine"; also means "light"    | Short, memorable, conveys lightweight  | New, no history            |
| **Crucible** | Container where metals are tested/shaped | Perfect metaphor (test + create)       | Longer to type             |

Recommendation: TBD — owner decides.

## Architecture

```
+------------------------------------------------------------------+
| Assay v0.1.0 (~9 MB static MUSL binary, Alpine container)       |
|                                                                  |
| CLI (auto-detected by file extension):                           |
|   assay config.yaml           (.yaml -> check orchestration)     |
|   assay script.lua            (.lua  -> run script)              |
|   assay --sandbox script.lua  (restricted builtins)              |
|                                                                  |
| Shebang support:                                                 |
|   #!/usr/bin/assay            (works like #!/usr/bin/python3)    |
|                                                                  |
| Rust Core:                                                       |
|   Config parser (YAML) -> Runner (retry/backoff/timeout)         |
|   -> Structured JSON output -> Exit code (0/1)                   |
|                                                                  |
| Lua Builtins (Rust-backed, all available to .lua scripts):       |
|   http.{get,post,put,patch,delete}  http.serve(port, routes)     |
|   ws.{connect,accept,send,recv}                                  |
|   json.{parse,encode}  yaml.{parse,encode}  toml.{parse,encode}  |
|   fs.{read,write}  base64.{encode,decode}                        |
|   crypto.{jwt_sign,hash,random}  regex.{match,find,replace}      |
|   db.{connect,query,execute}  (postgres, mysql, sqlite)          |
|   template.{render,render_string}                                |
|   assert.{eq,gt,lt,contains,not_nil,matches}                     |
|   log.{info,warn,error}  env.get  sleep  time                    |
|   async.{spawn,spawn_interval}                                   |
|                                                                  |
| Lua Stdlib (embedded .lua files via include_dir!):               |
|   Monitoring: prometheus, alertmanager, loki, grafana             |
|   K8s/GitOps: k8s, argocd, kargo, flux, traefik                 |
|   Security:   vault, openbao (alias), certmanager, eso, dex      |
|   Infra:      crossplane, velero, temporal, harbor               |
|   Utilities:  healthcheck                                        |
|                                                                  |
| Security:                                                        |
|   .yaml checks: Sandboxed (safe builtins only, fresh VM)        |
|   .lua scripts: All builtins, 64 MB memory limit                |
|   --sandbox:    Restricted builtins (future: untrusted code)     |
+------------------------------------------------------------------+
```

### Behavior by File Type

| Aspect        | `.yaml` (check orchestration)         | `.lua` (script execution)                       |
| ------------- | ------------------------------------- | ----------------------------------------------- |
| Input         | YAML config + Lua scripts             | Single .lua file                                |
| VM lifecycle  | Fresh per check (isolated)            | Single VM for script lifetime                   |
| Builtins      | Sandboxed (http, json, assert only)   | All builtins available                          |
| Output        | Structured JSON, exit code 0/1        | stdout/stderr, exit code                        |
| Retry/backoff | Built-in (YAML config)                | Manual (in Lua)                                 |
| Shebang       | N/A                                   | `#!/usr/bin/assay`                              |
| Use cases     | ArgoCD hooks, Kargo verify, E2E tests | K8s jobs, cron, web services, automation, tools |

The script decides its own behavior — there is no "serve mode". A script that calls
`http.serve(8080, routes)` becomes a web service. A script that calls `http.get()` and exits is a
job. Same binary, same builtins.

## Current State (v0.2.0)

- Binary: 5.1 MB (release, stripped, MUSL static)
- Direct deps: 10 crates
- Transitive deps: 239 packages
- Docker image: ~10 MB (Alpine 3.21 + binary)
- Deployed: 7 verification Jobs in jeebon test/dev (ArgoCD PostSync hooks)
- Builtins: http.{get,post,put,patch}, json.{parse,encode}, assert._, log._, env.get, sleep, time,
  prometheus.query
- Check types: `type: http`, `type: prometheus`, `type: script` (Lua)

## Comparison with Alternatives

### Container Image Size

```
+------------------------------------------------------------------+
| Docker image size comparison (compressed pull)                   |
|                                                                  |
| Assay Full       ## 6 MB                                         |
| Python alpine    ########## 17 MB                                |
| bitnami/kubectl  #################### 35 MB                     |
| Python slim      ########################## 43 MB               |
| Node.js alpine   ################################## 57 MB       |
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
| Deno            |      75 MB |    200 MB |   13x    | Partial |     No     |
| Node.js slim    |      79 MB |    240 MB |   16x    |   No    |     No     |
| Bun             |      85 MB |    250 MB |   17x    |   No    |     No     |
| postman/newman  |     165 MB |    450 MB |   28x    |   No    |     No     |

### Feature Comparison

| Feature                  | Assay | Python | Node.js |  Deno   | Go binary | Shell+curl |
| ------------------------ | :---: | :----: | :-----: | :-----: | :-------: | :--------: |
| HTTP client              |  Yes  |  Yes   |   Yes   |   Yes   |    Yes    |    curl    |
| HTTP server              |  Yes  |  Yes   |   Yes   |   Yes   |    Yes    |     No     |
| WebSocket                |  Yes  |  pip   |   npm   |   Yes   |    Yes    |     No     |
| Database (SQL)           |  Yes  |  pip   |   npm   |   npm   |    Yes    |     No     |
| JSON/YAML/TOML           |  Yes  |  Yes   |   npm   |   Yes   |    Yes    |   jq/yq    |
| JWT signing              |  Yes  |  pip   |   npm   |   npm   |    Yes    |  openssl   |
| Regex                    |  Yes  |  Yes   |   Yes   |   Yes   |    Yes    |    grep    |
| Templates                |  Yes  |  Yes   |   npm   |   npm   |    Yes    |     No     |
| Sandbox                  |  Yes  |   No   |   No    |   Yes   |    No     |     No     |
| Retry/backoff (built-in) |  Yes  |   No   |   No    |   No    |    No     |     No     |
| Structured assertions    |  Yes  | pytest |  jest   |   Yes   |  testing  |     No     |
| Structured JSON output   |  Yes  |   No   |   No    |   No    |    No     |     No     |
| K8s exit code handling   |  Yes  |   No   |   No    |   No    |    No     |     No     |
| No compile step          |  Yes  |  Yes   |   Yes   |   Yes   |  **No**   |    Yes     |
| Image size               | 15 MB | 50 MB+ | 180 MB+ | 200 MB+ | 15-30 MB  |   50 MB+   |

### Shell Tool Equivalents

| Shell Tool | Assay Equivalent                        | Advantage                                    |
| ---------- | --------------------------------------- | -------------------------------------------- |
| curl       | `http.get/post/put/patch/delete`        | Structured response, error handling, retry   |
| jq         | `json.parse` + Lua table access         | Actual programming (loops, conditions)       |
| yq         | `yaml.parse/encode`                     | Same                                         |
| base64     | `base64.encode/decode`                  | Built-in, no pipe chains                     |
| openssl    | `crypto.jwt_sign/hash`                  | Focused on K8s needs                         |
| grep/sed   | `regex.match/find/replace`              | Programming language, not line-oriented      |
| kubectl    | `http.get` to K8s API + service account | No kubectl binary needed (saves 35 MB image) |

## Per-Feature Cost Breakdown

### Binary Size Impact

Assay already has reqwest + tokio + hyper + tower + serde. Adding axum/websocket shares most of
their weight.

| Feature                              | Crate(s)                   | Binary Delta | New Deps | AI Agent Time |  Risk   |
| ------------------------------------ | -------------------------- | :----------: | :------: | :-----------: | :-----: |
| **Step 1: Core Builtins (P0)**       |                            |              |          |               |         |
| fs.read                              | (stdlib)                   |    +0 KB     |    0     |    30 min     |   LOW   |
| crypto.jwt_sign                      | jsonwebtoken 10.3, zeroize |   +200 KB    |    2     |     1 hr      |   LOW   |
| http.delete                          | (existing reqwest)         |    +0 KB     |    0     |    15 min     | TRIVIAL |
| base64.encode/decode                 | data-encoding              |    +10 KB    |    1     |    20 min     | TRIVIAL |
| DRY http builtins (loop)             | refactor                   |    +0 KB     |    0     |    30 min     | TRIVIAL |
| Lua stdlib system                    | include_dir                |    +30 KB    |    1     |     1 hr      |   LOW   |
| **Step 2: Foundation (P1)**          |                            |              |          |               |         |
| crypto.hash                          | sha2, sha3                 |   +100 KB    |    2     |    30 min     |   LOW   |
| crypto.random                        | (stdlib rand)              |    +50 KB    |    1     |    20 min     | TRIVIAL |
| regex                                | regex-lite                 |    +94 KB    |    1     |    45 min     |   LOW   |
| Lua stdlib helpers                   | (embedded .lua)            |    +10 KB    |    0     |     1 hr      |   LOW   |
| **Step 3: General Purpose (P2)**     |                            |              |          |               |         |
| fs.write                             | (stdlib)                   |    +0 KB     |    0     |    30 min     |   LOW   |
| yaml.parse/encode                    | (existing serde_yml)       |    +0 KB     |    0     |    30 min     | TRIVIAL |
| toml.parse/encode                    | toml                       |    +80 KB    |    1     |    20 min     | TRIVIAL |
| async.spawn                          | (existing tokio)           |    +0 KB     |    0     |     2 hrs     | MEDIUM  |
| **Step 4: Server Mode**              |                            |              |          |               |         |
| http.serve (axum)                    | axum (minimal features)    |   +150 KB    |    3     |     4 hrs     | MEDIUM  |
| Routing + middleware                 | (included in axum)         |    +0 KB     |    0     |     2 hrs     | MEDIUM  |
| Static file serving                  | tower-http                 |    +50 KB    |    1     |     1 hr      |   LOW   |
| **Step 5: Database**                 |                            |              |          |               |         |
| db.connect/query (Postgres)          | sqlx (postgres)            |   +1.2 MB    |    8     |     4 hrs     | MEDIUM  |
| db.connect/query (MySQL/MariaDB)     | sqlx (mysql)               |   +0.8 MB    |    2     |     1 hr      |   LOW   |
| db.connect/query (SQLite embedded)   | sqlx (sqlite)              |   +0.5 MB    |    2     |     1 hr      |   LOW   |
| **Step 6: WebSocket + Templates**    |                            |              |          |               |         |
| WebSocket                            | tokio-tungstenite          |   +200 KB    |    2     |     3 hrs     | MEDIUM  |
| template.render                      | minijinja                  |   +300 KB    |    1     |     2 hrs     |   LOW   |
| **Step 7: E2E + Polish**             |                            |              |          |               |         |
| E2E dogfood tests                    | (assay itself)             |    +0 KB     |    0     |     3 hrs     |   LOW   |
| Docs + README                        |                            |    +0 KB     |    0     |     2 hrs     |   LOW   |
| **Step 8: Stable Release (v0.1.0)**  |                            |              |          |               |         |
| Stable API audit + crates.io publish |                            |    +0 KB     |    0     |     3 hrs     |   LOW   |
| **Totals**                           |                            | **+3.7 MB**  |  **27**  |  **~42 hrs**  |         |

### Binary Size Progression

```
v0.0.1  ###########################  5.1 MB  (current baseline)
Step 1  #############################  5.3 MB  (+jwt, +fs.read, +base64, +stdlib)
Step 2  ##############################  5.5 MB  (+crypto, +regex)
Step 3  ##############################  5.6 MB  (+yaml, +toml, +async)
Step 4  ###############################  5.8 MB  (+axum server)
Step 5  #####################################  8.3 MB  (+sqlx postgres/mysql/sqlite)
Step 6  ######################################  9.0 MB  (+websocket, +templates)
v0.1.0  ######################################  ~9 MB   (stable, all features)
```

Docker image: Alpine 3.21 (3.6 MB) + binary (~9 MB) = **~13 MB on-disk, ~6 MB compressed pull.**

## Rubernetes Integration

Rubernetes (plan 07) is a from-scratch Rust implementation of Kubernetes. One ~65 MB binary replaces
K8s + ArgoCD + Kargo + KServe + Dashboard.

### Binary Budget Impact

```
+-------------------------------------------------------+
| Rubernetes Binary Budget: 65 MB                       |
|                                                       |
| K8s core (API, scheduler, controllers)   ~20 MB       |
| Nushell (interactive REPL)               ~10 MB       |
| LanceDB + vectors                        ~14 MB       |
| GitOps engine                             ~8 MB       |
| AI Gateway                                ~2 MB       |
| Dashboard (embedded web UI)               ~2 MB       |
| -----------------------------------------------      |
| Subtotal (without Assay)                 ~56 MB       |
| Assay Lua runtime (incremental)           ~1.5 MB     |
| Total                                    ~57.5 MB     |
| Buffer remaining                          ~7.5 MB     |
+-------------------------------------------------------+
```

Incremental cost is only ~1.5 MB because Rubernetes already links most of Assay's dependencies:

| Component             | Already in Rubernetes? | Incremental |
| --------------------- | :--------------------: | :---------: |
| mlua (Lua 5.4 VM)     |     Yes (plan 07e)     |    0 MB     |
| reqwest (HTTP client) |    Yes (AI gateway)    |    0 MB     |
| tokio (async runtime) |       Yes (core)       |    0 MB     |
| serde/json/yaml       |     Yes (K8s API)      |    0 MB     |
| axum (HTTP server)    |    Yes (API server)    |    0 MB     |
| WebSocket             |  Yes (watch streams)   |    0 MB     |
| regex                 |  Yes (nushell has it)  |    0 MB     |
| sqlx (Postgres)       |       Maybe not        |   +1.2 MB   |
| minijinja (templates) |           No           |   +0.3 MB   |
| **Total incremental** |                        | **~1.5 MB** |

### Assay + Nushell: Complementary Roles

Assay does NOT replace Nushell in Rubernetes. They serve different users:

| Aspect       | Nushell (Human Interface)        | Assay/Lua (Machine Interface)             |
| ------------ | -------------------------------- | ----------------------------------------- |
| Primary user | Human operators at a REPL        | The control plane itself                  |
| Interaction  | Interactive, tab completion      | Programmatic, script files                |
| Startup      | ~50 ms (acceptable for REPL)     | <1 ms (critical for 1000s of hooks)       |
| Memory       | ~10 MB                           | ~200 KB per VM                            |
| Strength     | Explore, query, ad-hoc ops       | Automate, verify, serve                   |
| Example      | `pods \| where status == "Fail"` | `http.get(url); assert.eq(r.status, 200)` |

### Migration Path

```
TODAY (K8s + ArgoCD):
+-----------------------------------------------------------+
| ArgoCD PostSync Job                                       |
| +-- assay container (~12 MB image)                        |
| +-- Mounts ConfigMap with checks.yaml + Lua scripts       |
| +-- Runs Lua scripts via embedded Lua VM                  |
+-----------------------------------------------------------+

FUTURE (Rubernetes native):
+-----------------------------------------------------------+
| Rubernetes GitOps controller (in-process)                 |
| +-- Same Lua scripts, same builtins, no container needed  |
| +-- <1ms startup, zero pod overhead                       |
| +-- Plus k8s.* builtins (direct API server access)        |
+-----------------------------------------------------------+

MIGRATION: Copy .lua files. Done.
```

## Testing Strategy

### The Question: Test Assay with Assay?

Assay is a testing/verification tool. Using it to test itself is legitimate dogfooding — like Go's
test framework testing Go's standard library. But it cannot be the ONLY testing layer.

### Three-Layer Testing

```
+------------------------------------------------------------------+
| Layer 1: Rust Unit Tests (cargo test)                            |
|                                                                  |
| What: Individual Rust functions, parser logic, error handling    |
| How: #[test] functions in each module                            |
| Coverage: Config parsing, output formatting, CLI args, sandbox   |
| Runs: Every commit (CI)                                          |
+------------------------------------------------------------------+
| Layer 2: Rust Integration Tests (cargo test --test '*')          |
|                                                                  |
| What: Lua builtins executed in a real Lua VM                     |
| How: tests/ directory with Rust test harness                     |
| Coverage: Every Lua builtin function, edge cases, error paths   |
|   - HTTP: mock server (wiremock-rs) + real requests              |
|   - Database: SQLite in-memory or testcontainers                 |
|   - Crypto: Known test vectors (RFC 7515 for JWT)                |
|   - Server: Start axum, send requests, verify responses          |
|   - Sandbox: Verify restricted functions are blocked             |
| Runs: Every commit (CI)                                          |
+------------------------------------------------------------------+
| Layer 3: E2E / Dogfood Tests (assay check tests/e2e.yaml)       |
|                                                                  |
| What: Assay testing itself via its own check mode                |
| How: YAML + Lua test scripts in tests/e2e/                      |
| Coverage: Full pipeline (config parse -> run -> output -> exit)  |
|   - Run assay as subprocess, verify JSON output                  |
|   - Test retry/backoff behavior with a flaky mock server         |
|   - Test all three modes (check, run, serve)                     |
|   - Test sandbox enforcement (expect failures)                   |
| Runs: Every release (CI, after cargo test passes)                |
+------------------------------------------------------------------+
```

### Test Infrastructure

| Component  | Tool               | Purpose                                     |
| ---------- | ------------------ | ------------------------------------------- |
| Unit       | cargo test         | Rust function tests                         |
| Mocks      | wiremock-rs        | HTTP mock server for builtin tests          |
| Database   | SQLite in-memory   | Database builtin tests (no Postgres needed) |
| Containers | testcontainers     | Optional: real Postgres for integration     |
| E2E        | assay itself       | Dogfood testing (meta but useful)           |
| CI         | GitHub Actions     | Run all layers on every PR                  |
| Lint       | clippy -D warnings | Zero warnings policy                        |
| Format     | dprint             | Markdown, YAML, JSON, TOML                  |

### Test Counts

| Layer       | Current | Target | When                                       |
| ----------- | :-----: | :----: | ------------------------------------------ |
| Unit        |   26    |  ~40   | v0.1.0 (11 lib + 15 main)                  |
| Integration |  ~464   |  ~470  | v0.1.0 (grow with each builtin/stdlib)     |
| E2E         |    1    |   ~5   | v0.1.0 (1 test validates 8 builtin checks) |
| **Total**   | **491** |        | All passing, 0 clippy warnings             |

## What Our K8s Jobs Currently Do

Analysis of all shell scripts and Jobs in jeebon gitops:

| Job                 | What It Does                          | Image           | Could Be Lua?                      |
| ------------------- | ------------------------------------- | --------------- | ---------------------------------- |
| openbao-bootstrap   | Init Bao, create secrets, policies    | openbao/openbao | Yes: HTTP, base64, JSON, file read |
| postgres-bootstrap  | Generate password, store in Bao       | openbao/openbao | Yes: HTTP, base64, JSON, random    |
| redis-bootstrap     | Same pattern for Redis                | openbao/openbao | Yes: same builtins                 |
| mariadb-bootstrap   | Same pattern for MariaDB              | openbao/openbao | Yes: same builtins                 |
| argocd-rbac-sync    | Parse emails, patch ConfigMap         | bitnami/kubectl | Yes: HTTP (K8s API), base64, JSON  |
| kargo-rbac-sync     | Same pattern                          | bitnami/kubectl | Yes: same builtins                 |
| 7x postsync-verify  | Verification checks                   | assay:v0.2.0    | Already Lua                        |
| config-verification | Kargo pipeline verification           | alpine/k8s      | Yes: HTTP                          |
| zitadel-configure   | JWT auth, Admin API, store OIDC creds | TBD (Plan 21)   | Yes: JWT, HTTP, JSON, file read    |
| content-layer       | OAuth app registration (future)       | TBD             | Yes: JWT, HTTP, JSON               |

Key pattern: Almost every Job does HTTP calls + JSON + base64 + file read. They use heavyweight
images (openbao:2.5.0 at ~150 MB, bitnami/kubectl at ~90 MB, alpine/k8s at ~150 MB) for work that
Assay handles in 15 MB.

## Use Cases

### UC-1: ArgoCD Hook Jobs (Current)

PreSync and PostSync Jobs that bootstrap, configure, and verify services during ArgoCD syncs.

Requirements: HTTP client, JSON, base64, file read, assert, env vars, structured output,
retry/backoff, exit codes.

### UC-2: Zitadel Auth Configuration (Immediate — Plan 21)

PostSync Job that authenticates to Zitadel using JWT RS256, then configures Google IdP, org domain,
projects, and OIDC apps via Admin API. Stores resulting OIDC credentials back in OpenBao.

Requirements: All of UC-1 plus JWT signing (RS256 with PEM key), file read (machine key), multi-step
API orchestration.

### UC-3: Platform Maintenance (Near-term)

Ad-hoc Jobs for operational tasks: rotate secrets, verify cross-service connectivity, generate
reports, run database health checks.

Requirements: UC-1 + UC-2 plus database access (SQL queries), YAML generation.

### UC-4: Lightweight Web Services (Near-term)

Replace Python/Node.js containers with Lua scripts for simple web services: webhook receivers, API
proxies, mock servers, health dashboards.

Requirements: HTTP server (axum), WebSocket, database, templates, routing, middleware.

### UC-5: User-Accessible Runtime (Future — Rubernetes)

Offering platform users a lightweight Lua runtime to run small utilities inside Kubernetes (or
Rubernetes) without needing external container images. Think: cron jobs, webhooks, data transforms,
API integrations.

Requirements: Full runtime + sandboxing (untrusted user code) + resource limits.

## Key Design Decisions

### D1: Single Binary (No Light/Full Split)

Binary delta between "light" (no server/db) and "full" is ~2.4 MB. Not worth splitting:

- Two Docker images = double CI, double confusion
- Sandbox is Lua-level (which builtins are registered), not binary-level
- Users shouldn't have to choose an image variant

Decision: One binary, one Docker image. All features compiled in. Modes control exposure.

### D2: Lua 5.5 (Not LuaJIT)

Lua 5.5.0 (released 22 Dec 2025) over 5.4 and LuaJIT. Key 5.5 improvements:

- Declarations for global variables (catches accidental globals — reduces bugs)
- Named vararg tables (cleaner function signatures)
- More compact arrays (less memory)
- Major GC done incrementally (smoother latency for long-running `http.serve()` scripts)

Our scripts are I/O bound (HTTP calls, sleep between retries). CPU-bound Lua execution is <1% of
total Job time. LuaJIT's 5-10x speedup on CPU ops gives near-zero benefit.

LuaJIT disadvantages:

- Lua 5.1 only (missing 5.5 features: native int64, goto, global declarations, incremental major GC)
- 4GB memory ceiling (32-bit pointers internally)
- Maintenance concerns (Mike Pall stepped down)
- MUSL static linking issues with LuaJIT's assembler

Decision: Lua 5.5 default. mlua supports LuaJIT via cargo feature flag if ever needed.

### D3: Sandbox Architecture

"Sandbox" means controlled access, not no access:

- `.yaml` checks: Only http, json, assert, log, env, sleep, time, base64 exposed. No fs, no db, no
  server. Fresh VM per check.
- `.lua` scripts: All builtins available. 64 MB memory limit.
- `--sandbox` flag: Restricts builtins to check-level (future: untrusted user code in Rubernetes).

No separate "serve mode" — a script that calls `http.serve()` is just a long-running Lua script with
all builtins available. The sandbox is a flag, not a mode.

### D4: Hybrid Builtin Architecture

- Core builtins in Rust: http, json, assert, crypto, fs, db, server (performance + safety critical)
- 19 stdlib modules as embedded Lua: monitoring (prometheus, alertmanager, loki, grafana),
  k8s/gitops (k8s, argocd, kargo, flux, traefik), security (vault/openbao, certmanager, eso, dex),
  infra (crossplane, velero, temporal, harbor), utilities (healthcheck)
- Lua stdlib embedded in binary via `include_dir!` (no external files)
- Users can `require("assay.prometheus")` etc.

### D5: What We Learned from Astra

Adopted:

- Lua stdlib file pattern (Lua wrappers over Rust builtins, embedded in binary)
- `require()` system for module loading
- Type definition files (.d.lua) for IDE support (future)

Rejected:

- `unsafe_new()` (keep our sandbox)
- Global shared VM (keep fresh-per-check isolation)
- 30+ dependency tree (keep deps minimal)
- Pre-1.0 instability (we control our release cycle)

## Options Analysis (Historical Record)

The following options were evaluated before deciding on Option A (evolve incrementally):

### Option A: Evolve Assay Incrementally (CHOSEN)

Rationale:

1. Assay already runs 7 verification Jobs — proven foundation
2. Sandbox architecture is a strategic advantage (enables user code in Rubernetes)
3. Binary stays small (~8 MB vs Astra's 30-50 MB)
4. We control the roadmap and release cycle
5. Astra's best ideas (Lua stdlib pattern) adopted without forking
6. Full feature set adds only ~2.4 MB over baseline

### Option B: Fork Astra (REJECTED)

Rejected because:

- HIGH effort to rearchitect security model (unsafe_new is fundamental)
- Would strip 60% of code then add 40% of our own — net rewrite
- Upstream instability (pre-1.0, 330 commits in 8 months)
- Fork maintenance burden exceeds building from scratch

### Option C: New Project (REJECTED)

Rejected because:

- Most effort — rewriting what already works
- No production track record
- 7 existing Jobs need migration for zero benefit

### Option D: Use Astra As-Is (REJECTED)

Rejected because:

- No sandbox — blocks user code use case
- No structured output, retry/backoff — must reimplement in Lua
- 30-50 MB container image
- Subject to upstream breaking changes

## Implementation Roadmap

All steps target **v0.1.0** — the first feature-complete release. Current state is tagged v0.0.1.

### Step 1 — Core Builtins (Plan 21 Unblock)

**Goal**: Add builtins needed for Zitadel auth configuration. **AI agent time**: ~3 hours

Scope:

- Add Rust builtins: `fs.read`, `crypto.jwt_sign` (RS256/384/512), `http.delete`,
  `base64.encode/decode`
- Keep native check types: `type: http`, `type: prometheus`, `type: script` (batteries-included DX
  for common patterns; Lua scripts for complex cases)
- Add Lua stdlib system (embedded .lua files via `include_dir!`)
- Ship `stdlib/prometheus.lua` (Lua-side Prometheus client for `type: script` checks)
- DRY http builtins (collapsed 4x duplicated methods into generic loop)
- Add Rust unit tests (base64, JSON conversion, value equality, string escaping)
- Add Rust integration tests with wiremock (HTTP methods, JWT sign+verify, fs.read, base64, stdlib
  require, env, assert, json, time/sleep)
- Dependencies: +jsonwebtoken 10.3 (rust_crypto), +zeroize 1.8, +data-encoding 2.10, +include_dir
  0.7
- Dev dependencies: +wiremock 0.6, +tokio-test 0.4
- Add `src/lib.rs` for integration test access to `lua` module

### Step 2 — Foundation

**Goal**: Complete crypto, add regex, ship comprehensive stdlib. **AI agent time**: ~3 hours

Scope:

- Add builtins: `crypto.hash` (SHA2/SHA3), `crypto.random` (secure random strings), `regex`
  (match/find/replace via regex-lite)
- Ship 19 stdlib modules (embedded Lua, all using `require("assay.*")`):
  - **Monitoring**: prometheus (query, alerts, targets, rules), alertmanager (alerts, silences,
    receivers), loki (push, query, labels, series), grafana (health, dashboards, datasources)
  - **K8s/GitOps**: k8s (30+ resource types, CRDs, readiness), argocd (apps, sync, health,
    projects), kargo (stages, freight, promotions), flux (git repos, kustomizations, helm releases),
    traefik (routers, services, middlewares, entrypoints)
  - **Security**: vault/openbao (KV, policies, auth, transit, PKI), certmanager (certificates,
    issuers, ACME), eso (external secrets, secret stores), dex (OIDC discovery, JWKS, health)
  - **Infrastructure**: crossplane (providers, XRDs, compositions, managed resources), velero
    (backups, restores, schedules, storage locations), temporal (workflows, task queues, schedules),
    harbor (projects, repositories, artifacts, vulnerability scanning)
  - **Utilities**: healthcheck (HTTP checks, JSON path, body matching, latency, multi-check)
- Dependencies: +sha2, +sha3, +rand, +regex-lite
- Target binary: ~5.5 MB

### Step 3 — General Purpose + Direct Lua Execution ✅

**Goal**: Serde completeness + async + fs.write + `assay script.lua` support with shebang. **AI
agent time**: ~4.5 hours

Completed (commit TBD):

- Added builtins: `fs.write`, `yaml.parse/encode`, `toml.parse/encode`, `async.spawn/spawn_interval`
- Added direct .lua execution: `assay script.lua` (auto-detect by file extension, positional arg)
- CLI changed from `assay --config file.yaml` to `assay <file>` (auto-detect .yaml/.yml/.lua)
- Shebang support: `#!/usr/bin/assay` (Lua 5.5 natively skips `#!` lines — zero code needed)
- async.spawn uses `tokio::task::spawn_local` + `LocalSet` (Lua values are !Send)
- async.spawn returns handle with `.await()` method; spawn_interval returns handle with `.cancel()`
- 33 new tests (fs.write: 4, yaml: 9, toml: 8, async: 9, plus 3 existing fs tests kept)
- Dependencies: +toml 0.9.12 (serde_yml already existed)
- Total: 449 tests, 0 failures, 0 clippy warnings

### Step 4 — HTTP Server Builtin ✅

**Goal**: Add `http.serve()` so Lua scripts can be web services. **AI agent time**: ~9 hours

Completed:

- Added builtin: `http.serve(port, routes)` — scripts call this to become a web service
- Routing with method tables (GET, POST, PUT, PATCH, DELETE), Lua handler functions
- Graceful shutdown on SIGTERM (K8s pod lifecycle)
- No special "serve mode" — just a script that calls `http.serve()` and blocks
- 8 new tests (server start/stop, routing, methods, request/response handling)
- Dependencies: +axum 0.8.8, +hyper 1, +hyper-util 0.1, +tower-http 0.6.8, +http-body-util 0.1
- Total: 457 tests, 0 failures, 0 clippy warnings

```lua
#!/usr/bin/assay
-- This script IS the web service. No special mode needed.
http.serve(8080, {
  GET = {
    ["/health"] = function(req) return { status = 200, body = "ok" } end,
    ["/api/users"] = function(req)
      local rows = db.query(pg, "SELECT * FROM users")
      return { status = 200, json = rows }
    end,
  }
})
```

### Step 5 — Database ✅

**Goal**: SQL database access for Lua scripts (all three backends). **AI agent time**: ~6 hours

Completed:

- Added builtins: `db.connect(url)`, `db.query(conn, sql, params)`, `db.execute(conn, sql, params)`
- Connection pooling via sqlx AnyPool
- Three backends: PostgreSQL, MySQL/MariaDB, SQLite (embedded)
- URL scheme selects backend: `postgres://`, `mysql://`, `sqlite://`
- 10 new tests (connect, query, execute, parameterized queries, error handling)
- Dependencies: +sqlx 0.8.6 (postgres, mysql, sqlite, runtime-tokio-rustls, any)
- Total: 467 tests, 0 failures, 0 clippy warnings

```lua
-- PostgreSQL (jeebon primary DB)
local pg = db.connect("postgres://user:pass@postgres.database.svc:5432/jeebon")
local rows = db.query(pg, "SELECT count(*) as n FROM users")

-- MariaDB (Seafile backend)
local maria = db.connect("mysql://user:pass@mariadb.database.svc:3306/seafile")
local tables = db.query(maria, "SHOW TABLES")

-- SQLite (embedded, no server needed)
local lite = db.connect("sqlite:///tmp/state.db")
db.execute(lite, "CREATE TABLE IF NOT EXISTS cache (key TEXT, value TEXT)")
```

### Step 6 — WebSocket + Templates ✅

**Goal**: Complete the feature set. **AI agent time**: ~5 hours

Completed:

- Added builtins: `ws.connect(url)`, `ws.send(conn, msg)`, `ws.recv(conn)`, `ws.close(conn)`,
  `template.render(path, vars)`, `template.render_string(tmpl, vars)`
- WebSocket client via tokio-tungstenite (connect to external services)
- Jinja2-compatible templates via minijinja (loops, conditionals, filters, nested objects)
- 17 new tests (ws: connect/send/recv/close, echo server, error handling; template: render_string,
  render from file, filters, loops, conditionals, nested objects, edge cases)
- Dependencies: +tokio-tungstenite 0.28.0, +futures-util 0.3, +minijinja 2.15.1
- Total: 490 tests, 0 failures, 0 clippy warnings

### Step 7 — E2E + Polish ✅

**Goal**: Dogfood testing (Assay testing itself via YAML check mode). **AI agent time**: ~1 hour

Completed:

- E2E test suite: `tests/e2e/builtins-check.yaml` with 8 Lua check scripts testing all sync builtins
  (json, yaml, toml, fs, crypto, regex, base64, template)
- Integration test `tests/e2e_tests.rs` runs the E2E suite by invoking the `assay` binary as a
  subprocess and validating the JSON output
- Validates the full pipeline: CLI → config parse → runner → VM creation → script execution →
  structured JSON output → exit code
- 1 new E2E integration test (validates 8 checks pass)
- Total: 491 tests, 0 failures, 0 clippy warnings

### Step 8 — Stable Release (v0.1.0) ✅

**Goal**: Stable API, production-ready, first feature-complete release. **AI agent time**: ~3 hours

Completed:

- Version bumped to 0.1.0 in Cargo.toml
- README.md written with feature overview, quickstart, API reference, examples
- CHANGELOG.md created with v0.1.0 and v0.0.1 entries
- CI updated with macOS build matrix (`macos-14` Apple Silicon runner)
- Final audit: clippy clean, all tests green, dprint formatted
- Target binary: ~9 MB

### Timeline Summary

| Step      | Features                          | Agent Time | Status |
| --------- | --------------------------------- | :--------: | :----: |
| Step 1    | P0 builtins, stdlib system        |  4.5 hrs   |   ✅   |
| Step 2    | Crypto, regex, stdlib helpers     |   3 hrs    |   ✅   |
| Step 3    | Serde, async, fs.write, .lua exec |  4.5 hrs   |   ✅   |
| Step 4    | http.serve() builtin              |   9 hrs    |   ✅   |
| Step 5    | Database (Postgres/MySQL/SQLite)  |   6 hrs    |   ✅   |
| Step 6    | WebSocket, templates              |   5 hrs    |   ✅   |
| Step 7    | E2E tests, dogfood                |    1 hr    |   ✅   |
| Step 8    | Stable release (v0.1.0)           |   3 hrs    |   ✅   |
| **Total** |                                   | **36 hrs** |        |
