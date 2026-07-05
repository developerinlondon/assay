---
category: Monitoring & Observability
---

## assay.sonarqube

SonarQube web API client for code quality data — quality gates, issues, security hotspots, measures,
and projects. Client: `sonarqube.client(url, {token="..."})` (bearer) or
`sonarqube.client(url, {user="...", password="..."})` (basic). All methods are read-only.

### Quality Gates

- `c.qualitygate:project_status(project_key)` → `{projectStatus}`|nil — Quality gate status for a
  project (`GET /api/qualitygates/project_status`). Returns nil on 404.

### Issues

- `c.issues:search(opts?)` → `{total, issues, ...}` — Search issues (`GET /api/issues/search`).
  `opts`: `{component_keys, types, severities, statuses, resolved, page_size, page}`.

### Hotspots

- `c.hotspots:search(opts?)` → `{hotspots, paging}` — Search security hotspots
  (`GET /api/hotspots/search`). `opts`: `{project_key, status, resolution, page_size, page}`.

### Measures

- `c.measures:component(component, metric_keys)` → `{component}` — Component measures for the given
  metric keys (`GET /api/measures/component`). `metric_keys` may be a comma-separated string or a
  list of strings.

### Projects

- `c.projects:search(opts?)` → `{components, paging}` — Search projects
  (`GET /api/projects/search`). `opts`: `{query, qualifiers, page_size, page}`.

### Mutation

None. Every method is a read (`http.get`). Quality-gate and project administration writes are out of
scope for this module.

Example:

```lua
local sonarqube = require("assay.sonarqube")
local c = sonarqube.client("https://sonar.example.com", { token = env.get("SONAR_TOKEN") })

local gate = c.qualitygate:project_status("demo-project")
assert.eq(gate.projectStatus.status, "OK")

local issues = c.issues:search({ component_keys = "demo-project", severities = "BLOCKER" })
print(issues.total)

local measures = c.measures:component("demo-project", { "coverage", "bugs", "vulnerabilities" })
for _, m in ipairs(measures.component.measures) do
  print(m.metric, m.value)
end
```
