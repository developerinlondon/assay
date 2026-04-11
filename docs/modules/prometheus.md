## assay.prometheus

Prometheus monitoring queries. PromQL instant/range queries, alerts, targets, rules, series.
Client: `prometheus.client(url)`.

- `c.queries:instant(promql)` → number|[{metric, value}] — Instant PromQL query. Single result returns number, multiple returns array.
- `c.queries:range(promql, start_time, end_time, step)` → [result] — Range PromQL query over time window
- `c.alerts:list()` → [alert] — List active alerts
- `c.targets:list()` → `{activeTargets, droppedTargets}` — List scrape targets with health status
- `c.targets:metadata(opts?)` → [metadata] — Get targets metadata. `opts`: `{match_target, metric, limit}`
- `c.rules:list(opts?)` → [group] — List alerting/recording rules. `opts.type` filters by `"alert"` or `"record"`.
- `c.labels:values(label_name)` → [string] — List all values for a label name
- `c.series:list(match_selectors)` → [series] — Query series metadata. `match_selectors` is array of selectors.
- `c.config:reload()` → bool — Trigger Prometheus configuration reload via `/-/reload`

Example:
```lua
local prom = require("assay.prometheus")
local c = prom.client("http://prometheus:9090")
local count = c.queries:instant("count(up)")
assert.gt(count, 0, "No targets up")
```
