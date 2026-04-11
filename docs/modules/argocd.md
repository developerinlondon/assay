## assay.argocd

ArgoCD GitOps application management. Apps, sync, health, projects, repositories, clusters.
Client: `argocd.client(url, {token="..."})` or `{username="...", password="..."}`.

- `c.apps:list(opts?)` → [app] — List applications. `opts`: `{project, selector}`
- `c.apps:get(name)` → app — Get application by name
- `c.apps:health(name)` → `{status, sync, message}` — Get app health and sync status
- `c.apps:sync(name, opts?)` → result — Trigger sync. `opts`: `{revision, prune, dry_run, strategy}`
- `c.apps:refresh(name, opts?)` → app — Refresh app state. `opts.type`: `"normal"` (default) or `"hard"`
- `c.apps:rollback(name, id)` → result — Rollback to history ID
- `c.apps:resources(name)` → resource_tree — Get application resource tree
- `c.apps:manifests(name, opts?)` → manifests — Get manifests. `opts`: `{revision}`
- `c.apps:delete(name, opts?)` → nil — Delete app. `opts`: `{cascade, propagation_policy}`
- `c.apps:is_healthy(name)` → bool — Check if app health status is "Healthy"
- `c.apps:is_synced(name)` → bool — Check if app sync status is "Synced"
- `c.apps:wait_healthy(name, timeout_secs)` → true — Wait for app to become healthy, errors on timeout
- `c.apps:wait_synced(name, timeout_secs)` → true — Wait for app to become synced, errors on timeout
- `c.projects:list()` → [project] — List projects
- `c.projects:get(name)` → project — Get project by name
- `c.repositories:list()` → [repo] — List repositories
- `c.repositories:get(repo_url)` → repo — Get repository by URL
- `c.clusters:list()` → [cluster] — List clusters
- `c.clusters:get(server_url)` → cluster — Get cluster by server URL
- `c.settings:get()` → settings — Get ArgoCD settings
- `c:version()` → version — Get ArgoCD version info

Example:
```lua
local argocd = require("assay.argocd")
local c = argocd.client("https://argocd.example.com", {token = env.get("ARGOCD_TOKEN")})
c.apps:sync("my-app", {prune = true})
c.apps:wait_healthy("my-app", 120)
```
