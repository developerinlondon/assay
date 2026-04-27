---
category: Builtins
---

## linux

`/proc` and `/sys/...` readers exposed as a Rust builtin. Linux-only — the table is registered empty
on macOS and Windows. Plan 18 / v0.15.1.

Backed by the `procfs` crate; numbers are bytes (kB fields are converted in Rust so callers don't
have to).

### Host

- `linux.kernel()` → `{version, hostname, os_release, btime}`
- `linux.uptime()` → `{uptime_secs, idle_secs}`
- `linux.loadavg()` → `{one, five, fifteen, running, total, last_pid}`
- `linux.meminfo()` → `{total, free, available, buffers, cached, swap_total, swap_free, …}`
- `linux.netdev()` → per-interface RX/TX counters
- `linux.diskstats()` → per-block-device IO stats

### CPU

- `linux.cpu_stat()` → aggregate `/proc/stat` first row, jiffies
  (`{user, nice, system, idle, iowait, irq, softirq, steal, guest, guest_nice}`)
- `linux.cpu_stat_per_core()` → list of per-CPU rows
- `linux.cpu_percent(prev, curr)` → `{total_pct}` — Lua-side delta math, no kernel call

### Per-process

- `linux.proc_stat(pid)` → `/proc/<pid>/stat` (state, ppid, utime, stime, vsize, rss_pages,
  num_threads, …)
- `linux.proc_status(pid)` → `/proc/<pid>/status` (name, uid, gid, vm_rss, vm_size, threads, …)

Errors surface as `linux.X: …` runtime errors. The `process.list` builtin gives you a
names-and-cmdlines view; combine it with `linux.proc_stat` for a top-style readout.
