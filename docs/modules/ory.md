## assay.ory.kratos

Ory Kratos identity management. Self-service login, registration, recovery and settings flows,
identity CRUD via the admin API, session introspection (whoami), and identity schemas.
Client: `kratos.client({public_url="...", admin_url="..."})`.

- `c:whoami(cookie_or_token)` → session|nil — Introspect a session (nil on 401)
- `c:create_login_flow(opts?)` → flow — Initialize a login flow (browser or api)
- `c:create_registration_flow(opts?)` → flow — Initialize a registration flow
- `c:create_recovery_flow(opts?)` → flow — Initialize a recovery flow
- `c:create_settings_flow(opts?)` → flow — Initialize a settings flow
- `c:submit_login(flow_id, payload)` → `{session, session_token?}` — Submit login flow
- `c:submit_registration(flow_id, payload)` → `{identity, session?}` — Submit registration flow
- `c:list_identities(opts?)` → [identity] — Admin: list identities
- `c:get_identity(id)` → identity|nil — Admin: get identity by ID
- `c:create_identity(payload)` → identity — Admin: create identity
- `c:update_identity(id, payload)` → identity — Admin: update identity
- `c:delete_identity(id)` → bool — Admin: delete identity
- `c:list_schemas()` → [schema] — List identity schemas
- `c:get_schema(id)` → schema|nil — Get schema by ID

Example:
```lua
local kratos = require("assay.ory.kratos")
local c = kratos.client({
  public_url = "http://kratos-public:4433",
  admin_url = "http://kratos-admin:4434",
})
local session = c:whoami(cookie)
log.info("Logged in as: " .. session.identity.traits.email)
```

## assay.ory.hydra

Ory Hydra OAuth2 and OpenID Connect server. OAuth2 client CRUD via the admin API,
authorize URL builder, token exchange, accept/reject login and consent challenges,
introspection, JWK endpoint, and OIDC discovery.
Client: `hydra.client({public_url="...", admin_url="..."})`.

- `c:discovery()` → `{issuer, authorization_endpoint, token_endpoint, jwks_uri, ...}` — OIDC discovery
- `c:jwks()` → `{keys}` — JSON Web Key Set
- `c:list_clients(opts?)` → [client] — Admin: list OAuth2 clients
- `c:get_client(id)` → client|nil — Admin: get client by ID
- `c:create_client(payload)` → client — Admin: create OAuth2 client
- `c:update_client(id, payload)` → client — Admin: update OAuth2 client
- `c:delete_client(id)` → bool — Admin: delete OAuth2 client
- `c:authorize_url(opts)` → string — Build an authorization URL from `{client_id, redirect_uri, scope, state, ...}`
- `c:exchange_token(opts)` → `{access_token, id_token?, refresh_token?, expires_in}` — Exchange code for tokens
- `c:introspect(token, scope?)` → `{active, sub, scope, ...}` — Admin introspection
- `c:accept_login(challenge, payload)` → `{redirect_to}` — Accept a login challenge
- `c:reject_login(challenge, payload)` → `{redirect_to}` — Reject a login challenge
- `c:accept_consent(challenge, payload)` → `{redirect_to}` — Accept a consent challenge
- `c:reject_consent(challenge, payload)` → `{redirect_to}` — Reject a consent challenge

Example:
```lua
local hydra = require("assay.ory.hydra")
local c = hydra.client({
  public_url = "https://hydra.example.com",
  admin_url = "http://hydra-admin:4445",
})
local client = c:create_client({
  client_name = "my-app",
  grant_types = { "authorization_code", "refresh_token" },
  redirect_uris = { "https://app.example.com/callback" },
})
```

## assay.ory.keto

Ory Keto relationship-based access control (Zanzibar-style ReBAC). Relation-tuple CRUD,
permission checks, role/group membership queries, and the expand API.
Client: `keto.client({read_url="...", write_url="..."})`.

- `c:check(namespace, object, relation, subject)` → bool — Check if a relation tuple allows access
- `c:create_tuple(tuple)` → bool — Create a relation tuple `{namespace, object, relation, subject_id|subject_set}`
- `c:delete_tuple(tuple)` → bool — Delete a relation tuple
- `c:list_tuples(query)` → [tuple] — List tuples matching query filters
- `c:expand(namespace, object, relation, depth?)` → tree — Expand a subject tree (Zanzibar expand)
- `c:list_relations(namespace, object)` → [relation] — List relations for an object

Example:
```lua
local keto = require("assay.ory.keto")
local c = keto.client({
  read_url = "http://keto-read:4466",
  write_url = "http://keto-write:4467",
})
c:create_tuple({
  namespace = "apps", object = "cc", relation = "admin",
  subject_id = "user:alice",
})
assert(c:check("apps", "cc", "admin", "user:alice"))
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

- `p:user_roles(user_id)` → `{role}` — held roles, sorted by rank descending
- `p:user_primary_role(user_id)` → role — highest-ranked, for compact UI badges
- `p:user_capabilities(user_id)` → `{cap=true,...}` — union over all held roles, falls back to `default_role` caps when empty
- `p:user_has_capability(user_id, cap)` → bool — single capability check
- `p:add(user_id, role)` — idempotent membership add (no-op if already a member)
- `p:remove(user_id, role)` — membership remove (swallows 404)
- `p:list_members(role)` → `{user_id}` — direct members of a role
- `p:list_all_memberships()` → `{[role]={user_id,...}}` — full snapshot
- `p:reset_role(role)` — delete all members of a role (for bootstrap/seed scripts)
- `p:require_capability(cap, handler)` → handler — `http.serve` middleware that 403s callers without `cap`

Example:
```lua
local keto = require("assay.ory.keto")
local rbac = require("assay.ory.rbac")

local kc = keto.client({ read_url = "http://keto-read:4466", write_url = "http://keto-write:4467" })
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

policy:add("user:alice", "approver")
assert(policy:user_has_capability("user:alice", "approve"))
assert(not policy:user_has_capability("user:alice", "trigger"))
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
local allowed = o.keto:check("apps", "cc", "admin", "user:alice")
```
