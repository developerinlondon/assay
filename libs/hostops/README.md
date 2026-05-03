# hostops

A host-visibility dashboard packaged as an `assay` library. Mount it on a consumer app's `routes`
table to expose pages for nspawn containers, systemd services, cron timers, journal logs, network
interfaces, tunnels, tailscale, and audit-log viewing — all from a single Lua require.

This library replaces the standalone `knowhere0426` host monolith. See plan 21 in
`assay/.claude/plans/21-libs-folder-and-install.md` for the full design.

## Quick start

```lua
local hostops = require("hostops.mount")

local routes = { GET = {}, POST = {} }
hostops.mount(routes, {
  prefix = "/host",                     -- optional, default "/"
  state  = require("app.services.state"),   -- machine + disk + proc snapshots
  audit  = require("app.services.audit"),   -- audit-log writer
  jobs   = require("app.services.jobs"),    -- job/task tracker
  secret = require("app.services.secret"),  -- secret-store reader
  brand  = require("app.brand"),            -- brand pack (logo/colors/strings)
  engine = engine_http_client,              -- HTTP wrapper to engine sidecar
  lib_root = "/opt/assay/libs/hostops",     -- where the lib is installed
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
| `secret`   | table  | `secret.read(scope, key)`                            |
| `brand`    | table  | `brand.snapshot()` returns logo / accent / strings   |
| `engine`   | table  | HTTP wrapper to the engine sidecar (workflow/auth/…) |
| `lib_root` | string | path to this lib's install dir (for `static/` reads) |

Every page reads from these via the shared `hostops.ctx` module rather than top-level requires, so
consumer apps can swap implementations without re-loading the library.

## What's in / what's not

**In** — host visibility surfaces:

- Dashboard (machines grid, status strip, recent activity)
- nspawn container list + per-container detail (services, cron, logs, shell)
- Host services (systemd unit list)
- Host cron (timers + crontabs)
- Host logs (journal viewer + SSE stream)
- Networks (tunnels, interfaces, tailscale)
- Audit log viewer + export
- Host shell (xterm.js + PTY)

**Not in** (lives elsewhere):

- Backups — moves to the future `assay-ops` extension; rebuilt cleanly via the `rustic` CLI binary
  at that point.
- Packages — owned by plan 20's `pkg` stdlib, available directly to consumer apps.
- Auth / vault / workflow / engine consoles — separate libraries or extensions.

## Install

Hostops ships as a tarball alongside the assay binary. Declare it in your app's `Manifest.lua` and
run `assay install`:

```lua
return {
  libs = {
    { name = "hostops", version = "0.1.0", sha256 = "..." },
  },
}
```

`assay install` extracts the tree to `<lib-dir>/hostops/`, where `<lib-dir>` defaults to
`/opt/assay/libs/` (root) or `$XDG_DATA_HOME/assay/libs/` (per-user). Pass that path as
`opts.lib_root` to `mount()`.

## Running tests in-repo

```
LUA_PATH='libs/?.lua;libs/?/init.lua;libs/hostops/?.lua;libs/hostops/tests-lua/?.lua;;' \
  assay libs/hostops/tests-lua/smoke.test.lua
```

The smoke test boots the lib with stub services and asserts a representative set of routes render.
See plan 21 phase 4 for the test strategy.

## Versioning

Per-lib semver tracked in `VERSION`. Consumer apps pin via the `Manifest.lua` `libs` entry;
`assay install` resolves the matching release tarball.
