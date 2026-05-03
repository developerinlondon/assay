---
category: Infrastructure
tagline: rustic backup CLI wrapper — snapshots, backup, restore, init, check, forget (require("assay.rustic"), v0.15.7+)
---

## assay.rustic

Lua stdlib that wraps the [rustic](https://rustic.cli.rs/) backup CLI. The underlying binary stays
external (no `rustic_core` crate is linked into the assay binary); this module just shells out to it
and parses the `--json` output. Repository URL + credentials travel as environment variables so they
don't leak via `/proc/<pid>/cmdline`.

```lua
local rustic = require("assay.rustic")

local opts = {
  repository        = "s3:https://s3.example.com/backups/host",
  password          = "topsecret",
  region            = "us-east-1",
  access_key_id     = "AKIA…",
  secret_access_key = "…",
}

local snaps, err = rustic.snapshots(opts)
if err then error("snapshots: " .. err) end
print("found " .. #snaps .. " snapshot(s)")
```

### Connection table

Every function takes an `opts` table with the connection parameters:

| Field               | Type   | Required | Notes                            |
| ------------------- | ------ | -------- | -------------------------------- |
| `repository`        | string | yes      | rustic repo URL or local path    |
| `password`          | string | yes      | repo encryption password         |
| `region`            | string | no       | S3 region (`AWS_REGION`)         |
| `access_key_id`     | string | no       | S3 (`AWS_ACCESS_KEY_ID`)         |
| `secret_access_key` | string | no       | S3 (`AWS_SECRET_ACCESS_KEY`)     |
| `timeout`           | number | no       | per-call shell timeout (seconds) |

### Read-only operations

- `rustic.snapshots(opts)` → `[snapshot] | nil, err` — List every snapshot in the repository.
  Returns the JSON array rustic emits.
- `rustic.snapshot_detail(opts, id)` → `snapshot | nil, err` — Detail for one snapshot.
- `rustic.check(opts)` → `{ok=true} | nil, err` — Verify connectivity + repository integrity.
  Read-only; safe to call from a status probe.

### Mutating operations

- `rustic.init(opts)` → `{ok=true, stdout} | nil, err` — Initialise a fresh repository.
- `rustic.backup(opts, args)` → `{ok=true, summary?} | nil, err` — Run a backup.
  `args = { sources = {…}, tags = {…}, exclude = {…}, json = bool, timeout = N }`. When
  `args.json = true` the rustic summary is parsed and exposed on the return.
- `rustic.restore(opts, id, target, args)` → `{ok=true} | nil, err` — Restore snapshot `id` into
  directory `target`. `args.dry_run = true` for a no-write preview.
- `rustic.forget(opts, args)` → `{ok=true, removed?} | nil, err` — Apply a retention policy.
  `args = { keep_daily, keep_weekly, keep_monthly, keep_yearly, keep_hourly, keep_last, tags = {…}, prune = bool, json = bool }`.
  `prune` additionally frees space (slower).

### Errors

Failures return `nil, err_string`. The error includes the rustic command name, exit code, and the
last stderr line. Functions never raise — callers chain on the second return.

### Crash-consistent capture

For btrfs / zfs sources the canonical pattern is to bracket `backup` with
[`assay.fs_snapshot`](fs_snapshot.md):

```lua
local snap = require("assay.fs_snapshot")
snap.with_snapshot("manual", "/var/lib/machines", function(handle)
  rustic.backup(opts, {
    sources = { handle.path .. "/agentx" },
    tags    = { "host", "daily" },
    json    = true,
  })
end)
```

The `with_snapshot` wrapper releases the snapshot even if `backup` errors.

### Integration with hostops

`libs/hostops/services/host/backups.lua` drives this stdlib through the hostops `/backups` dashboard
(snapshot list, last-run marker, manual runs, restore). When `assay-ops` ships, fleet-level
orchestration (scheduling across hosts, retention policy fan-out) will live there — this stdlib
stays the single host-side primitive both layers call.
