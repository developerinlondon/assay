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
--!   - at least one tuple already exists in (object=engine:core,
--!     relation=admin) — admins are already configured.

local ctx  = require("sysops.ctx")
local auth = require("sysops.auth")

local M = {}

-- The canonical tuple this module writes / inspects. Convention picked
-- up from libs/sysops/pages/zanzibar/bootstrap.lua's seed list (the
-- non-OIDC bootstrap page that PR #150 added). Keeping the two in
-- sync means both bootstrap paths agree on what "admin" means.
local ADMIN_OBJECT_TYPE = "engine"
local ADMIN_OBJECT_ID   = "core"
local ADMIN_RELATION    = "admin"

local function admin_object_str()
  return ADMIN_OBJECT_TYPE .. ":" .. ADMIN_OBJECT_ID
end

--- Returns true if at least one (engine:core#admin) tuple exists.
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
  -- The auth SDK returns resp.body verbatim. Real consumer-app engine
  -- wrappers parse JSON to a table; bare stubs may pass a string. Cope
  -- with both, matching the vault SDK's defensive pattern.
  local tuples = body
  if type(body) == "string" then
    local ok, decoded = pcall(json.parse, body)
    if not ok then return true end
    tuples = decoded
  end
  if type(tuples) == "table" then
    if tuples.items and type(tuples.items) == "table" then
      return #tuples.items > 0
    end
    return #tuples > 0
  end
  return true
end

--- Grant claims.sub the admin tuple, idempotently. Returns (ok, err).
local function grant_admin(zanzibar, sub)
  return zanzibar.write_tuple({
    subject     = "user:" .. sub,
    relation    = ADMIN_RELATION,
    object_type = ADMIN_OBJECT_TYPE,
    object_id   = ADMIN_OBJECT_ID,
  })
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
