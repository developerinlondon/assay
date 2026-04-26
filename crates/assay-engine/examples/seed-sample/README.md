# sample-data seeder (Lua)

Populate a running assay-engine with demo users, OIDC clients, an upstream provider, Zanzibar
tuples, and demo workflows — so operators can poke at the dashboards without writing curl scripts.

Replaces the v0.13.x `assay-engine seed-sample` Rust subcommand (retired in plan-15 slice 5). Same
fixtures, same idempotency.

## Prerequisites

- A running `assay-engine` reachable from your shell.
- An admin api-key configured under `[auth].admin_api_keys` in the engine's `engine.toml` — the
  seeder hits `/api/v1/engine/auth/admin/*` endpoints which require it.
- `examples/init/init.lua` already run, so the operator user exists and the default Zanzibar
  namespace schemas (`engine`, `auth`, `workflow`) are defined. Without that, this script's
  user/tuple writes have nothing to anchor against.
- Auth module enabled (`engine.modules.auth.enabled = TRUE`). When auth is off the seeder skips the
  auth fixtures and only seeds workflows.

## Run

```bash
ASSAY_ENGINE_URL=http://localhost:8420 \
ASSAY_ADMIN_KEY=dev-admin-key-change-me \
assay run examples/seed-sample/seed.lua
```

Re-running is a no-op — every insert path either checks for the row first or uses an upsert
endpoint, so the seeder is safe to invoke repeatedly during local-dev iteration.

## What lands

| kind             | name(s)                                                                                                                    | endpoint                                         |
| ---------------- | -------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| `namespace`      | `demo`, `prod`                                                                                                             | `POST /api/v1/engine/workflow/namespaces`        |
| `workflow`       | `demo-greet-1`, `demo-greet-2`, `demo-greet-3`                                                                             | `POST /api/v1/engine/workflow/workflows`         |
| `user`           | `alice@example.com` (verified, pw `assay-demo`), `bob@example.com`, `cousin@example.com` (unverified), `admin@example.com` | `POST /api/v1/engine/auth/admin/users`           |
| `oidc_client`    | `demo-spa` (PKCE-only public), `demo-service` (confidential client_secret)                                                 | `POST /api/v1/engine/auth/admin/oidc/clients`    |
| `oidc_upstream`  | `example` (mock issuer `https://accounts.example.com`)                                                                     | `POST /api/v1/engine/auth/admin/oidc/upstream`   |
| `zanzibar_tuple` | `family:alice` admin/member by alice; `family:bob` admin/member by bob; `circle:inner` members alice + bob                 | `POST /api/v1/engine/auth/admin/zanzibar/tuples` |

## Notes

- HTTP-only — works against any reachable engine, regardless of backend (SQLite or PG).
- Demo workflows assume a `demo.greet` worker is registered. If no worker has joined yet, the
  workflows still create — they sit in `Started` state until a worker picks them up.
- The mock OIDC upstream isn't reachable; it's there so the dashboard has a row to render. Replace
  `client_secret` before pointing at a real IdP.
