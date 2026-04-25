# 12e — Phases 8 + 9 + 10 — Engine binary, CI, and ship

> Sub-plan of [12-v0.13.0-execution.md](./12-v0.13.0-execution.md). Prerequisites: Phases 3, 5, 6, 7
> complete.

## v0.1.2 alignment

Module enablement is now driven by `engine.modules`, not just compile features or static config.
Phase 8 boot sequence:

1. Open engine storage; CREATE SCHEMA / open file for `engine`; run engine schema migrations.
2. `SELECT name, version, config FROM engine.modules WHERE enabled = TRUE`.
3. For each enabled module: PG `CREATE SCHEMA IF NOT EXISTS <m>` / SQLite
   `ATTACH DATABASE
   'data/<m>.db' AS <m>`; run pending migrations recorded in
   `engine.migrations`.
4. Wire trait routing per module; mount HTTP routes; start scheduler/workers.

When auth is added to `engine.modules` (insert row with
`name='auth', enabled=TRUE,
version='0.1.0'`), the auth schema is created/attached on next boot.
Dashboard panes (workflow, auth) render conditionally based on enabled modules read from
`engine.modules` at startup. See [14-v0.13.2-engine-schemas.md](./14-v0.13.2-engine-schemas.md) for
storage model details and [12c §"v0.1.2 alignment"](./12c-phase-4-6-auth-identity-zanzibar.md) for
auth-specific deltas.

---

**Phase 8 goal:** `assay-engine` is a runnable binary that loads a config, connects to a backend,
runs migrations, composes module routers via `FromRef`, and serves workflow + auth + dashboard on
one port. Runtime binary's dashboard is restored (via engine's composition helper).

**Phase 9 goal:** CI publishes per-crate tags with no manual work. Tests run against a PostgreSQL 18
service container and in-process SQLite. Docker images ship for both binaries.

**Phase 10 goal:** Docs cover the runtime/engine split, CHANGELOGs are complete, migration notes
help 0.12 consumers land on 0.13, and v0.13.0 tags are pushed.

---

## Phase 8 — Engine binary + dashboard

### Task 8.1: `EngineConfig` full schema

**Files:** `crates/assay-engine/src/config.rs`.

- [ ] **Step 1: TOML schema**

```toml
# Example config — engine.toml
[server]
bind_addr = "0.0.0.0:3000"
public_url = "https://auth.example.com" # used for issuer in OIDC provider

[backend]
type = "postgres" # "postgres" | "sqlite"
url = "postgres://postgres:postgres@localhost/assay"
# For sqlite: path = "/var/lib/assay/engine.db"

[workflow]
enabled = true
default_task_queue = "main"

[auth]
enabled = true

[auth.oidc_provider]
enabled = true
issuer_override = "https://auth.example.com"
key_rotation_interval_days = 30

[auth.session]
ttl_seconds = 604800 # 7 days
cookie_secure = true
cookie_domain = "example.com"

[dashboard]
enabled = true
product_name = "Assay"
logo_url = "https://example.com/logo.svg"

[logging]
level = "info"
format = "json" # "json" | "pretty"

[[migrate]]
run_on_startup = true
```

- [ ] **Step 2: Rust types**

```rust
#[derive(Clone, Debug, Deserialize)]
pub struct EngineConfig {
    pub server: ServerConfig,
    pub backend: Backend,
    #[serde(default)] pub workflow: WorkflowConfig,
    #[serde(default)] pub auth: AuthConfig,
    #[serde(default)] pub dashboard: DashboardConfig,
    #[serde(default)] pub logging: LoggingConfig,
    #[serde(default)] pub migrate: MigrateConfig,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ServerConfig {
    pub bind_addr: String,
    pub public_url: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Backend {
    Postgres { url: String },
    Sqlite   { path: String },
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct WorkflowConfig {
    #[serde(default = "default_true")] pub enabled: bool,
    pub default_task_queue: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct AuthConfig {
    #[serde(default = "default_true")] pub enabled: bool,
    #[serde(default)] pub oidc_provider: OidcProviderConfig,
    #[serde(default)] pub session: SessionConfig,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct OidcProviderConfig {
    #[serde(default = "default_true")] pub enabled: bool,
    pub issuer_override: Option<String>,
    #[serde(default = "default_key_rotation")] pub key_rotation_interval_days: u32,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SessionConfig {
    #[serde(default = "default_session_ttl")] pub ttl_seconds: u64,
    #[serde(default = "default_true")]        pub cookie_secure: bool,
    pub cookie_domain: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct DashboardConfig {
    #[serde(default = "default_true")] pub enabled: bool,
    #[serde(default = "default_name")] pub product_name: String,
    pub logo_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]  pub level: String,
    #[serde(default = "default_log_format")] pub format: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct MigrateConfig {
    #[serde(default = "default_true")] pub run_on_startup: bool,
}

// Default helper functions
fn default_true() -> bool { true }
fn default_key_rotation() -> u32 { 30 }
fn default_session_ttl() -> u64 { 604800 }
fn default_name() -> String { "Assay".into() }
fn default_log_level() -> String { "info".into() }
fn default_log_format() -> String { "pretty".into() }

impl EngineConfig {
    pub fn from_file(path: &std::path::Path) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&raw)?)
    }

    pub fn from_env_overrides(mut self) -> Self {
        if let Ok(v) = std::env::var("ASSAY_ENGINE_BIND_ADDR") { self.server.bind_addr = v; }
        if let Ok(v) = std::env::var("ASSAY_ENGINE_PUBLIC_URL") { self.server.public_url = v; }
        // ... selective env overrides
        self
    }
}
```

- [ ] **Step 2: Example configs in `crates/assay-engine/examples/`**

`sqlite.toml`, `postgres.toml` — minimum viable configs for each backend.

- [ ] **Step 3: Tests**

- Parse each example; assert parses cleanly.
- Missing required field → clear error pointing at the file + field.

- [ ] **Step 4: Commit** — `feat(engine): full EngineConfig schema + examples`.

---

### Task 8.2: `EngineState` with `FromRef` composition

**Files:** `crates/assay-engine/src/state.rs`.

This is the culmination of Phase 1's architectural work. `EngineState` owns the three (or more)
module contexts. `axum::extract::FromRef` derives the sub-state extractors.

> **Revision note (2026-04-21):** The original example below showed a non-generic `EngineState`.
> Reality: `WorkflowCtx` is generic on `S: WorkflowStore` (trait has RPITIT, not dyn-compatible —
> see plan 12 Architecture Principle 2). So `EngineState<S>` is also generic, and the engine's
> `main()` picks the concrete `S` via a match on `cfg.backend`. Each backend compiles as a separate
> monomorphisation; runtime cost zero, binary gains ~20-40 KB per backend.

- [ ] **Step 1: EngineState**

```rust
use axum::extract::FromRef;
use assay_domain::WorkflowStore;
use std::sync::Arc;

#[derive(Clone)]
pub struct EngineState<S: WorkflowStore> {
    #[cfg(feature = "workflow")]
    pub workflow: Arc<assay_workflow::WorkflowCtx<S>>,

    #[cfg(feature = "auth")]
    pub auth: Arc<assay_auth::AuthCtx>,

    #[cfg(feature = "dashboard")]
    pub dashboard: Arc<assay_dashboard::DashboardCtx>,
}

#[cfg(feature = "workflow")]
impl<S: WorkflowStore> FromRef<EngineState<S>> for Arc<assay_workflow::WorkflowCtx<S>> {
    fn from_ref(s: &EngineState<S>) -> Self { Arc::clone(&s.workflow) }
}

#[cfg(feature = "auth")]
impl<S: WorkflowStore> FromRef<EngineState<S>> for Arc<assay_auth::AuthCtx> {
    fn from_ref(s: &EngineState<S>) -> Self { Arc::clone(&s.auth) }
}

#[cfg(feature = "dashboard")]
impl<S: WorkflowStore> FromRef<EngineState<S>> for Arc<assay_dashboard::DashboardCtx> {
    fn from_ref(s: &EngineState<S>) -> Self { Arc::clone(&s.dashboard) }
}
```

**Engine main() backend selection (added):**

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = EngineConfig::from_file(&cli.config)?;
    match cfg.backend.clone() {
        #[cfg(feature = "backend-postgres")]
        Backend::Postgres { .. } => run_engine::<assay_workflow::PostgresStore>(cfg).await,
        #[cfg(feature = "backend-sqlite")]
        Backend::Sqlite   { .. } => run_engine::<assay_workflow::SqliteStore>(cfg).await,
        #[allow(unreachable_patterns)]
        _ => anyhow::bail!("backend feature not compiled in"),
    }
}

async fn run_engine<S: WorkflowStore + 'static>(cfg: EngineConfig) -> anyhow::Result<()> {
    let state: EngineState<S> = build_state(&cfg).await?;
    server::run(cfg, state).await
}
```

- [ ] **Step 2: Build function**

```rust
pub async fn build(cfg: &EngineConfig) -> anyhow::Result<EngineState> {
    let (wf_ctx, auth_ctx) = match &cfg.backend {
        #[cfg(feature = "backend-postgres")]
        Backend::Postgres { url } => {
            let pg = sqlx::PgPool::connect(url).await?;
            if cfg.migrate.run_on_startup {
                assay_workflow::migrate::postgres(&pg).await?;
                #[cfg(feature = "auth")]
                assay_auth::migrate::postgres(&pg).await?;
            }
            let wf = assay_workflow::WorkflowCtx::from_pg_pool(pg.clone(), cfg.workflow.clone())?;
            #[cfg(feature = "auth")]
            let auth = assay_auth::AuthCtx::from_pg_pool(pg, cfg)?;
            (wf, #[cfg(feature = "auth")] auth)
        }
        #[cfg(feature = "backend-sqlite")]
        Backend::Sqlite { path } => { /* parallel */ todo!() }
        #[allow(unreachable_patterns)] _ => anyhow::bail!("backend compiled out"),
    };

    #[cfg(feature = "dashboard")]
    let dashboard = assay_dashboard::DashboardCtx {
        branding: assay_dashboard::Branding {
            product_name: cfg.dashboard.product_name.clone(),
            logo_url: cfg.dashboard.logo_url.clone(),
        },
    };

    Ok(EngineState {
        #[cfg(feature = "workflow")] workflow: wf_ctx,
        #[cfg(feature = "auth")]     auth: auth_ctx,
        #[cfg(feature = "dashboard")] dashboard,
    })
}
```

- [ ] **Step 3: `from_pg_pool` constructors in each module**

Add to `WorkflowCtx` and `AuthCtx`:

```rust
impl WorkflowCtx {
    pub fn from_pg_pool(pool: PgPool, cfg: WorkflowConfig) -> anyhow::Result<Self> {
        let store = Arc::new(assay_workflow::PostgresStore::new(pool));
        let engine = Arc::new(Engine::new(store.clone(), /* options from cfg */));
        let (event_tx, _) = tokio::sync::broadcast::channel(1024);
        Ok(Self { store, engine, event_tx, /* etc */ })
    }
}
```

- [ ] **Step 4: Tests**

- Build `EngineState` against SQLite from a minimal config, assert each sub-context extracts via
  `FromRef`.
- Per-feature build matrix:
  - `--no-default-features --features "workflow server backend-sqlite"` → `EngineState` has only
    workflow.
  - `--no-default-features --features "auth server backend-postgres"` → only auth.
  - defaults → all three.

- [ ] **Step 5: Commit** — `feat(engine): EngineState with FromRef composition`.

---

### Task 8.3: Binary entrypoint + CLI

**Files:** `crates/assay-engine/src/bin/assay-engine.rs`, `crates/assay-engine/src/server.rs`,
`crates/assay-engine/src/cli.rs`.

- [ ] **Step 1: CLI**

```rust
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(version, about = "assay-engine — workflow + auth HTTP server")]
struct Cli {
    #[arg(short, long, env = "ASSAY_ENGINE_CONFIG", default_value = "assay-engine.toml")]
    config: std::path::PathBuf,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Start the HTTP server (default action if no subcommand given).
    Serve,

    /// Run pending migrations, then exit.
    Migrate,

    /// Admin actions.
    Admin {
        #[command(subcommand)]
        action: AdminAction,
    },
}

#[derive(Subcommand)]
enum AdminAction {
    ClientCreate { name: String, #[arg(long)] redirect: Vec<String> },
    ClientList,
    ClientDelete { client_id: String },
    FederationAdd { slug: String, #[arg(long)] issuer: String,
                    #[arg(long)] client_id: String, #[arg(long)] client_secret: String },
    FederationList,
    KeyRotate,
}
```

- [ ] **Step 2: main()**

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let cfg = EngineConfig::from_file(&cli.config)?.from_env_overrides();
    init_tracing(&cfg.logging);

    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => {
            let state = state::build(&cfg).await?;
            server::run(cfg, state).await
        }
        Command::Migrate => {
            run_migrations(&cfg).await
        }
        Command::Admin { action } => {
            admin_cli::run(cfg, action).await
        }
    }
}
```

- [ ] **Step 3: Server wiring**

```rust
// crates/assay-engine/src/server.rs
use axum::Router;
use crate::state::EngineState;

pub async fn run(cfg: EngineConfig, state: EngineState) -> anyhow::Result<()> {
    let mut app = Router::new();

    #[cfg(feature = "workflow")]
    { app = app.merge(assay_workflow::router()); }

    #[cfg(feature = "auth")]
    { app = app.merge(assay_auth::router()); }

    #[cfg(feature = "dashboard")]
    { app = app.merge(assay_dashboard::router()); }

    let app = app
        .route("/healthz", axum::routing::get(|| async { "ok" }))
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&cfg.server.bind_addr).await?;
    tracing::info!(addr = %cfg.server.bind_addr, "assay-engine listening");

    // Spawn key rotation task if OIDC provider is enabled
    #[cfg(feature = "auth")]
    if cfg.auth.oidc_provider.enabled {
        assay_auth::oidc_provider::spawn_key_rotation(/* ctx.auth.oidc_provider */);
    }

    axum::serve(listener, app).await.map_err(Into::into)
}
```

- [ ] **Step 4: Smoke e2e test**

`crates/assay-engine/tests/e2e/smoke.rs` — start the binary in a subprocess against a SQLite config,
hit `/healthz`, hit `/api/v1/workflows`, hit `/.well-known/openid-configuration`, assert all three
succeed.

- [ ] **Step 5: Commit** — `feat(engine): binary entrypoint + serve + migrate + admin CLI`.

---

### Task 8.4: Dashboard engine views

**Files:** new views under `crates/assay-dashboard/assets/engine/` + handlers in `router.rs`.

Plan 10 mandated queue stats + worker registry in the dashboard. Plan 11's admin views (client
registry, user list, session browser, Zanzibar tuple browser) land behind the `auth` feature.

- [ ] **Step 1: Engine workflow views**

- `/engine/queues` — table of queues with pending / running / worker counts from
  `WorkflowStore::get_queue_stats`.
- `/engine/workers` — registered workers with heartbeat age, task queues, namespace.
- `/engine/namespaces` — namespaces with stats (running, pending, completed, failed).

- [ ] **Step 2: Engine auth views (feature=auth)**

- `/engine/clients` — OIDC client list with create/edit/delete buttons.
- `/engine/federation` — upstream providers.
- `/engine/users` — user search + detail.
- `/engine/sessions` — active sessions with revoke button.
- `/engine/zanzibar/tuples` — browse + check.
- `/engine/zanzibar/check` — interactive check playground.

- [ ] **Step 3: Navigation**

Side nav with sections: Workflow · Auth · Engine. Link groups expand based on enabled features.

- [ ] **Step 4: Tests**

Expand the existing Playwright suite (`crates/assay-workflow/tests-e2e/` →
`crates/assay-dashboard/tests-e2e/`) with engine-view assertions. Add an `engine` e2e scenario that
logs in as admin, navigates each engine view, asserts expected table headers and at least one row.

- [ ] **Step 5: Commit** —
      `feat(dashboard): engine views (queues, workers, clients, sessions, Zanzibar)`.

---

### Task 8.5 — REMOVED (runtime dashboard retired in v0.13.0)

> **Decision (2026-04-21):** drop the runtime dashboard permanently instead of restoring it. The
> 0.12 story where `assay serve --workflow` also served the dashboard was a legacy carry-over; plan
> 10 § "Deployment shapes" always split it as Shape A (scripting, no HTTP) vs Shape B (server,
> HTTP). v0.13.0 is the right moment to commit to that split.
>
> **Effect:**
>
> - `crates/assay/Cargo.toml` drops its dashboard / axum-router wiring. Keep `http.serve()` Lua
>   stdlib (different feature — scripts exposing small HTTP endpoints).
> - Users who want a dashboard run `assay-engine` instead.
> - Plan 10 § "Shape A" is already accurate — runtime has no dashboard.
> - CHANGELOG entry + migration notes must call out: 0.12 `assay serve --workflow` → 0.13
>   `assay-engine --config <backend>.toml`.
>
> Saves ~1h on execution and keeps the product surface cleaner.

---

### Phase 8 exit criteria

- `assay-engine` binary serves on the configured port against any of three backends.
- `assay-engine migrate` runs migrations then exits; `assay-engine admin client create` works.
- Dashboard renders engine views for queues, workers, clients, sessions, Zanzibar tuples.
- Runtime binary's `assay serve` command restores the dashboard.
- All e2e tests (workflow + dashboard) green.
- `cargo test --workspace` green.

---

## Phase 9 — CI + release tooling

### Task 9.1: Per-crate moon.yml release tasks

**Files:**
`crates/{assay,assay-domain,assay-workflow,assay-dashboard,assay-engine,assay-auth}/moon.yml`.

- [ ] **Step 1: Template** — see [plan 12 main doc](./12-v0.13.0-execution.md#release-gate-flow) for
      the task shape.

Add to each crate's moon.yml:

```yaml
tasks:
  release:
    command: |
      bash -c '
        set -euo pipefail
        CRATE="$(basename "$(pwd)")"
        VERSION=$(cargo metadata --format-version 1 --no-deps \
          | jq -r ".packages[] | select(.name==\"$CRATE\") | .version")
        STATUS=$(curl -sS -o /dev/null -w "%{http_code}" \
          "https://crates.io/api/v1/crates/$CRATE/$VERSION")
        if [ "$STATUS" = "200" ]; then
          echo "skip: $CRATE@$VERSION on crates.io"
          exit 0
        fi
        cargo publish -p "$CRATE" --allow-dirty
      '
    inputs:
      - "src/**/*"
      - "Cargo.toml"
      - "../../Cargo.toml"
    options:
      cache: false
      runInCI: false # triggered by release workflow only
```

For `assay`, the crate name is `assay-lua`, not matching the dir — override with an explicit
`CRATE="assay-lua"` line.

- [ ] **Step 2: Commit** — `ci(moon): per-crate release task templates`.

---

### Task 9.2: CI test matrix — PG18 + SQLite

**Files:** `.github/workflows/ci.yml`.

- [ ] **Step 1: Add test job variants**

```yaml
test-workflow-pg:
  runs-on: ubuntu-latest
  services:
    postgres:
      image: postgres:18-alpine
      env: { POSTGRES_PASSWORD: postgres }
      ports: ["5432:5432"]
      options: >-
        --health-cmd pg_isready --health-interval 10s
  steps:
    - uses: actions/checkout@v4
    - uses: dtolnay/rust-toolchain@stable
    - uses: Swatinem/rust-cache@v2
    - run: cargo test -p assay-workflow --features backend-postgres --test '*'
      env: { DATABASE_URL: postgres://postgres:postgres@localhost/postgres }

test-workflow-sqlite:
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v4
    - uses: dtolnay/rust-toolchain@stable
    - uses: Swatinem/rust-cache@v2
    - run: cargo test -p assay-workflow --features backend-sqlite --test '*'

test-auth-pg:
  # same pattern against assay-auth, postgres:18-alpine service
  ...
test-auth-sqlite:
  ...
```

- [ ] **Step 2: Parallelise** — all four jobs run concurrently on a single PR push.

- [ ] **Step 3: Commit** — `ci: parametrised backend test matrix (PG18 + SQLite)`.

---

### Task 9.3: Release workflow — per-crate tag prefixes

**Files:** `.github/workflows/release.yml`.

- [ ] **Step 1: Parse tag job**

```yaml
on:
  push:
    tags: ["*-v*"]

jobs:
  parse-tag:
    runs-on: ubuntu-latest
    outputs:
      crate: ${{ steps.parse.outputs.crate }}
      version: ${{ steps.parse.outputs.version }}
      emits_binary: ${{ steps.parse.outputs.emits_binary }}
    steps:
      - id: parse
        run: |
          tag=${GITHUB_REF#refs/tags/}
          crate=${tag%-v*}
          version=${tag##*-v}
          case "$crate" in
            assay|assay-engine) emits_binary=true ;;
            *) emits_binary=false ;;
          esac
          echo "crate=$crate" >> "$GITHUB_OUTPUT"
          echo "version=$version" >> "$GITHUB_OUTPUT"
          echo "emits_binary=$emits_binary" >> "$GITHUB_OUTPUT"
```

Note: the `assay` tag maps to the `assay-lua` crate. Handle this in the subsequent jobs (look up the
real package name from the dir):

```yaml
crates-io:
  runs-on: ubuntu-latest
  needs: [parse-tag]
  steps:
    - uses: actions/checkout@v4
    - uses: dtolnay/rust-toolchain@stable
    - name: Publish to crates.io
      env:
        CARGO_REGISTRY_TOKEN: ${{ secrets.CARGO_REGISTRY_TOKEN }}
        CRATE: ${{ needs.parse-tag.outputs.crate }}
        VERSION: ${{ needs.parse-tag.outputs.version }}
      run: |
        set -euo pipefail
        # Map dir name to cargo package name
        case "$CRATE" in
          assay) PKG=assay-lua ;;
          *) PKG=$CRATE ;;
        esac
        STATUS=$(curl -sS -o /dev/null -w '%{http_code}' \
          "https://crates.io/api/v1/crates/$PKG/$VERSION")
        if [ "$STATUS" = "200" ]; then
          echo "skip: $PKG@$VERSION on crates.io"
          exit 0
        fi
        cargo publish -p "$PKG" --allow-dirty
```

- [ ] **Step 2: Build-binary job (gated)**

```yaml
build-binaries:
  if: ${{ needs.parse-tag.outputs.emits_binary == 'true' }}
  needs: [parse-tag]
  strategy:
    fail-fast: false
    matrix:
      include:
        - os: ubuntu-latest
          target: x86_64-unknown-linux-musl
          deps: sudo apt-get update && sudo apt-get install -y musl-tools
        - os: macos-14
          target: aarch64-apple-darwin
          deps: ""
  runs-on: ${{ matrix.os }}
  steps:
    - uses: actions/checkout@v4
    - uses: dtolnay/rust-toolchain@stable
      with: { targets: ${{ matrix.target }} }
    - uses: Swatinem/rust-cache@v2
    - run: ${{ matrix.deps }}
    - name: Resolve binary name
      id: bin
      run: |
        case "${{ needs.parse-tag.outputs.crate }}" in
          assay)        echo "binary=assay" >> "$GITHUB_OUTPUT"; echo "pkg=assay-lua" >> "$GITHUB_OUTPUT" ;;
          assay-engine) echo "binary=assay-engine" >> "$GITHUB_OUTPUT"; echo "pkg=assay-engine" >> "$GITHUB_OUTPUT" ;;
        esac
    - run: cargo build --release -p ${{ steps.bin.outputs.pkg }} --target ${{ matrix.target }}
    - run: cp target/${{ matrix.target }}/release/${{ steps.bin.outputs.binary }} \
           ${{ steps.bin.outputs.binary }}-${{ matrix.target }}
    - uses: actions/upload-artifact@v4
      with:
        name: ${{ steps.bin.outputs.binary }}-${{ matrix.target }}
        path: ${{ steps.bin.outputs.binary }}-${{ matrix.target }}
```

- [ ] **Step 3: GitHub Release job (binary crates only)**

```yaml
github-release:
  if: ${{ needs.parse-tag.outputs.emits_binary == 'true' }}
  needs: [parse-tag, build-binaries]
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v4
    - uses: actions/download-artifact@v4
      with: { path: artifacts }
    - run: |
        set -euo pipefail
        cd artifacts
        sha256sum */* > ../checksums.txt
    - env:
        GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
      run: |
        gh release create "${{ github.ref_name }}" \
          artifacts/*/* \
          checksums.txt \
          --generate-notes
```

- [ ] **Step 4: Docker (binary crates only)**

```yaml
docker:
  if: ${{ needs.parse-tag.outputs.emits_binary == 'true' }}
  needs: [parse-tag]
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v4
    - uses: docker/login-action@v3
      with:
        registry: ghcr.io
        username: ${{ github.actor }}
        password: ${{ secrets.GITHUB_TOKEN }}
    - uses: docker/build-push-action@v6
      with:
        push: true
        file: Dockerfile.${{ needs.parse-tag.outputs.crate }}
        tags: |
          ghcr.io/developerinlondon/${{ needs.parse-tag.outputs.crate }}:${{ needs.parse-tag.outputs.version }}
          ghcr.io/developerinlondon/${{ needs.parse-tag.outputs.crate }}:latest
```

Two Dockerfiles needed: `Dockerfile.assay` (runtime) and `Dockerfile.assay-engine` (engine). Each
FROMs a minimal base (alpine or scratch for musl), copies the built binary.

- [ ] **Step 5: NPM (openclaw-extension untouched)**

Keep the existing `npm` job gated on `${{ needs.parse-tag.outputs.crate == 'assay' }}` — the Lua
extension follows the runtime binary.

- [ ] **Step 6: Commit** — `ci: per-crate tag-driven release workflow with binary matrix`.

---

### Task 9.4: Dockerfiles

**Files:** `Dockerfile.assay`, `Dockerfile.assay-engine`.

- [ ] **Step 1: Runtime Dockerfile**

```dockerfile
# Dockerfile.assay — runtime image
FROM rust:1-alpine AS builder
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY . .
RUN cargo build --release -p assay-lua --target x86_64-unknown-linux-musl

FROM alpine:3
COPY --from=builder /src/target/x86_64-unknown-linux-musl/release/assay /usr/local/bin/assay
ENTRYPOINT ["/usr/local/bin/assay"]
```

- [ ] **Step 2: Engine Dockerfile**

```dockerfile
# Dockerfile.assay-engine
FROM rust:1-alpine AS builder
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY . .
RUN cargo build --release -p assay-engine --target x86_64-unknown-linux-musl

FROM alpine:3
COPY --from=builder /src/target/x86_64-unknown-linux-musl/release/assay-engine /usr/local/bin/assay-engine
EXPOSE 3000
ENTRYPOINT ["/usr/local/bin/assay-engine"]
CMD ["serve"]
```

- [ ] **Step 3: Replace the old `Dockerfile` with a pointer**

Keep the root `Dockerfile` as a compatibility shim — but make it ERROR with a useful message:

```dockerfile
FROM alpine:3
RUN echo "Use Dockerfile.assay or Dockerfile.assay-engine — the root Dockerfile is obsolete post-0.13.0" && exit 1
```

Or delete the root Dockerfile entirely and document the two in README.

- [ ] **Step 4: Commit** — `ci: split Dockerfile into runtime vs engine`.

---

### Phase 9 exit criteria

- Tagging `assay-workflow-v0.2.0` publishes only the workflow crate (no binary, no Docker).
- Tagging `assay-engine-v0.1.0` publishes the crate + binary + Docker image.
- Tagging `assay-v0.13.0` publishes the `assay-lua` crate + `assay` binary + Docker image.
- CI test matrix runs PG18 + SQLite in parallel on every PR.
- A synthetic tag push on a test branch successfully produces all expected artefacts.

---

## Phase 10 — Docs + ship

### Task 10.1: CHANGELOGs per crate

Follow plan 09 format (changelog-per-release-files).

- [ ] **Step 1: Per-crate changelogs**

```
CHANGELOG/
├── assay-0.13.0.md
├── assay-workflow-0.2.0.md
├── assay-domain-0.1.0.md
├── assay-dashboard-0.1.0.md
├── assay-engine-0.1.0.md
└── assay-auth-0.1.0.md
```

Each file: ## Summary · ## Added · ## Changed · ## Removed · ## Upgrade notes.

- [ ] **Step 2: Index**

Root `CHANGELOG.md` has a "## v0.13.0 (2026-MM-DD)" section pointing to each per-crate file.

- [ ] **Step 3: Commit** — `docs: CHANGELOGs for 0.13.0`.

---

### Task 10.2: README updates

**Files:** root `README.md`, `crates/*/README.md`.

- [ ] **Step 1: Runtime vs engine table**

```markdown
## Two binaries, two use cases

| Use case               | Binary         | Install                                       |
| ---------------------- | -------------- | --------------------------------------------- |
| Scripting / automation | `assay`        | `cargo install assay-lua` or download release |
| Workflow + auth server | `assay-engine` | `cargo install assay-engine` or Docker        |

- `assay` runs Lua scripts with embedded workflow engine (PG/SQLite). Call `assay-engine` over HTTP
  for auth.
- `assay-engine` is a standalone HTTP server with workflow + auth + dashboard, pluggable across PG
  (default, PG18 minimum) and SQLite — both backends compiled in, runtime-selected via config.

See [docs/migration-to-0.13.0.md](./docs/migration-to-0.13.0.md) for the upgrade path from older
versions.
```

- [ ] **Step 2: Per-crate READMEs**

Each module crate gets a README.md explaining: what this crate is, when to depend on it, feature
flags, minimum examples.

- [ ] **Step 3: Commit** — `docs(readme): document runtime/engine split`.

---

### Task 10.3: Migration notes (0.12 → 0.13)

**Files:** `docs/migration-to-0.13.0.md`.

Key messages:

- **Binary `assay` users:** no action. `cargo install assay-lua --version 0.13.0` and go.
- **Crate consumers of `assay` lib:** the crate is renamed from `assay-lua` to `assay-lua`
  (unchanged); internal module paths changed. Update `use assay_workflow::...` imports (previously
  `use assay::workflow::...`).
- **`assay-workflow` 0.1 embedders:** the workflow store trait moved to `assay-domain`. Update
  imports: `use assay_workflow::WorkflowStore` → `use assay_domain::WorkflowStore`. The `Engine` is
  no longer generic — drop the type parameter.
- **New engine consumers:** follow `docs/engine-quickstart.md` (to be written).

- [ ] **Step 1: Write migration doc with code snippets.**
- [ ] **Step 2: Commit** — `docs: migration-to-0.13.0 notes`.

---

### Task 10.4: `llms.txt` update

**Files:** root `llms.txt`.

- [ ] **Step 1: Reflect workspace layout**

New sections:

- Architecture: link to plan 10.
- Execution plan: link to plan 12 + sub-plans.
- Crates: one line per crate with purpose.
- Feature flags: the top-level feature matrix.

- [ ] **Step 2: Commit** — `docs(llms): update for 0.13.0 workspace`.

---

### Task 10.5: Final full-stack smoke

- [ ] **Step 1: Manual e2e against a local config**

```bash
# Start engine against SQLite
cargo run --release -p assay-engine -- --config crates/assay-engine/examples/sqlite.toml &

# Register an OIDC client via admin CLI
cargo run --release -p assay-engine -- admin client-create \
    --name "Test App" --redirect "http://localhost:8080/callback"

# Run a Lua workflow via runtime binary
cat > /tmp/wf.lua <<EOF
local id, err = workflow.start({ name = "hello", payload = { greet = "world" } })
print(id, err)
EOF
cargo run --release -p assay-lua -- run /tmp/wf.lua
```

Expected: workflow starts, dashboard shows it, engine's OIDC provider is up.

- [ ] **Step 2: Run all exit-criteria checks from plan 12 main doc.**

- [ ] **Step 3: Gather evidence into `docs/0.13.0-release-evidence.md`**

Test counts, binary sizes, per-backend smoke-test logs. Commit:

```bash
git commit -m "docs: 0.13.0 release evidence"
```

---

### Task 10.6: Merge, tag, ship

- [ ] **Step 1: Review PR** `feature/0.13.0-engine-split` → `main`.

Get a second pair of eyes or use code-reviewer agent. Fix feedback.

- [ ] **Step 2: Merge**

Prefer squash merge so main has one commit per logical unit. Commit message:

```
feat: v0.13.0 — engine split + auth (PG18 + SQLite) (#XXX)

See plan 12 and sub-plans 12a-12e for full architecture details.
```

- [ ] **Step 3: Tag all six crates**

```bash
git checkout main
git pull
git tag assay-domain-v0.1.0
git tag assay-workflow-v0.2.0
git tag assay-dashboard-v0.1.0
git tag assay-auth-v0.1.0
git tag assay-engine-v0.1.0
git tag assay-v0.13.0                 # triggers runtime binary + docker
git push origin --tags
```

CI fires six times. Each tag-prefix matches a specific job set. Check the Actions tab; all should
complete within ~20 min.

- [ ] **Step 4: Verify publishes**

```bash
cargo search assay-domain assay-workflow assay-dashboard assay-auth assay-engine assay-lua
```

Versions should match. Hit `ghcr.io/developerinlondon/assay:0.13.0` and
`ghcr.io/developerinlondon/assay-engine:0.1.0` — both should pull.

- [ ] **Step 5: Publish release notes**

Post the v0.13.0 announcement (GitHub release notes, any community channels). Include the migration
guide link.

---

## Phase 10 exit criteria

- All six per-crate CHANGELOGs present.
- README reflects runtime/engine split.
- Migration doc covers 0.12 → 0.13 paths.
- `cargo search` shows every crate at the expected version.
- Docker images pullable.
- `feature/0.13.0-engine-split` branch merged to `main`.

---

## What's next (post-0.13.0)

- **0.13.x patch releases** as bugs surface from real usage.
- **0.14.0 planning:** advanced Zanzibar features (caveats, temporal), TOTP / SMS MFA (auth module),
  admin UI (dashboard), SAML (auth module, if asked for), HSM support for JWKS keys.
- **Third-party backend contributions:** lay the groundwork for users writing their own
  `MySqlWorkflowStore` etc. via a public backend trait + test harness.

The architecture established in 0.13.0 (`FromRef` composition, per-module Ctx, Layout 1 backends)
should absorb all of these without core refactors. That's the test of whether the architecture
landed correctly.
