## assay.loki

Loki log aggregation. Push logs, query with LogQL, labels, series, tail.
Client: `loki.client(url)`. Module helper: `M.selector(labels)`.

- `M.selector(labels)` → string — Build LogQL stream selector from labels table. `M.selector({app="nginx"})` → `{app="nginx"}`
- `c.logs:push(stream_labels, entries)` → true — Push log entries. `entries`: array of strings or `{timestamp, line}` pairs
- `c.queries:instant(logql, opts?)` → [result] — Instant LogQL query. `opts`: `{limit, time, direction}`
- `c.queries:range(logql, opts?)` → [result] — Range LogQL query. `opts`: `{start, end_time, limit, step, direction}`
- `c.queries:tail(logql, opts?)` → data — Tail log stream. `opts`: `{limit, start}`
- `c.labels:list(opts?)` → [string] — List label names. `opts`: `{start, end_time}`
- `c.labels:values(label_name, opts?)` → [string] — List values for a label. `opts`: `{start, end_time}`
- `c.series:list(match_selectors, opts?)` → [series] — Query series metadata. `opts`: `{start, end_time}`
- `c.health:ready()` → bool — Check Loki readiness
- `c.health:metrics()` → string — Get Loki metrics in Prometheus exposition format

Example:
```lua
local loki = require("assay.loki")
local c = loki.client("http://loki:3100")
c.logs:push({app="myservice", env="prod"}, {"Request processed", "Job complete"})
local logs = c.queries:instant('{app="myservice"}', {limit = 10})
```
