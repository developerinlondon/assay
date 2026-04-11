## assay.crossplane

Crossplane infrastructure management. Providers, XRDs, compositions, managed resources.
Client: `crossplane.client(url, token)`.

### Providers

- `c:providers()` → `{items}` — List all providers
- `c:provider(name)` → provider|nil — Get provider by name
- `c:is_provider_healthy(name)` → bool — Check if provider has Healthy=True condition
- `c:is_provider_installed(name)` → bool — Check if provider has Installed=True condition
- `c:provider_status(name)` → `{installed, healthy, current_revision, conditions}` — Get full provider status
- `c:provider_revisions()` → `{items}` — List provider revisions
- `c:provider_revision(name)` → revision|nil — Get provider revision by name

### Configurations

- `c:configurations()` → `{items}` — List configurations
- `c:configuration(name)` → config|nil — Get configuration by name
- `c:is_configuration_healthy(name)` → bool — Check if configuration is healthy
- `c:is_configuration_installed(name)` → bool — Check if configuration is installed

### Functions

- `c:functions()` → `{items}` — List composition functions
- `c:xfunction(name)` → function|nil — Get function by name
- `c:is_function_healthy(name)` → bool — Check if function is healthy

### Composite Resource Definitions (XRDs)

- `c:xrds()` → `{items}` — List all XRDs
- `c:xrd(name)` → xrd|nil — Get XRD by name
- `c:is_xrd_established(name)` → bool — Check if XRD has Established=True condition

### Compositions

- `c:compositions()` → `{items}` — List all compositions
- `c:composition(name)` → composition|nil — Get composition by name

### Managed Resources

- `c:managed_resource(api_group, version, kind, name)` → resource|nil — Get managed resource
- `c:is_managed_ready(api_group, version, kind, name)` → bool — Check if managed resource has Ready=True
- `c:managed_resources(api_group, version, kind)` → `{items}` — List managed resources

### Utilities

- `c:all_providers_healthy()` → `{healthy, unhealthy, total, unhealthy_names}` — Check all providers health
- `c:all_xrds_established()` → `{established, not_established, total}` — Check all XRDs status

Example:
```lua
local crossplane = require("assay.crossplane")
local c = crossplane.client("https://k8s-api:6443", env.get("K8S_TOKEN"))
local status = c:all_providers_healthy()
assert.eq(status.unhealthy, 0, "Unhealthy providers: " .. table.concat(status.unhealthy_names, ", "))
```
