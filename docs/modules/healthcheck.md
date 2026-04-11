## assay.healthcheck

HTTP health checking utilities. Status codes, JSON path, body matching, latency, multi-check.
Module-level functions (no client needed): `M.function(url, ...)`.

- `M.http(url, opts?)` → `{ok, status, latency_ms, error?}` — HTTP health check. `opts`: `{expected_status, method, body, headers}`. Default expects 200.
- `M.json_path(url, path_expr, expected, opts?)` → `{ok, actual, expected, error?}` — Check JSON response field. Dot-notation path: `"data.status"`.
- `M.status_code(url, expected, opts?)` → `{ok, status, error?}` — Check specific HTTP status code
- `M.body_contains(url, pattern, opts?)` → `{ok, found, error?}` — Check if response body contains literal pattern
- `M.endpoint(url, opts?)` → `{ok, status, latency_ms, error?}` — Check status and latency. `opts`: `{max_latency_ms, expected_status, headers}`
- `M.multi(checks)` → `{ok, results, passed, failed, total}` — Run multiple checks. `checks`: `[{name, check=function}]`
- `M.wait(url, opts?)` → `{ok, status, attempts}` — Wait for endpoint to become healthy. `opts`: `{timeout, interval, expect_status, headers}`. Default 60s timeout.

Example:
```lua
local hc = require("assay.healthcheck")
local result = hc.multi({
  {name = "api", check = function() return hc.http("http://api:8080/health") end},
  {name = "db-field", check = function() return hc.json_path("http://api:8080/health", "database", "ok") end},
  {name = "latency", check = function() return hc.endpoint("http://api:8080/health", {max_latency_ms = 500}) end},
})
assert.eq(result.ok, true, result.failed .. " health checks failed")
```
