## assay.ory.kratos

Ory Kratos identity management. Self-service login, registration, recovery and settings flows,
identity CRUD via the admin API, session introspection (whoami), and identity schemas.
Client: `kratos.client({public_url="...", admin_url="..."})`.

**Sessions** (`c.sessions`):

- `c.sessions:whoami(cookie_or_token)` → session|nil — Introspect a session (nil on 401)
- `c.sessions:list(identity_id)` → [session] — Admin: list sessions for an identity
- `c.sessions:revoke(identity_id)` → nil — Admin: revoke all sessions for an identity

**Flows** (`c.flows`):

- `c.flows:create_login(opts?)` → flow — Initialize a login flow (browser or api)
- `c.flows:get_login(flow_id, cookie?)` → flow — Fetch an existing login flow
- `c.flows:submit_login(flow_id, payload, cookie?)` → `{session, session_token?}` — Submit login flow
- `c.flows:create_registration(opts?)` → flow — Initialize a registration flow
- `c.flows:get_registration(flow_id, cookie?)` → flow — Fetch a registration flow
- `c.flows:submit_registration(flow_id, payload, cookie?)` → `{identity, session?}` — Submit registration flow
- `c.flows:create_recovery(opts?)` → flow — Initialize a recovery flow
- `c.flows:get_recovery(flow_id, cookie?)` → flow — Fetch a recovery flow
- `c.flows:submit_recovery(flow_id, payload, cookie?)` → flow — Submit recovery flow
- `c.flows:create_settings(cookie)` → flow — Initialize a settings flow
- `c.flows:get_settings(flow_id, cookie?)` → flow — Fetch a settings flow
- `c.flows:submit_settings(flow_id, payload, cookie?)` → flow — Submit settings flow

**Identities** (`c.identities`):

- `c.identities:list(opts?)` → [identity] — Admin: list identities
- `c.identities:get(id)` → identity|nil — Admin: get identity by ID
- `c.identities:create(payload)` → identity — Admin: create identity
- `c.identities:update(id, payload)` → identity — Admin: update identity
- `c.identities:delete(id)` → nil — Admin: delete identity

**Schemas** (`c.schemas`):

- `c.schemas:list()` → [schema] — List identity schemas
- `c.schemas:get(id)` → schema|nil — Get schema by ID

Example:
```lua
local kratos = require("assay.ory.kratos")
local c = kratos.client({
  public_url = "http://kratos-public:4433",
  admin_url = "http://kratos-admin:4434",
})
local session = c.sessions:whoami(cookie)
log.info("Logged in as: " .. session.identity.traits.email)
```

## assay.ory.hydra

Ory Hydra OAuth2 and OpenID Connect server. OAuth2 client CRUD via the admin API,
authorize URL builder, token exchange, accept/reject login and consent challenges,
introspection, JWK endpoint, and OIDC discovery.
Client: `hydra.client({public_url="...", admin_url="..."})`.

**Clients** (`c.clients`):

- `c.clients:list(opts?)` → [client] — Admin: list OAuth2 clients
- `c.clients:get(id)` → client|nil — Admin: get client by ID
- `c.clients:create(payload)` → client — Admin: create OAuth2 client
- `c.clients:update(id, payload)` → client — Admin: update OAuth2 client
- `c.clients:delete(id)` → nil — Admin: delete OAuth2 client

**OAuth2** (`c.oauth2`):

- `c.oauth2:authorize_url(client_id, opts)` → string — Build an authorization URL
- `c.oauth2:exchange_code(opts)` → `{access_token, id_token?, refresh_token?, expires_in}` — Exchange code for tokens
- `c.oauth2:refresh_token(client_id, client_secret, refresh_token)` → tokens — Refresh an access token
- `c.oauth2:introspect(token)` → `{active, sub, scope, ...}` — Admin introspection
- `c.oauth2:revoke_token(client_id, client_secret, token)` → nil — Revoke a token

**Login challenges** (`c.login`):

- `c.login:get(challenge)` → `{challenge, subject, client, ...}` — Fetch a pending login challenge
- `c.login:accept(challenge, subject, opts?)` → `{redirect_to}` — Accept a login challenge
- `c.login:reject(challenge, error?)` → `{redirect_to}` — Reject a login challenge

**Consent challenges** (`c.consent`):

- `c.consent:get(challenge)` → `{challenge, subject, requested_scope, ...}` — Fetch a pending consent challenge
- `c.consent:accept(challenge, opts)` → `{redirect_to}` — Accept a consent challenge
- `c.consent:reject(challenge, error?)` → `{redirect_to}` — Reject a consent challenge

**Logout challenges** (`c.logout`):

- `c.logout:get(challenge)` → `{request_url, rp_initiated, sid, subject, client}` — Fetch a pending logout challenge
- `c.logout:accept(challenge)` → `{redirect_to}` — Accept a logout challenge
- `c.logout:reject(challenge)` → nil — Reject a logout challenge

**Discovery** (`c.discovery`):

- `c.discovery:openid_config()` → `{issuer, authorization_endpoint, ...}` — OIDC discovery document
- `c.discovery:jwks()` → `{keys}` — JSON Web Key Set

Example:
```lua
local hydra = require("assay.ory.hydra")
local c = hydra.client({
  public_url = "https://hydra.example.com",
  admin_url = "http://hydra-admin:4445",
})
local client = c.clients:create({
  client_name = "my-app",
  grant_types = { "authorization_code", "refresh_token" },
  redirect_uris = { "https://app.example.com/callback" },
})
```

## assay.ory.keto

Ory Keto relationship-based access control (Zanzibar-style ReBAC). Relation-tuple CRUD,
permission checks, role/group membership queries, and the expand API.
Client: `keto.client(read_url, {write_url="..."})`.

**Tuples** (`c.tuples`):

- `c.tuples:list(query)` → `{relation_tuples, next_page_token}` — List tuples matching query filters
- `c.tuples:create(tuple)` → nil — Create a relation tuple `{namespace, object, relation, subject_id|subject_set}`
- `c.tuples:delete(tuple)` → nil — Delete a relation tuple
- `c.tuples:delete_all(filters)` → nil — Delete all matching relation tuples

**Permissions** (`c.permissions`):

- `c.permissions:check(namespace, object, relation, subject)` → bool — Check if a relation tuple allows access
- `c.permissions:check({namespace, object, relation, subject_id})` → bool — Check (table form)
- `c.permissions:batch_check(tuples)` → [bool] — Check multiple tuples in one call
- `c.permissions:expand(namespace, object, relation, depth?)` → tree — Expand a subject tree (Zanzibar expand)

**Roles** (`c.roles`):

- `c.roles:user_roles(user_id, namespace?)` → [{object, relation}] — Get all role memberships for a user
- `c.roles:has_any(user_id, role_objects, namespace?)` → bool — Check if a user has any of the given roles

Example:
```lua
local keto = require("assay.ory.keto")
local c = keto.client("http://keto-read:4466", {
  write_url = "http://keto-write:4467",
})
c.tuples:create({
  namespace = "apps", object = "cc", relation = "admin",
  subject_id = "user:alice",
})
assert(c.permissions:check("apps", "cc", "admin", "user:alice"))
```

## assay.ory.rbac

Capability-based RBAC engine layered on top of Ory Keto. Define a policy once
(role → capability set) and get user lookups, capability checks, and membership
management for free. Users can hold multiple roles and the effective capability
set is the union, so separation of duties is enforceable at the authorization
layer (an `approver` role can have `approve` without also getting `trigger`,
even if listed above an `operator` role with `trigger`).

Policy: `rbac.policy({namespace, keto, roles, default_role?})`. `namespace`
filters Keto tuples (e.g. `"command-center"`); `keto` is a Keto client;
`roles` maps role names to `{rank, capabilities, label?, description?}`;
`default_role` is the role assumed for users with no memberships.

**Users** (`p.users`):

- `p.users:roles(user_id)` → `{role}` — held roles, sorted by rank descending
- `p.users:primary_role(user_id)` → role — highest-ranked, for compact UI badges
- `p.users:capabilities(user_id)` → `{cap=true,...}` — union over all held roles, falls back to `default_role` caps when empty
- `p.users:has_capability(user_id, cap)` → bool — single capability check

**Members** (`p.members`):

- `p.members:add(user_id, role)` — idempotent membership add (no-op if already a member)
- `p.members:remove(user_id, role)` — membership remove (swallows 404)
- `p.members:list(role)` → `{user_id}` — direct members of a role
- `p.members:list_all()` → `{[role]={user_id,...}}` — full snapshot
- `p.members:reset(role)` — delete all members of a role (for bootstrap/seed scripts)

**Policy** (`p.policy`):

- `p.policy:roles()` → `[role_name]` — all configured role names, highest rank first
- `p.policy:get(role_name)` → `{rank, capabilities}` — role metadata from the policy definition

**Middleware** (`p.middleware`):

- `p.middleware:require_capability(cap, handler)` → handler — `http.serve` middleware that 403s callers without `cap`

Example:
```lua
local keto = require("assay.ory.keto")
local rbac = require("assay.ory.rbac")

local kc = keto.client("http://keto-read:4466", { write_url = "http://keto-write:4467" })
local policy = rbac.policy({
  namespace = "command-center",
  keto = kc,
  default_role = "viewer",
  roles = {
    owner    = { rank = 5, capabilities = { "manage_roles", "approve", "trigger", "view" } },
    admin    = { rank = 4, capabilities = { "manage_roles", "approve", "trigger", "view" } },
    approver = { rank = 3, capabilities = { "approve", "view" } },
    operator = { rank = 2, capabilities = { "trigger", "view" } },
    viewer   = { rank = 1, capabilities = { "view" } },
  },
})

policy.members:add("user:alice", "approver")
assert(policy.users:has_capability("user:alice", "approve"))
assert(not policy.users:has_capability("user:alice", "trigger"))
```

## assay.ory

Convenience wrapper re-exporting `assay.ory.kratos`, `assay.ory.hydra`,
`assay.ory.keto`, and `assay.ory.rbac`, with `ory.connect(opts)` to build
all three Ory clients in a single call.

- `M.kratos` — re-export of `assay.ory.kratos`
- `M.hydra` — re-export of `assay.ory.hydra`
- `M.keto` — re-export of `assay.ory.keto`
- `M.rbac` — re-export of `assay.ory.rbac`
- `M.connect(opts)` → `{kratos, hydra, keto}` — Build all three clients. `opts`: `{kratos_public, kratos_admin, hydra_public, hydra_admin, keto_read, keto_write}`

Example:
```lua
local ory = require("assay.ory")
local o = ory.connect({
  kratos_public = "http://kratos-public:4433",
  kratos_admin = "http://kratos-admin:4434",
  hydra_public = "https://hydra.example.com",
  hydra_admin = "http://hydra-admin:4445",
  keto_read = "http://keto-read:4466",
  keto_write = "http://keto-write:4467",
})
local allowed = o.keto.permissions:check("apps", "cc", "admin", "user:alice")
```
