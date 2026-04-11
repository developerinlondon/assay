## assay.flux

Flux CD GitOps toolkit. GitRepositories, Kustomizations, HelmReleases, notifications, image automation.
Client: `flux.client(url, token)`.

- `c:git_repositories(namespace)` → `{items}` — List GitRepositories
- `c:git_repository(namespace, name)` → repo|nil — Get GitRepository by name (nil if 404)
- `c:is_git_repo_ready(namespace, name)` → bool — Check if GitRepository has Ready=True condition
- `c:helm_repositories(namespace)` → `{items}` — List HelmRepositories
- `c:helm_repository(namespace, name)` → repo|nil — Get HelmRepository by name
- `c:is_helm_repo_ready(namespace, name)` → bool — Check if HelmRepository is ready
- `c:helm_charts(namespace)` → `{items}` — List HelmCharts
- `c:oci_repositories(namespace)` → `{items}` — List OCIRepositories
- `c:kustomizations(namespace)` → `{items}` — List Kustomizations
- `c:kustomization(namespace, name)` → ks|nil — Get Kustomization by name
- `c:is_kustomization_ready(namespace, name)` → bool — Check if Kustomization is ready
- `c:kustomization_status(namespace, name)` → `{ready, revision, last_applied_revision, conditions}`|nil — Get status
- `c:helm_releases(namespace)` → `{items}` — List HelmReleases
- `c:helm_release(namespace, name)` → hr|nil — Get HelmRelease by name
- `c:is_helm_release_ready(namespace, name)` → bool — Check if HelmRelease is ready
- `c:helm_release_status(namespace, name)` → `{ready, revision, last_applied_revision, conditions}`|nil — Get status
- `c:alerts(namespace)` → `{items}` — List notification alerts
- `c:providers_list(namespace)` → `{items}` — List notification providers
- `c:receivers(namespace)` → `{items}` — List notification receivers
- `c:image_policies(namespace)` → `{items}` — List image automation policies
- `c:all_sources_ready(namespace)` → `{ready, not_ready, total, not_ready_names}` — Check all Git+Helm sources
- `c:all_kustomizations_ready(namespace)` → `{ready, not_ready, total, not_ready_names}` — Check all Kustomizations
- `c:all_helm_releases_ready(namespace)` → `{ready, not_ready, total, not_ready_names}` — Check all HelmReleases

Example:
```lua
local flux = require("assay.flux")
local c = flux.client("https://k8s-api:6443", env.get("K8S_TOKEN"))
local status = c:all_kustomizations_ready("flux-system")
assert.eq(status.not_ready, 0, "Some Kustomizations not ready: " .. table.concat(status.not_ready_names, ", "))
```
