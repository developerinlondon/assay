# sysops

A host-visibility dashboard packaged as an `assay` library. Mount it on a consumer app's `routes`
table to expose pages for nspawn containers, systemd services, cron timers, journal logs, network
interfaces, tunnels, tailscale, and audit-log viewing — all from a single Lua require.

This library replaces the standalone `knowhere0426` host monolith. See plan 21 in
`assay/.claude/plans/21-libs-folder-and-install.md` for the full design.

## Quick start

```lua
local sysops = require("sysops.mount")
local vault  = require("sysops.vault")

local routes = { GET = {}, POST = {} }
sysops.mount(routes, {
  prefix = "/host",                     -- optional, default "/"
  state  = require("app.services.state"),   -- machine + disk + proc snapshots
  audit  = require("app.services.audit"),   -- audit-log writer
  jobs   = require("app.services.jobs"),    -- job/task tracker
  secret = vault.secret_store({
    app = "my-host-manager",
    admin_key_envs = { "MY_APP_ADMIN_API_KEYS" },
  }),
  brand  = require("app.brand"),            -- brand pack (logo/colors/strings)
  engine = engine_http_client,              -- HTTP wrapper to engine sidecar
  lib_root = "/opt/assay/libs/sysops",     -- where the lib is installed
})

http.serve(8080, routes)
```

## Contract

The library is a pure mounting layer. It owns no global state; every service it needs flows in via
`mount()`'s `opts`:

| Field      | Type   | Used for                                             |
| ---------- | ------ | ---------------------------------------------------- |
| `prefix`   | string | URL prefix for every registered route. Default `/`.  |
| `state`    | table  | `state.snapshot()`, `state.machine_deep(name)`, …    |
| `audit`    | table  | `audit.append({...})`, `audit.recent(n)`             |
| `jobs`     | table  | `jobs.active({kind=...})`, `jobs.append_log(id, …)`  |
| `secret`   | table  | `secret.read/write/delete/available(...)`            |
| `brand`    | table  | `brand.snapshot()` returns logo / accent / strings   |
| `engine`   | table  | HTTP wrapper to the engine sidecar (workflow/auth/…) |
| `lib_root` | string | path to this lib's install dir (for `static/` reads) |

Every page reads from these via the shared `sysops.ctx` module rather than top-level requires, so
consumer apps can swap implementations without re-loading the library.

## Vault-backed host secrets

`sysops.vault` provides the shared Assay Engine Vault integration for host-manager apps. Use
`vault.secret_store(opts)` as the `secret` service passed to `mount()`.

```lua
local vault = require("sysops.vault")

local secret = vault.secret_store({
  app = "knowhere",
  admin_key_envs = { "KNOWHERE_ADMIN_API_KEYS" },
  kv_prefix = "knowhere",
})
```

The returned table implements:

| Method      | Purpose                                                        |
| ----------- | -------------------------------------------------------------- |
| `read`      | Read `scope/key` from engine KV, then legacy local fallbacks.   |
| `write`     | Store `scope/key` in assay-engine KV v2.                       |
| `delete`    | Soft-delete the latest KV version for `scope/key`.              |
| `available` | Report engine vault seal/status information when configured.    |

This store is for host/app operational secrets, such as backup repository credentials. User-owned
secrets should live in Assay Engine Vault's authenticated personal vault and Bitwarden-compatible
surface, not in the sysops host secret store.

## What's in / what's not

**In** — host visibility surfaces:

- Dashboard (machines grid, status strip, recent activity)
- nspawn container list + per-container detail (services, cron, logs, shell)
- Host services (systemd unit list, sortable CPU/memory stats, expandable systemd details, start/stop/restart actions)
- Host cron (timers + crontabs)
- Host logs (journal viewer + SSE stream)
- Networks (tunnels, interfaces, tailscale)
- Audit log viewer + export
- Host shell (xterm.js + PTY)

**Not in** (lives elsewhere):

- Backups — moves to the future `assay-ops` extension; rebuilt cleanly via the `rustic` CLI binary
  at that point.
- Packages — owned by plan 20's `pkg` stdlib, available directly to consumer apps.
- Workflow / engine consoles — served by the engine SPA via the existing sidebar `Engine` link.

## Optional auth + vault modules (0.1.5)

Sysops 0.1.5 adds opt-in auth and vault dashboards that render in-process as Lua pages instead of
forcing a hand-off to the engine SPA. They're gated behind `mount` opts so existing 0.1.4
consumers see no change.

```lua
sysops.mount(routes, {
  -- … all existing opts …
  active_modules    = { "auth", "vault" },     -- enable both, or pick one
  engine_admin_key  = env.get("KNOWHERE_ADMIN_API_KEYS"),
})
```

When `"auth"` is in `active_modules`, the sidebar gains an **Auth** link to `/auth/users` and
sysops registers GET/POST routes for:

| Path                       | Page                                |
| -------------------------- | ----------------------------------- |
| `/auth/users`              | List + create + delete users        |
| `/auth/users/{id}/edit`    | Edit a user                         |
| `/auth/sessions`           | List + revoke sessions              |
| `/auth/oidc-clients`       | OIDC client list (read-only)        |
| `/auth/upstreams`          | Upstream IdP list (read-only)       |
| `/auth/jwks`               | JWKS keys                           |
| `/auth/biscuit`            | Biscuit signing-key info            |
| `/auth/audit`              | Auth audit log                      |
| `/zanzibar`                | Zanzibar namespaces                 |
| `/zanzibar/tuples`         | Write / delete tuples               |
| `/zanzibar/check`          | Run a `check(s,r,o)` query          |

When `"vault"` is in `active_modules`, the sidebar gains a **Vault** link to `/vault` and sysops
registers GET/POST routes for:

| Path                  | Page                                     |
| --------------------- | ---------------------------------------- |
| `/vault`              | Overview + seal status pill              |
| `/vault/kv`           | KV browser (`?prefix=…`) + put / del     |
| `/vault/transit`      | Transit keys + encrypt / decrypt         |
| `/vault/sealing`      | Seal / unseal / shamir init              |
| `/vault/dynamic`      | Dynamic credential leases                |
| `/vault/share`        | Share tokens (mint / revoke)             |
| `/vault/collections`  | Shared cipher collections                |
| `/vault/me`           | Personal cipher vault (bitwarden compat) |

All pages call the engine via the HTTP client passed as `opts.engine`. Admin-scoped endpoints
(users / sessions / sealing / transit / zanzibar) require `engine_admin_key`.

## SDK (0.1.5)

Pure-Lua HTTP wrappers around the engine vault + auth API. Useful from plugins and other lib code
that needs to read or write engine state without re-rolling HTTP. Each module accepts the same
HTTP client passed via `mount` opts:

```lua
local vault = require("sysops.vault").new(ctx.engine)
vault.kv.get("apps/foo/db_url")
vault.transit.encrypt("master", "hello world")
vault.dynamic.lease("postgres", "readonly")
vault.sealing.status()

local auth = require("sysops.auth").new(ctx.engine)
auth.users.list({ search = "alice" })
auth.zanzibar.check("user:alice", "viewer", "doc:foo")
```

Every method returns `(data, nil)` on 2xx or `(nil, err)` where `err = { status, body }`.

## Install

Sysops ships as a tarball alongside the assay binary. Declare it in your app's `Manifest.lua` and
run `assay install`:

```lua
return {
  libs = {
    { name = "sysops", version = "0.1.0", sha256 = "..." },
  },
}
```

`assay install` extracts the tree to `<lib-dir>/sysops/`, where `<lib-dir>` defaults to
`/opt/assay/libs/` (root) or `$XDG_DATA_HOME/assay/libs/` (per-user). Pass that path as
`opts.lib_root` to `mount()`.

## Running tests in-repo

```
LUA_PATH='libs/?.lua;libs/?/init.lua;libs/sysops/?.lua;libs/sysops/tests-lua/?.lua;;' \
  assay libs/sysops/tests-lua/services.test.lua
LUA_PATH='libs/?.lua;libs/?/init.lua;libs/sysops/?.lua;libs/sysops/tests-lua/?.lua;;' \
  assay libs/sysops/tests-lua/smoke.test.lua
```

The service helper test covers service stats formatting, detail extraction, and lifecycle action validation. The smoke test boots
the lib with stub services and asserts a representative set of routes render. See plan 21 phase 4
for the test strategy.

## Versioning

Per-lib semver tracked in `VERSION`. Consumer apps pin via the `Manifest.lua` `libs` entry;
`assay install` resolves the matching release tarball.
