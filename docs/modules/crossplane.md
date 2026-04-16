## assay.crossplane

Crossplane infrastructure management. Providers, XRDs, compositions, managed resources. Client:
`crossplane.client(url, token)`.

### Providers

- `c.providers:list()` -> `{items}` -- List all providers
- `c.providers:get(name)` -> provider|nil -- Get provider by name
- `c.providers:is_healthy(name)` -> bool -- Check if provider has Healthy=True condition
- `c.providers:is_installed(name)` -> bool -- Check if provider has Installed=True condition
- `c.providers:status(name)` -> `{installed, healthy, current_revision, conditions}` -- Get full
  provider status
- `c.providers:all_healthy()` -> `{healthy, unhealthy, total, unhealthy_names}` -- Check all
  providers health

### Provider Revisions

- `c.provider_revisions:list()` -> `{items}` -- List provider revisions
- `c.provider_revisions:get(name)` -> revision|nil -- Get provider revision by name

### Configurations

- `c.configurations:list()` -> `{items}` -- List configurations
- `c.configurations:get(name)` -> config|nil -- Get configuration by name
- `c.configurations:is_healthy(name)` -> bool -- Check if configuration is healthy
- `c.configurations:is_installed(name)` -> bool -- Check if configuration is installed

### Functions

- `c.functions:list()` -> `{items}` -- List composition functions
- `c.functions:get(name)` -> function|nil -- Get function by name
- `c.functions:is_healthy(name)` -> bool -- Check if function is healthy

### Composite Resource Definitions (XRDs)

- `c.xrds:list()` -> `{items}` -- List all XRDs
- `c.xrds:get(name)` -> xrd|nil -- Get XRD by name
- `c.xrds:is_established(name)` -> bool -- Check if XRD has Established=True condition
- `c.xrds:all_established()` -> `{established, not_established, total}` -- Check all XRDs status

### Compositions

- `c.compositions:list()` -> `{items}` -- List all compositions
- `c.compositions:get(name)` -> composition|nil -- Get composition by name

### Managed Resources

- `c.managed_resources:get(api_group, version, kind, name)` -> resource|nil -- Get managed resource
- `c.managed_resources:is_ready(api_group, version, kind, name)` -> bool -- Check if managed
  resource has Ready=True
- `c.managed_resources:list(api_group, version, kind)` -> `{items}` -- List managed resources

Example:

```lua
local crossplane = require("assay.crossplane")
local c = crossplane.client("https://k8s-api:6443", env.get("K8S_TOKEN"))
local status = c.providers:all_healthy()
assert.eq(status.unhealthy, 0, "Unhealthy providers: " .. table.concat(status.unhealthy_names, ", "))
```
