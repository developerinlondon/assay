## assay.zitadel

Zitadel OIDC identity management. Projects, OIDC apps, IdPs, users, login policies.
Client: `zitadel.client({url="...", domain="...", machine_key=...})` or `{..., machine_key_file="..."}` or `{..., token="..."}`.
Authenticates via JWT machine key exchange.

### Domain & Organization

- `c:ensure_primary_domain(domain)` → bool — Set organization primary domain

### Projects

- `c:find_project(name)` → project|nil — Find project by exact name
- `c:create_project(name, opts?)` → project — Create project. `opts`: `{projectRoleAssertion}`
- `c:ensure_project(name, opts?)` → project — Create project if not exists, return existing if found

### OIDC Applications

- `c:find_app(project_id, name)` → app|nil — Find OIDC app by name within project
- `c:create_oidc_app(project_id, opts)` → app — Create OIDC app. `opts`: `{name, subdomain, callbackPath, redirectUris, grantTypes, ...}`
- `c:ensure_oidc_app(project_id, opts)` → app — Create OIDC app if not exists

### Identity Providers

- `c:find_idp(name)` → idp|nil — Find identity provider by name
- `c:ensure_google_idp(opts)` → idp_id|nil — Ensure Google IdP. `opts`: `{clientId, clientSecret, scopes, providerOptions}`
- `c:ensure_oidc_idp(opts)` → idp_id|nil — Ensure generic OIDC IdP. `opts`: `{name, clientId, clientSecret, issuer, scopes, ...}`
- `c:add_idp_to_login_policy(idp_id)` → bool — Add IdP to organization login policy

### User Management

- `c:search_users(query)` → [user] — Search users by query table
- `c:update_user_email(user_id, email)` → bool — Update user email (auto-verified)

### Login Policy

- `c:get_login_policy()` → policy|nil — Get current login policy
- `c:update_login_policy(policy)` → bool — Update login policy
- `c:disable_password_login()` → bool — Disable password-based login, enable external IdP

Example:
```lua
local zitadel = require("assay.zitadel")
local c = zitadel.client({
  url = "https://zitadel.example.com",
  domain = "example.com",
  machine_key_file = "/secrets/zitadel-key.json",
})
local proj = c:ensure_project("my-platform")
local app = c:ensure_oidc_app(proj.id, {
  name = "grafana", subdomain = "grafana", callbackPath = "/login/generic_oauth",
})
```
