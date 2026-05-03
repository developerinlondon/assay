---
category: Infrastructure
tagline: btrfs / zfs subvolume snapshot wrapper for crash-consistent backup capture (require("assay.fs_snapshot"), v0.15.7+)
---

## assay.fs_snapshot

Lua stdlib for taking read-only filesystem snapshots before a backup read. Auto-selects a backend by
inspecting the source's mount point:

| Backend | Detected when           | Snapshot mechanism                                 |
| ------- | ----------------------- | -------------------------------------------------- |
| `btrfs` | `findmnt` reports btrfs | `btrfs subvolume snapshot -r <subvol> <snap_path>` |
| `zfs`   | `findmnt` reports zfs   | `zfs snapshot <pool>/<dataset>@<id>`               |
| `none`  | any other fs            | no snapshot — caller reads live                    |

```lua
local snap = require("assay.fs_snapshot")

-- Bracket pattern (preferred) — releases on error.
snap.with_snapshot("manual", "/var/lib/machines", function(handle)
  -- handle.path points at the read-only snapshot view (or the live path
  -- on the `none` backend). Pass it to your read flow.
  do_backup(handle.path)
end)
```

### Functions

- `fs_snapshot.detect(path)` → `{backend, source, fstype}` — Identify the FS backing `path`. Calls
  `findmnt` once.
- `fs_snapshot.take(name, path)` → `handle` — Take a read-only snapshot. The handle is an opaque
  table the caller MUST pass back to `release`. Errors via `error()` if the snapshot command fails.
- `fs_snapshot.release(handle)` → `{ok=true} | {ok=false, error}` — Release a snapshot handle. No-op
  for `none` backend.
- `fs_snapshot.with_snapshot(name, path, fn)` → result-of-fn — Convenience: bracket `fn(handle)`
  between `take` + `release`. Releases even when `fn` errors; the error then propagates to the
  caller.

### Handle shape

```
{
  backend     = "btrfs" | "zfs" | "none",
  path        = "/var/lib/machines/.assay-snap-manual-1730500800",
  source_path = "/var/lib/machines",
  -- zfs only:
  snap_ref    = "tank/data@manual-1730500800",
}
```

The `path` field is what callers read from — the read-only snapshot view on btrfs/zfs, or the
original `path` on the `none` backend.

### Privilege model

`btrfs` / `zfs` commands typically need root. The stdlib detects whether the running uid is 0 and
prepends `sudo -n` when not, so a host with a sudoers rule for `btrfs subvolume snapshot/delete` and
`zfs
snapshot/destroy` works out of the box.

### Integration with assay.rustic

The canonical use is bracketing a [`assay.rustic`](rustic.md) backup call so the capture is
crash-consistent:

```lua
snap.with_snapshot("daily", "/var/lib/machines", function(h)
  rustic.backup(repo_opts, {
    sources = { h.path .. "/agentx", h.path .. "/web" },
    tags    = { "host", "daily" },
    json    = true,
  })
end)
```
