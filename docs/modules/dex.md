## assay.dex

Dex OIDC identity provider. Discovery, JWKS, health, and configuration validation. Client:
`dex.client(url)`. Module-level functions also available for backward compatibility.

### Discovery (`c.discovery`)

- `c.discovery:config()` → `{issuer, authorization_endpoint, token_endpoint, jwks_uri, ...}` — Get
  OIDC discovery configuration
- `c.discovery:jwks()` → `{keys}` — Get JSON Web Key Set (fetches jwks_uri from discovery)
- `c.discovery:issuer()` → string — Get issuer URL from discovery
- `c.discovery:has_endpoint(endpoint_name)` → bool — Check if endpoint exists in discovery doc

### Health (`c.health`)

- `c.health:check()` → bool — Check Dex health via `/healthz`
- `c.health:ready()` → bool — Check Dex readiness (alias for check)

### Scopes (`c.scopes`)

- `c.scopes:list()` → [string] — List supported OIDC scopes
- `c.scopes:supports(scope)` → bool — Check if a specific scope is supported

### Grants (`c.grants`)

- `c.grants:list()` → [string] — List supported grant types
- `c.grants:supports(grant_type)` → bool — Check if a specific grant type is supported
- `c.grants:response_types()` → [string] — List supported response types

### Top-level

- `c:validate_config()` → `{ok, errors}` — Validate OIDC configuration completeness (checks issuer,
  endpoints, jwks_uri)
- `c:admin_version()` → version|nil — Get Dex admin API version (nil if unavailable)

### Backward Compatibility

All legacy module-level functions (`M.discovery(url)`, `M.health(url)`, `M.supported_scopes(url)`,
etc.) remain available and delegate to the client sub-objects.

Example:

```lua
local dex = require("assay.dex")

-- New client sub-object style
local c = dex.client("http://dex:5556")
assert.eq(c.health:check(), true, "Dex not healthy")
local validation = c:validate_config()
assert.eq(validation.ok, true, "OIDC config invalid: " .. table.concat(validation.errors, ", "))
local scopes = c.scopes:list()
assert.eq(c.scopes:supports("openid"), true)

-- Legacy module-level style still works
assert.eq(dex.health("http://dex:5556"), true, "Dex not healthy")
local validation = dex.validate_config("http://dex:5556")
```
