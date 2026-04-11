## assay.argocd

ArgoCD GitOps application management. Apps, sync, health, projects, repositories, clusters.
Client: `argocd.client(url, {token="..."})` or `{username="...", password="..."}`.

- `c:applications(opts?)` → [app] — List applications. `opts`: `{project, selector}`
- `c:application(name)` → app — Get application by name
- `c:app_health(name)` → `{status, sync, message}` — Get app health and sync status
- `c:sync(name, opts?)` → result — Trigger sync. `opts`: `{revision, prune, dry_run, strategy}`
- `c:refresh(name, opts?)` → app — Refresh app state. `opts.type`: `"normal"` (default) or `"hard"`
- `c:rollback(name, id)` → result — Rollback to history ID
- `c:app_resources(name)` → resource_tree — Get application resource tree
- `c:app_manifests(name, opts?)` → manifests — Get manifests. `opts`: `{revision}`
- `c:delete_app(name, opts?)` → nil — Delete app. `opts`: `{cascade, propagation_policy}`
- `c:projects()` → [project] — List projects
- `c:project(name)` → project — Get project by name
- `c:repositories()` → [repo] — List repositories
- `c:repository(repo_url)` → repo — Get repository by URL
- `c:clusters()` → [cluster] — List clusters
- `c:cluster(server_url)` → cluster — Get cluster by server URL
- `c:settings()` → settings — Get ArgoCD settings
- `c:version()` → version — Get ArgoCD version info
- `c:is_healthy(name)` → bool — Check if app health status is "Healthy"
- `c:is_synced(name)` → bool — Check if app sync status is "Synced"
- `c:wait_healthy(name, timeout_secs)` → true — Wait for app to become healthy, errors on timeout
- `c:wait_synced(name, timeout_secs)` → true — Wait for app to become synced, errors on timeout

Example:
```lua
local argocd = require("assay.argocd")
local c = argocd.client("https://argocd.example.com", {token = env.get("ARGOCD_TOKEN")})
c:sync("my-app", {prune = true})
c:wait_healthy("my-app", 120)
```
