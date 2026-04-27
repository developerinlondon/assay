---
category: Linux & systemd
---

## assay.system

Umbrella stdlib that re-exports the host introspection and control surface (`linux`, `cgroup`,
`systemd`, `assay.cron`) under a single `require`, plus three convenience aggregates that span
sub-modules. Plan 18 / v0.15.1.

```lua
local sys = require("assay.system")
```

### Direct passthrough

```lua
sys.linux.cpu_stat()
sys.cgroup.memory(path)
sys.systemd.list_machines()
sys.cron.all()
```

Both call shapes work — `linux.cpu_stat()` (low-level global) and `sys.linux.cpu_stat()` (umbrella)
hit the same machine code. Pick the one that reads better in your script.

### Convenience aggregates

- `sys.host_snapshot()` — `{cpu, mem, load, uptime, netdev, kernel}`. One call gives you everything
  a host-vitals dashboard tile needs.
- `sys.machine_snapshot(name)` — `{info, cgroup={cpu,memory,io,pids}, journal_tail}` for one
  systemd-nspawn container. Combines `systemd.machine_status` + the four `cgroup.*` reads + the last
  20 journal lines for the machine.
- `sys.machines()` — `systemd.list_machines()` with each row enriched by its cgroup utilisation
  snapshot. Drives a multi-container overview without the caller stitching the joins by hand.

Every aggregate uses `pcall` internally so a single missing field degrades to `nil` rather than
aborting the whole snapshot.
