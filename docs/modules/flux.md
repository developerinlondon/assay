## assay.flux

Flux CD GitOps toolkit. GitRepositories, Kustomizations, HelmReleases, notifications, image automation.
Client: `flux.client(url, token)`.

### Git Repositories

- `c.git_repos:list(namespace)` -> `{items}` -- List GitRepositories
- `c.git_repos:get(namespace, name)` -> repo|nil -- Get GitRepository by name (nil if 404)
- `c.git_repos:is_ready(namespace, name)` -> bool -- Check if GitRepository has Ready=True condition

### Helm Repositories

- `c.helm_repos:list(namespace)` -> `{items}` -- List HelmRepositories
- `c.helm_repos:get(namespace, name)` -> repo|nil -- Get HelmRepository by name
- `c.helm_repos:is_ready(namespace, name)` -> bool -- Check if HelmRepository is ready

### Helm Charts

- `c.helm_charts:list(namespace)` -> `{items}` -- List HelmCharts

### OCI Repositories

- `c.oci_repos:list(namespace)` -> `{items}` -- List OCIRepositories

### Kustomizations

- `c.kustomizations:list(namespace)` -> `{items}` -- List Kustomizations
- `c.kustomizations:get(namespace, name)` -> ks|nil -- Get Kustomization by name
- `c.kustomizations:is_ready(namespace, name)` -> bool -- Check if Kustomization is ready
- `c.kustomizations:status(namespace, name)` -> `{ready, revision, last_applied_revision, conditions}`|nil -- Get status
- `c.kustomizations:all_ready(namespace)` -> `{ready, not_ready, total, not_ready_names}` -- Check all Kustomizations

### Helm Releases

- `c.helm_releases:list(namespace)` -> `{items}` -- List HelmReleases
- `c.helm_releases:get(namespace, name)` -> hr|nil -- Get HelmRelease by name
- `c.helm_releases:is_ready(namespace, name)` -> bool -- Check if HelmRelease is ready
- `c.helm_releases:status(namespace, name)` -> `{ready, revision, last_applied_revision, conditions}`|nil -- Get status
- `c.helm_releases:all_ready(namespace)` -> `{ready, not_ready, total, not_ready_names}` -- Check all HelmReleases

### Notifications

- `c.notifications:alerts(namespace)` -> `{items}` -- List notification alerts
- `c.notifications:providers(namespace)` -> `{items}` -- List notification providers
- `c.notifications:receivers(namespace)` -> `{items}` -- List notification receivers

### Image Policies

- `c.image_policies:list(namespace)` -> `{items}` -- List image automation policies

### Sources (aggregate)

- `c.sources:all_ready(namespace)` -> `{ready, not_ready, total, not_ready_names}` -- Check all Git+Helm sources

Example:
```lua
local flux = require("assay.flux")
local c = flux.client("https://k8s-api:6443", env.get("K8S_TOKEN"))
local status = c.kustomizations:all_ready("flux-system")
assert.eq(status.not_ready, 0, "Some Kustomizations not ready: " .. table.concat(status.not_ready_names, ", "))
```
