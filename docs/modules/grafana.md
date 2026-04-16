## assay.grafana

Grafana monitoring and dashboards. Health, datasources, annotations, alerts, folders. Client:
`grafana.client(url, {api_key="..."})` or `{username="...", password="..."}`.

- `c:health()` → `{database, version, commit}` — Check Grafana server health
- `c:datasources()` → `[{id, name, type, url}]` — List all datasources
- `c:datasource(id_or_uid)` → `{id, name, type, ...}` — Get datasource by numeric ID or string UID
- `c:search(opts?)` → `[{id, title, type}]` — Search dashboards/folders. `opts`:
  `{query, type, tag, limit}`
- `c:dashboard(uid)` → `{dashboard, meta}` — Get dashboard by UID
- `c:annotations(opts?)` → `[{id, text, time}]` — List annotations. `opts`:
  `{from, to, dashboard_id, limit, tags}`
- `c:create_annotation(annotation)` → `{id}` — Create annotation. `annotation`:
  `{text, dashboardId?, tags?}`
- `c:org()` → `{id, name}` — Get current organization
- `c:alert_rules()` → `[{uid, title}]` — List provisioned alert rules
- `c:folders()` → `[{id, uid, title}]` — List all folders

Example:

```lua
local grafana = require("assay.grafana")
local c = grafana.client("http://grafana:3000", {api_key = "glsa_..."})
local h = c:health()
assert.eq(h.database, "ok")
```
