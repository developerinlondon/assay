# Migration guide — Assay 0.12.x → 0.13.0

v0.13.0 is a breaking release. The monolithic `assay` binary is split into six crates and the
embedded workflow-engine-inside-the-runtime is retired. This doc covers every required change for
binary users, Rust library consumers, and operators running the old `assay serve` in production.

If something here isn't clear, open an issue.

## TL;DR

- `assay serve` is gone. Run `assay-engine serve --config <path.toml>` instead.
- SurrealDB backend removed. PG18 + SQLite only.
- Library imports changed: `WorkflowStore` is in `assay-domain` now, not `assay-workflow`.
  `Engine<S>` dropped its generic.
- `assay-lua` (the runtime binary) no longer embeds workflow. It's a pure Lua runtime + HTTP client
  that talks to a deployed `assay-engine`.
- Dashboard is served by `assay-engine` only, not by `assay-lua`.

## Scenario 1 — you ran `assay serve` in production

**Old (0.12.x):**

```bash
assay serve --backend postgres://user:pass@host/assay --port 8080 --auth-issuer https://...
```

**New (0.13.0):**

1. Install the engine binary: `cargo install assay-engine` (or pull the Docker image when released).
2. Write `engine.toml`:

   ```toml
   [server]
   bind_addr = "0.0.0.0:8080"

   [backend]
   type = "postgres" # or "sqlite"
   url = "postgres://user:pass@host/assay" # PG18 minimum
   # path = "/var/lib/assay/engine.db"       # SQLite alternative
   ```

3. Run: `assay-engine serve --config engine.toml`

Authentication: in v0.13.0 the engine runs open (`AuthMode::no_auth()`). JWT / API-key protection
returns in v0.14.0 when the IdP modules land. **Do not expose v0.13.0 engines on the public internet
without a network gatekeeper** (Cloudflare Access, Tailscale, VPN, or similar).

## Scenario 2 — you used `--backend surreal://...`

The SurrealDB backend is removed. Move to PG18 (recommended) or SQLite.

- PG18: create a fresh database; `assay-engine serve --config engine.toml` will run migrations on
  first connect.
- SQLite: set `backend.type = "sqlite"` and `path = "/path/to/file.db"`.

There's no in-place migration path from SurrealDB state to PG/SQLite — the schemas and ID encodings
differ. Either:

- Accept a clean slate on v0.13.0 (simplest), or
- Write a one-off migration script using the SurrealDB 3.x client to read tuples and the new
  `assay-engine` HTTP API to replay them. No tooling is provided for this.

Rationale for dropping SurrealDB: plan 12 Revision log in `.claude/plans/12-v0.13.0-execution.md` —
~3× build time + 3× compile RAM with no capability gain over PG18 + `pgvector` + recursive CTEs.

## Scenario 3 — you depended on `assay-workflow` as a library

**Import paths moved:**

```rust
// Old (0.1.x)
use assay_workflow::{WorkflowStore, WorkflowRecord, WorkflowEvent, Engine};

// New (0.2.0)
use assay_domain::{WorkflowStore, types::{WorkflowRecord, WorkflowEvent}};
use assay_workflow::WorkflowCtx;
```

**The `<S>` generic dropped from `Engine`:**

```rust
// Old
let store = PostgresStore::connect(&url).await?;
let engine = Engine::<PostgresStore>::new(store);

// New — WorkflowCtx is generic, but the type parameter is usually elided:
let store = PostgresStore::new(&url).await?;          // migrations run automatically
let ctx = WorkflowCtx::start(Arc::new(store));
// ctx IS the orchestrator AND the axum state. See plan 12a Task 1.3.
```

**Features are additive:** `backend-postgres` and `backend-sqlite` can both be compiled in the same
binary. Runtime selection happens via `EngineConfig.backend`. No mutual exclusion.

**`subscribe_runnable` / `subscribe_tasks` are now `async`:**

```rust
// Old (0.1.x) — returned a lazy stream; LISTEN was issued on first poll.
// Callers could race the subscription against the first pg_notify.
let mut stream = store.subscribe_runnable("main");

// New (0.2.0) — awaits LISTEN registration before handing back the stream.
// By the time `.await` resolves, the subscription is active on the server.
let mut stream = store.subscribe_runnable("main").await;
```

The old shape dropped notifications when a caller `pg_notify`'d between construction and first poll.
The new shape makes this impossible. `PostgresStore::from_pool(pool)` is also new — use it when the
engine owns the pool and hands a clone to the workflow module.

## Scenario 4 — you're writing Lua scripts against the engine HTTP API

No change. `$ASSAY_ENGINE_URL` + the CLI subcommands (`assay workflow start`,
`assay schedule describe`, `assay namespace list`, etc.) work exactly as in v0.12.

```lua
-- still works
local http = require("http")
local r = http.post("http://engine:8080/api/v1/workflows", { json = {
  workflow_id = "demo-1",
  workflow_type = "demo.greet",
  namespace = "main",
  task_queue = "default",
  input = [[{"name":"world"}]],
}})
```

## Scenario 5 — you embedded `assay-engine` as a crate (new in 0.13.0)

This is the supported path for projects like `jeebon-api`:

```toml
# jeebon-api/Cargo.toml
[dependencies]
assay-engine = { version = "0.1", default-features = false, features = ["backend-postgres", "backend-sqlite" # "server",  # <- enable if you want the binary's clap-based entrypoint
  # "auth",    # <- empty in 0.13.0; ready for v0.14.0
] }
```

Then:

```rust
use assay_engine::{EngineConfig, run};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = EngineConfig::from_file(std::path::Path::new("engine.toml"))?;
    run(cfg).await
}
```

## Scenario 6 — CI pipelines that built from `cargo build --release`

Workspace root is now pure — no `[package]`. If your CI did:

```bash
cargo build --release           # built the `assay` binary
```

That still works — it builds every crate in the workspace. To get just one binary:

```bash
cargo build --release -p assay-lua --bin assay                        # runtime (size-optimised)
cargo build --profile server-release -p assay-engine --bin assay-engine # engine (panic=unwind)
```

The `assay` binary (`assay-lua` crate) is smaller in 0.13.0 (~11 MB stripped vs ~14 MB in 0.12)
because the workflow engine is no longer linked in.

## Feature-flag changes summary

| Crate                     | Feature               | 0.12.x    | 0.13.0                    |
| ------------------------- | --------------------- | --------- | ------------------------- |
| `assay-lua` (was `assay`) | `workflow`            | default   | **removed**               |
| `assay-lua`               | `db`, `server`, `cli` | default   | unchanged                 |
| `assay-workflow`          | `backend-postgres`    | always on | default (opt-outable)     |
| `assay-workflow`          | `backend-sqlite`      | always on | default (opt-outable)     |
| `assay-workflow`          | `backend-surrealdb`   | opt-in    | **removed**               |
| `assay-engine`            | `server`              | —         | default (requires `clap`) |
| `assay-engine`            | `auth`                | —         | optional, empty in 0.13.0 |

## Known gaps in 0.13.0 (filled in 0.14.0)

- No built-in auth on `assay-engine`. Operators must gate the engine at the network layer.
- Dashboard shows workflow + schedule + queue + workers tabs, but no auth/user/Zanzibar views (those
  crates are scaffolds).
- `assay-auth` crate is an empty placeholder — don't try to use it yet. Phases 4–7 of plan 12c build
  it out.

## Rollback

If v0.13.0 doesn't work for you, pin to `0.12.1` for the runtime binary. `assay-engine` has no 0.12
predecessor — the concept is new.

```
cargo install assay-lua --version 0.12.1    # old runtime + embedded workflow
```

Note that 0.12.x is the final major version with SurrealDB support.

## Questions

File issues against [github.com/developerinlondon/assay](https://github.com/developerinlondon/assay)
with the `migration` label. Include your old `assay serve` command line + which scenario above you
fall under.
