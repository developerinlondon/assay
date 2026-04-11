## assay.harbor

Harbor container registry. Projects, repositories, artifacts, vulnerability scanning.
Client: `harbor.client(url, {api_key="..."})` or `{username="...", password="..."}`.

### System

- `c:health()` → `{status, components}` — Check Harbor health
- `c:system_info()` → `{harbor_version, ...}` — Get system information
- `c:statistics()` → `{private_project_count, ...}` — Get registry statistics
- `c:is_healthy()` → bool — Check if all components report "healthy"

### Projects

- `c:projects(opts?)` → [project] — List projects. `opts`: `{name, public, page, page_size}`
- `c:project(name_or_id)` → project — Get project by name or numeric ID

### Repositories & Artifacts

- `c:repositories(project_name, opts?)` → [repo] — List repos. `opts`: `{page, page_size, q}`
- `c:repository(project_name, repo_name)` → repo — Get repository
- `c:artifacts(project_name, repo_name, opts?)` → [artifact] — List artifacts. `opts`: `{page, page_size, with_tag, with_scan_overview}`
- `c:artifact(project_name, repo_name, reference)` → artifact — Get artifact by tag or digest
- `c:artifact_tags(project_name, repo_name, reference)` → [tag] — List artifact tags
- `c:image_exists(project_name, repo_name, tag)` → bool — Check if image tag exists
- `c:latest_artifact(project_name, repo_name)` → artifact|nil — Get most recent artifact

### Vulnerability Scanning

- `c:scan_artifact(project_name, repo_name, reference)` → true — Trigger vulnerability scan (async)
- `c:artifact_vulnerabilities(project_name, repo_name, reference)` → `{total, fixable, critical, high, medium, low, negligible}`|nil — Get vulnerability summary

### Replication

- `c:replication_policies()` → [policy] — List replication policies
- `c:replication_executions(opts?)` → [execution] — List replication executions. `opts`: `{policy_id}`

Example:
```lua
local harbor = require("assay.harbor")
local c = harbor.client("https://harbor.example.com", {username = "admin", password = env.get("HARBOR_PASS")})
assert.eq(c:is_healthy(), true, "Harbor unhealthy")
c:scan_artifact("myproject", "myapp", "latest")
sleep(30)
local vulns = c:artifact_vulnerabilities("myproject", "myapp", "latest")
assert.eq(vulns.critical, 0, "Critical vulnerabilities found!")
```
