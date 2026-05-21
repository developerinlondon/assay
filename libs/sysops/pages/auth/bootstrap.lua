--! sysops.pages.auth.bootstrap - first-admin bootstrap.
--!
--! After a successful OIDC login, if zero admin tuples exist in
--! Zanzibar, automatically grant the logged-in user admin.
--!
--! This is "first-user-wins" — common in self-hosted software (Gitea,
--! Vault, Authentik all do this). Subsequent users must be granted by
--! an existing admin via the dashboard or sysops zanzibar UI.
--!
--! The grant fires from callback.lua right after id_token verification:
--!
--!   bootstrap.maybe_grant_first_admin(claims)
--!
--! It's a no-op when:
--!   - ctx.engine isn't wired (the consumer didn't provide an engine
--!     HTTP client, so we can't talk to Zanzibar at all)
--!   - ctx.authz_bootstrap_first_admin is false (operator opted out)
--!   - at least one user already has the engine:core#admin relation,
--!     either directly or through a non-empty userset.

local ctx  = require("sysops.ctx")
local auth = require("sysops.auth")

local M = {}

-- The full set of canonical tuples the first user gets. Convention
-- picked up from libs/sysops/pages/zanzibar/bootstrap.lua's seed list
-- (the non-OIDC bootstrap page that PR #150 added). All four resources
-- — auth, engine, workflow, vault — granted at once so the first OIDC
-- user has end-to-end access on a fresh install. Subsequent users get
-- nothing until an admin grants them specific tuples via the
-- /zanzibar UI.
local FIRST_ADMIN_TUPLES = {
  { object_type = "host",     object_id = "local",  relation = "admin"  },
  { object_type = "auth",     object_id = "system", relation = "admin"  },
  { object_type = "engine",   object_id = "core",   relation = "admin"  },
  { object_type = "workflow", object_id = "main",   relation = "access" },
  { object_type = "vault",    object_id = "main",   relation = "access" },
}

-- The "any admin already exists?" probe checks the engine:core#admin
-- tuple — that's the canonical "is this a fresh install" marker.
local ADMIN_OBJECT_TYPE = "engine"
local ADMIN_OBJECT_ID   = "core"
local ADMIN_RELATION    = "admin"

local function admin_object_str()
  return ADMIN_OBJECT_TYPE .. ":" .. ADMIN_OBJECT_ID
end

local function tuple_items(body)
  local tuples = body
  if type(body) == "string" then
    local ok, decoded = pcall(json.parse, body)
    if not ok then return nil end
    tuples = decoded
  end
  if type(tuples) ~= "table" then return nil end
  if type(tuples.items) == "table" then return tuples.items end
  return tuples
end

local function direct_user_tuple(t)
  if type(t) ~= "table" then return false end
  if t.subject_type == "user" and (t.subject_rel == nil or t.subject_rel == "") then
    return true
  end
  if type(t.subject) == "string" then
    return t.subject:match("^user:[^#]+$") ~= nil
  end
  return false
end

local function userset_filter(t)
  if type(t) ~= "table" then return nil end
  if type(t.subject_type) == "string"
      and type(t.subject_id) == "string"
      and type(t.subject_rel) == "string"
      and t.subject_rel ~= "" then
    return {
      object_type = t.subject_type,
      object_id   = t.subject_id,
      relation    = t.subject_rel,
      subject_type = "user",
    }
  end
  if type(t.subject) == "string" then
    local object_type, object_id, relation = t.subject:match("^([^:]+):([^#]+)#(.+)$")
    if object_type and object_id and relation then
      return {
        object_type = object_type,
        object_id   = object_id,
        relation    = relation,
        subject_type = "user",
      }
    end
  end
  return nil
end

local function userset_has_direct_user_member(zanzibar, t)
  local filter = userset_filter(t)
  if not filter then return false end
  local body, err = zanzibar.tuples(filter)
  if err then return true end
  local items = tuple_items(body)
  if not items then return true end
  for _, item in ipairs(items) do
    if direct_user_tuple(item) then return true end
  end
  return false
end

--- Returns true if at least one user already has engine:core#admin.
local function admins_exist(zanzibar)
  local body, err = zanzibar.tuples({
    object_type = ADMIN_OBJECT_TYPE,
    object_id   = ADMIN_OBJECT_ID,
    relation    = ADMIN_RELATION,
  })
  if err then
    -- If the engine doesn't expose tuples listing yet (some backends
    -- return 404/405 — the SDK's docstring notes this), fail closed:
    -- treat as "admins exist" so we don't accidentally grant.
    return true
  end
  local items = tuple_items(body)
  if not items then return true end
  for _, t in ipairs(items) do
    if direct_user_tuple(t) or userset_has_direct_user_member(zanzibar, t) then
      return true
    end
  end
  return false
end

-- Namespaces the gateway depends on. Some are auto-seeded by the
-- engine modules (auth, engine, workflow, vault) but `host` is
-- sysops-defined for the host-ops surface (audit/machines/services/…).
-- ensure_namespaces() runs once before granting tuples so the engine's
-- check() resolves them. Idempotent: existing definitions are no-ops.
local REQUIRED_NAMESPACES = {
  {
    name = "host",
    definitions = {
      admin = {
        name = "admin",
        kind = { kind = "direct", value = {
          { object_type = "user", relation = nil, wildcard = false },
        } },
      },
    },
  },
}

local function ensure_namespaces(zanzibar)
  for _, schema in ipairs(REQUIRED_NAMESPACES) do
    -- define_namespace returns existing-namespace 4xx; treat as no-op.
    zanzibar.define_namespace(schema)
  end
end

--- Grant claims.sub the full set of first-admin tuples (host, auth,
--- engine, workflow, vault). Best-effort: writes all five; returns
--- the first error if any. Idempotent — Zanzibar write is upsert-style.
---
--- The engine wants split (subject_type, subject_id, relation,
--- object_type, object_id) — the same shape libs/sysops/pages/zanzibar/
--- bootstrap.lua uses successfully. NOT the combined "user:sub" form.
local function grant_admin(zanzibar, sub)
  ensure_namespaces(zanzibar)
  local first_err
  for _, t in ipairs(FIRST_ADMIN_TUPLES) do
    local _, err = zanzibar.write_tuple({
      subject_type = "user",
      subject_id   = sub,
      subject_rel  = "",
      relation     = t.relation,
      object_type  = t.object_type,
      object_id    = t.object_id,
    })
    if err and not first_err then first_err = err end
  end
  if first_err then return nil, first_err end
  return true
end

--- The public entry point — called from callback.lua. Returns nil
--- normally; returns "granted" if this call actually wrote the tuple
--- so the caller can log it.
function M.maybe_grant_first_admin(claims)
  if ctx.authz_bootstrap_first_admin == false then return nil end
  if not ctx.engine then return nil end
  if type(claims) ~= "table" or type(claims.sub) ~= "string" then return nil end

  local zanzibar = auth.new(ctx.engine).zanzibar
  if admins_exist(zanzibar) then return nil end

  local ok, err = grant_admin(zanzibar, claims.sub)
  if not ok then return nil, err end

  if ctx.audit and ctx.audit.log then
    ctx.audit.log("auth.bootstrap_first_admin", { sub = claims.sub })
  end
  return "granted"
end

-- Exposed for tests + integration tooling.
M._admin_object_str = admin_object_str
M._admins_exist     = admins_exist
M._grant_admin      = grant_admin

return M
