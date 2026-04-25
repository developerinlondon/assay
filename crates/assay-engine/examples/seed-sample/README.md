# `assay-engine seed-sample`

Populate a running engine with a small set of fixture data — users, OIDC clients, an upstream
provider, Zanzibar tuples, and demo workflows — so operators can poke at the dashboards immediately
without writing curl scripts.

## Prerequisites

- A running `assay-engine` reachable from your shell.
- An admin api-key configured under `[auth].admin_api_keys` in the engine's `engine.toml`. The
  seeder hits `/admin/auth/*` and `/admin/oidc/*` endpoints which require it.
- Auth module enabled (i.e. `engine.modules.auth.enabled = TRUE`). When auth is off, the seeder
  skips the auth-only fixtures and only seeds workflows.

## Run

```bash
# Easiest: pass the same TOML the engine is running with so the
# seeder picks up the public_url automatically.
assay-engine seed-sample \
    --config engine.toml \
    --admin-key dev-admin-key-change-me

# Or override the base URL explicitly:
assay-engine seed-sample \
    --base-url http://localhost:8420 \
    --admin-key dev-admin-key-change-me
```

Re-running is a no-op — every insert path either checks for the row first or uses an upsert
endpoint, so the seeder is safe to invoke repeatedly during local-dev iteration.

## What lands

| kind             | name(s)                                                                                                                    | endpoint                           |
| ---------------- | -------------------------------------------------------------------------------------------------------------------------- | ---------------------------------- |
| `namespace`      | `demo`, `prod`                                                                                                             | `POST /api/v1/namespaces`          |
| `workflow`       | `demo-greet-1`, `demo-greet-2`, `demo-greet-3`                                                                             | `POST /api/v1/workflows`           |
| `user`           | `alice@example.com` (verified, pw `assay-demo`), `bob@example.com`, `cousin@example.com` (unverified), `admin@example.com` | `POST /admin/auth/users`           |
| `oidc_client`    | `demo-spa` (PKCE-only public), `demo-service` (confidential client_secret)                                                 | `POST /admin/oidc/clients`         |
| `oidc_upstream`  | `example` (mock issuer `https://accounts.example.com`)                                                                     | `POST /admin/oidc/upstream`        |
| `zanzibar_tuple` | `family:alice` admin/member by alice; `family:bob` admin/member by bob; `circle:inner` members alice + bob                 | `POST /admin/auth/zanzibar/tuples` |

Output is a status table per row — `created`, `exists`, `skipped`, `failed`. The process exits with
code 1 when any row fails so CI pipelines that wrap the seeder catch errors deterministically.

## Notes

- The seeder talks HTTP only. It works against any running engine reachable from your network,
  regardless of backend (SQLite or PG).
- Demo workflows assume a `demo.greet` worker is registered. If no worker has joined yet, the
  workflows still create — they just sit in `Started` state until a worker picks them up.
- The mock OIDC upstream isn't reachable; it's there so the dashboard has a row to render. Replace
  `client_secret` before pointing at a real IdP.
