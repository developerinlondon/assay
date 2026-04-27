---
category: Builtins
---

## systemd

D-Bus client for `org.freedesktop.systemd1` and `org.freedesktop.machine1`, plus a one-shot journal
reader. Linux-only — the table is empty on other platforms. Plan 18 / v0.15.1.

Backed by `zbus` 5 (async); a single system-bus connection is opened lazily on first call and cached
for the lifetime of the Lua VM. Every public function is async at the Rust level; mlua drives them
as Lua coroutines so callers write straight-line code.

### Units

- `systemd.list_units(filter?)` — `filter` is a glob like `"*.service"` or `"*.timer"`; nil for all.
- `systemd.unit_status(name)` — full property dict including `since`, `main_pid`,
  `exec_main_status`, `fragment_path`.
- `systemd.is_active(name)` → boolean
- `systemd.list_timers()` → list of timer rows with `next_elapse_realtime` / `last_trigger_realtime`
  / `passed` / `activates`.
- `systemd.start(name)` / `stop(name)` / `restart(name)` / `reload(name)` — return the job object
  path.

### Machines (systemd-machined)

- `systemd.list_machines()` → `[{name, class, service, leader_pid, addresses, root_directory}, …]`
- `systemd.machine_status(name)` — full per-machine dict
- `systemd.machine_start(name)` / `machine_poweroff(name)` / `machine_reboot(name)` /
  `machine_terminate(name)`

### Journal

- `systemd.journal({unit?, machine?, since?, until?, lines?, priority?})` — one-shot read of the
  most recent N entries. Implementation shells out to `journalctl --output=json`; subprocess is
  on-demand only.
- `systemd.journal_follow(opts, fn)` — not yet implemented; returns an explicit runtime error.
  Tracked as a Phase 3 follow-up (sd_journal_wait + cancellation handle across the FFI boundary).

### Permissions

Lifecycle methods (`start`, `stop`, machine lifecycle) require the calling process to have polkit
authorisation for the relevant D-Bus interface. Read-only queries (`list_*`, `unit_status`,
`is_active`) work for any caller with system-bus access.
