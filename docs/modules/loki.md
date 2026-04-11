## assay.loki

Loki log aggregation. Push logs, query with LogQL, labels, series, tail.
Client: `loki.client(url)`. Module helper: `M.selector(labels)`.

- `M.selector(labels)` → string — Build LogQL stream selector from labels table. `M.selector({app="nginx"})` → `{app="nginx"}`
- `c:push(stream_labels, entries)` → true — Push log entries. `entries`: array of strings or `{timestamp, line}` pairs
- `c:query(logql, opts?)` → [result] — Instant LogQL query. `opts`: `{limit, time, direction}`
- `c:query_range(logql, opts?)` → [result] — Range LogQL query. `opts`: `{start, end_time, limit, step, direction}`
- `c:labels(opts?)` → [string] — List label names. `opts`: `{start, end_time}`
- `c:label_values(label_name, opts?)` → [string] — List values for a label. `opts`: `{start, end_time}`
- `c:series(match_selectors, opts?)` → [series] — Query series metadata. `opts`: `{start, end_time}`
- `c:tail(logql, opts?)` → data — Tail log stream. `opts`: `{limit, start}`
- `c:ready()` → bool — Check Loki readiness
- `c:metrics()` → string — Get Loki metrics in Prometheus exposition format

Example:
```lua
local loki = require("assay.loki")
local c = loki.client("http://loki:3100")
c:push({app="myservice", env="prod"}, {"Request processed", "Job complete"})
local logs = c:query('{app="myservice"}', {limit = 10})
```
