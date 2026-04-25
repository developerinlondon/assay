# Engine + auth console end-to-end tests

Headless browser tests covering the engine console (`/engine/console`), auth console
(`/auth/console`), and the cross-console nav strip. Live in this directory so anyone touching the
engine HTTP API surface or the SPA components can add coverage in the same PR.

## Run locally

From `crates/assay-engine/tests-e2e/`:

```sh
npm install                                  # one-time
npx playwright install --with-deps chromium  # one-time

# Build the engine binary first (the seed-sample subcommand needs it):
cargo build --release -p assay-engine --features backend-sqlite,auth,server

# Run everything (boots engine + seeds + tests + tears down):
bash run.sh

# Or run pieces separately if you want a long-running engine:
ASSAY_ENGINE_BIN=../../../target/release/assay-engine bash fixtures/seed.sh
npx playwright test
npx playwright test --headed
npx playwright show-report
```

`run.sh` is what CI uses too — it wipes `/tmp/assay-engine-e2e-data`, boots `assay-engine serve`
against `fixtures/engine.toml` (SQLite + auth + admin api-key `dev-admin-key-change-me`), waits for
`/api/v1/engine/info` to answer, runs `assay-engine seed-sample` to populate fixtures, then drives
Playwright. Engine log lands at `/tmp/assay-engine-e2e.log` for post-mortem.

## What's covered

- `engine-console.spec.ts`
  - `/engine/console` shell loads + paints the Info pane from `/api/v1/engine/info` (version,
    instance, started, uptime, modules).
  - Modules table loads from `/api/v1/engine/modules` (admin token persisted via localStorage in
    beforeEach).
  - Instances table loads from `/api/v1/engine/instances`.
  - Audit log paginates (Prev/Next disabled at boundaries).
  - Config view shows the redacted `[REDACTED]` placeholder for `admin_api_keys` (no plaintext
    leakage).

- `auth-console.spec.ts`
  - `/auth/console` shell loads + each sidebar pane (Users, Sessions, OIDC Clients, Upstream,
    Zanzibar, JWKS / Biscuit, Audit) renders its table without errors.
  - Round-trip: create a one-off user, see it in the list, delete it, confirm it's gone.

- `cross-nav.spec.ts`
  - All three pills render on each console.
  - Active pill highlights match the current console.
  - Clicking a pill navigates to the corresponding console.
  - Header bar identity strip (version + leader + instance) populates.

## Adding new tests

Spec files live next to the existing ones. The `_setup.ts` module exports helper functions that
persist the admin token to localStorage before each test — reuse those rather than coding the
storage handshake in every spec.
