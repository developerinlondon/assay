---
category: Linux & systemd
---

## assay.cron

Pure-Lua scheduled-job inspector. Walks the conventional cron locations plus systemd timers (via the
`systemd` builtin) and returns a unified view. Plan 18 / v0.15.1.

```lua
local cron = require("assay.cron")
```

- `cron.system_crontab()` — parses `/etc/crontab` and every file in `/etc/cron.d/`. Returns rows
  with `{source, schedule, user, command, raw}`.
- `cron.user_crontabs()` — parses `/var/spool/cron/crontabs/*` (root-readable) into
  `{user = [rows], …}`.
- `cron.daily_dropins()` — directory listings of `/etc/cron.{hourly,daily,weekly,monthly}/`.
- `cron.timers()` — thin passthrough to `systemd.list_timers()`. Errors with a hint if the systemd
  builtin isn't loaded.
- `cron.all()` — unified
  `[{kind, source, schedule, command, user?, next_fire?, last_fire?, raw?}, …]` across every source
  above. `next_fire` / `last_fire` populate only for systemd timers (cron jobs would need a full
  schedule evaluator to compute these reliably).

The crontab parser handles 5-field user crontabs, 6-field system crontabs (with the user column),
and the `@reboot` / `@daily` / `@hourly` / `@weekly` / `@monthly` / `@yearly` / `@midnight`
shorthand.

Environment-assignment lines (`SHELL=/bin/sh`) are skipped; comment lines and blank lines are
ignored.
