## assay.dex

Dex OIDC identity provider. Discovery, JWKS, health, and configuration validation.
Module-level functions (no client needed): `M.function(url, ...)`.

- `M.discovery(url)` → `{issuer, authorization_endpoint, token_endpoint, jwks_uri, ...}` — Get OIDC discovery configuration
- `M.jwks(url)` → `{keys}` — Get JSON Web Key Set (fetches jwks_uri from discovery)
- `M.issuer(url)` → string — Get issuer URL from discovery
- `M.health(url)` → bool — Check Dex health via `/healthz`
- `M.ready(url)` → bool — Check Dex readiness (alias for health)
- `M.has_endpoint(url, endpoint_name)` → bool — Check if endpoint exists in discovery doc
- `M.supported_scopes(url)` → [string] — List supported OIDC scopes
- `M.supported_response_types(url)` → [string] — List supported response types
- `M.supported_grant_types(url)` → [string] — List supported grant types
- `M.supports_scope(url, scope)` → bool — Check if a specific scope is supported
- `M.supports_grant_type(url, grant_type)` → bool — Check if a specific grant type is supported
- `M.validate_config(url)` → `{ok, errors}` — Validate OIDC configuration completeness (checks issuer, endpoints, jwks_uri)
- `M.admin_version(url)` → version|nil — Get Dex admin API version (nil if unavailable)

Example:
```lua
local dex = require("assay.dex")
assert.eq(dex.health("http://dex:5556"), true, "Dex not healthy")
local validation = dex.validate_config("http://dex:5556")
assert.eq(validation.ok, true, "OIDC config invalid: " .. table.concat(validation.errors, ", "))
```
