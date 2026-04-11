## assay.harbor

Harbor container registry. Projects, repositories, artifacts, vulnerability scanning.
Client: `harbor.client(url, {api_key="..."})` or `{username="...", password="..."}`.

### System (`c.system`)

- `c.system:health()` → `{status, components}` — Check Harbor health
- `c.system:info()` → `{harbor_version, ...}` — Get system information
- `c.system:statistics()` → `{private_project_count, ...}` — Get registry statistics
- `c.system:is_healthy()` → bool — Check if all components report "healthy"

### Projects (`c.projects`)

- `c.projects:list(opts?)` → [project] — List projects. `opts`: `{name, public, page, page_size}`
- `c.projects:get(name_or_id)` → project — Get project by name or numeric ID

### Repositories (`c.repositories`)

- `c.repositories:list(project_name, opts?)` → [repo] — List repos. `opts`: `{page, page_size, q}`
- `c.repositories:get(project_name, repo_name)` → repo — Get repository

### Artifacts (`c.artifacts`)

- `c.artifacts:list(project_name, repo_name, opts?)` → [artifact] — List artifacts. `opts`: `{page, page_size, with_tag, with_scan_overview}`
- `c.artifacts:get(project_name, repo_name, reference)` → artifact — Get artifact by tag or digest
- `c.artifacts:tags(project_name, repo_name, reference)` → [tag] — List artifact tags
- `c.artifacts:exists(project_name, repo_name, tag)` → bool — Check if image tag exists
- `c.artifacts:latest(project_name, repo_name)` → artifact|nil — Get most recent artifact

### Scan (`c.scan`)

- `c.scan:trigger(project_name, repo_name, reference)` → true — Trigger vulnerability scan (async)
- `c.scan:vulnerabilities(project_name, repo_name, reference)` → `{total, fixable, critical, high, medium, low, negligible}`|nil — Get vulnerability summary

### Replication (`c.replication`)

- `c.replication:policies()` → [policy] — List replication policies
- `c.replication:executions(opts?)` → [execution] — List replication executions. `opts`: `{policy_id}`

### Backward Compatibility

All legacy colon-style methods (`c:health()`, `c:projects()`, `c:artifacts()`, etc.) remain available and delegate to the sub-objects above.

Example:
```lua
local harbor = require("assay.harbor")
local c = harbor.client("https://harbor.example.com", {username = "admin", password = env.get("HARBOR_PASS")})

-- New sub-object style
assert.eq(c.system:is_healthy(), true, "Harbor unhealthy")
c.scan:trigger("myproject", "myapp", "latest")
sleep(30)
local vulns = c.scan:vulnerabilities("myproject", "myapp", "latest")
assert.eq(vulns.critical, 0, "Critical vulnerabilities found!")

-- Legacy style still works
assert.eq(c:is_healthy(), true, "Harbor unhealthy")
c:scan_artifact("myproject", "myapp", "latest")
```
