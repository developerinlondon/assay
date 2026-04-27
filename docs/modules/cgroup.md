---
category: Builtins
---

## cgroup

cgroup v2 unified-hierarchy readers. Linux-only — the table is empty on other platforms. Plan 18 /
v0.15.1.

Pure `std::fs` reads with path canonicalisation; every input path is required to start with
`/sys/fs/cgroup/` after symlink resolution. Cgroup v2's `"max"` sentinel maps to Lua `nil` so
callers can write `if not mem.max then ...` instead of comparing strings.

- `cgroup.version()` → `"v2"` | `"v1"` | `"hybrid"`
- `cgroup.list(slice_path)` → child cgroup directory names (filtered to entries that actually
  contain a `cgroup.controllers` file). Sorted.
- `cgroup.cpu_stat(path)` →
  `{usage_usec, user_usec, system_usec, nr_periods, nr_throttled, throttled_usec, nr_bursts, burst_usec}`
- `cgroup.memory(path)` → `{current, max, swap_current, swap_max, peak, low, high, oom_kill, oom}`
- `cgroup.io(path)` → `[{device, rbytes, wbytes, rios, wios, dbytes, dios}, …]`
- `cgroup.pids(path)` → `{current, max}`
- `cgroup.procs(path)` → list of pids in this cgroup

Typical usage: walk every container slice under `/sys/fs/cgroup/machine.slice`, read `cpu_stat` and
`memory` once per refresh cycle, render a dashboard.
