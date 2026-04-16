## assay.zitadel

Zitadel OIDC identity management. Projects, OIDC apps, IdPs, users, login policies. Client:
`zitadel.client({url="...", domain="...", machine_key=...})` or `{..., machine_key_file="..."}` or
`{..., token="..."}`. Authenticates via JWT machine key exchange.

### Domains

- `c.domains:ensure_primary(domain)` -> bool -- Set organization primary domain

### Projects

- `c.projects:find(name)` -> project|nil -- Find project by exact name
- `c.projects:create(name, opts?)` -> project -- Create project. `opts`: `{projectRoleAssertion}`
- `c.projects:ensure(name, opts?)` -> project -- Create project if not exists, return existing if
  found

### OIDC Applications

- `c.apps:find(project_id, name)` -> app|nil -- Find OIDC app by name within project
- `c.apps:create_oidc(project_id, opts)` -> app -- Create OIDC app. `opts`:
  `{name, subdomain, callbackPath, redirectUris, grantTypes, ...}`
- `c.apps:ensure_oidc(project_id, opts)` -> app -- Create OIDC app if not exists

### Identity Providers

- `c.idps:find(name)` -> idp|nil -- Find identity provider by name
- `c.idps:ensure_google(opts)` -> idp_id|nil -- Ensure Google IdP. `opts`:
  `{clientId, clientSecret, scopes, providerOptions}`
- `c.idps:ensure_oidc(opts)` -> idp_id|nil -- Ensure generic OIDC IdP. `opts`:
  `{name, clientId, clientSecret, issuer, scopes, ...}`
- `c.idps:add_to_login_policy(idp_id)` -> bool -- Add IdP to organization login policy

### Users

- `c.users:search(query)` -> [user] -- Search users by query table
- `c.users:update_email(user_id, email)` -> bool -- Update user email (auto-verified)

### Login Policy

- `c.login_policy:get()` -> policy|nil -- Get current login policy
- `c.login_policy:update(policy)` -> bool -- Update login policy
- `c.login_policy:disable_password()` -> bool -- Disable password-based login, enable external IdP

Example:

```lua
local zitadel = require("assay.zitadel")
local c = zitadel.client({
  url = "https://zitadel.example.com",
  domain = "example.com",
  machine_key_file = "/secrets/zitadel-key.json",
})
local proj = c.projects:ensure("my-platform")
local app = c.apps:ensure_oidc(proj.id, {
  name = "grafana", subdomain = "grafana", callbackPath = "/login/generic_oauth",
})
```
