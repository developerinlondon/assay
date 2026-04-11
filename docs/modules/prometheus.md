## assay.prometheus

Prometheus monitoring queries. PromQL instant/range queries, alerts, targets, rules, series.
Module-level functions (no client needed): `M.function(url, ...)`.

- `M.query(url, promql)` → number|[{metric, value}] — Instant PromQL query. Single result returns number, multiple returns array.
- `M.query_range(url, promql, start_time, end_time, step)` → [result] — Range PromQL query over time window
- `M.alerts(url)` → [alert] — List active alerts
- `M.targets(url)` → `{activeTargets, droppedTargets}` — List scrape targets with health status
- `M.rules(url, opts?)` → [group] — List alerting/recording rules. `opts.type` filters by `"alert"` or `"record"`.
- `M.label_values(url, label_name)` → [string] — List all values for a label name
- `M.series(url, match_selectors)` → [series] — Query series metadata. `match_selectors` is array of selectors.
- `M.config_reload(url)` → bool — Trigger Prometheus configuration reload via `/-/reload`
- `M.targets_metadata(url, opts?)` → [metadata] — Get targets metadata. `opts`: `{match_target, metric, limit}`

Example:
```lua
local prom = require("assay.prometheus")
local count = prom.query("http://prometheus:9090", "count(up)")
assert.gt(count, 0, "No targets up")
```
