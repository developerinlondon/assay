--- @module assay.ory.rbac
--- @description Capability-based RBAC engine layered on top of Ory Keto. Define a policy once (role -> capabilities map) and get user lookups, capability checks, and membership management helpers. A user can hold multiple roles; their effective capability set is the union of all assigned roles. Each role also has a rank so a single "primary" role can be shown for display.
--- @keywords rbac, roles, permissions, capabilities, authz, authorization, keto, ory, zanzibar, policy
--- @quickref rbac.policy(opts) -> policy | Build a policy from a role map and a Keto client. opts: { namespace, keto, roles, default_role? }
--- @quickref p.users:roles(user_id) -> [role_name] | All roles a user holds, sorted by rank (highest first)
--- @quickref p.users:primary_role(user_id) -> role_name | Highest-ranked role (for UI badges)
--- @quickref p.users:capabilities(user_id) -> {cap=true, ...} | Union of capabilities from every role the user holds
--- @quickref p.users:has_capability(user_id, cap) -> bool | Check whether the user holds a given capability
--- @quickref p.members:add(user_id, role_name) -> nil | Add the user to a role (idempotent)
--- @quickref p.members:remove(user_id, role_name) -> nil | Remove the user from a role (idempotent)
--- @quickref p.members:list(role_name) -> [user_id] | List every user assigned to a specific role
--- @quickref p.members:list_all() -> {role_name=[user_id]} | Full snapshot of every role and its members
--- @quickref p.members:reset(role_name) -> nil | Delete every member of a role (for bootstrap/seed scripts)
--- @quickref p.policy:roles() -> [role_name] | All configured role names, highest rank first
--- @quickref p.policy:get(role_name) -> {rank, capabilities} | Role metadata from the policy definition
--- @quickref p.middleware:require_capability(cap, handler) -> handler | http.serve wrapper returning 403 when the caller lacks the capability

local M = {}

local KETO_NAMESPACE = "Role"

local function strip_user_prefix(subject)
  if type(subject) ~= "string" then return subject end
  local stripped = subject:match("^user:(.+)$")
  return stripped or subject
end

local function ensure_user_subject(user_id)
  if type(user_id) ~= "string" or user_id == "" then
    error("assay.ory.rbac: user_id must be a non-empty string")
  end
  if user_id:match("^user:") then
    return user_id
  end
  return "user:" .. user_id
end

-- Build a policy from a role map and a Keto client.
--
-- opts: {
--   namespace    = "command-center",    -- object prefix; tuples look like
--                                       --   Role:command-center:<role>@user:<id>
--   keto         = keto.client(...),    -- assay.ory.keto client with write_url set
--   roles        = {                    -- role map keyed by role name
--     owner    = { rank = 5, capabilities = {"read","trigger","approve"} },
--     admin    = { rank = 4, capabilities = {"read","trigger","approve"} },
--     operator = { rank = 2, capabilities = {"read","trigger"} },
--     viewer   = { rank = 1, capabilities = {"read"} },
--   },
--   default_role = "viewer",            -- optional; role returned when a
--                                       -- user has no explicit memberships
-- }
function M.policy(opts)
  opts = opts or {}
  if type(opts.namespace) ~= "string" or opts.namespace == "" then
    error("assay.ory.rbac.policy: namespace is required")
  end
  if type(opts.keto) ~= "table" then
    error("assay.ory.rbac.policy: keto client is required")
  end
  if type(opts.roles) ~= "table" or next(opts.roles) == nil then
    error("assay.ory.rbac.policy: roles map is required and must not be empty")
  end

  -- Capture into upvalues for closures
  local keto = opts.keto
  local ns = opts.namespace
  local default_role = opts.default_role

  -- Normalise each role to { rank, capability_set } where capability_set is
  -- a {cap=true} lookup for O(1) checks.
  local roles = {}
  for name, def in pairs(opts.roles) do
    local rank = tonumber(def.rank or 0) or 0
    local caps = {}
    for _, c in ipairs(def.capabilities or {}) do
      caps[c] = true
    end
    roles[name] = { rank = rank, capabilities = caps }
  end

  -- Precompute a list of role names sorted by rank (highest first) so
  -- users:roles / users:primary_role / policy:roles can return stable ordering.
  local ranked_role_names = {}
  for name, _ in pairs(roles) do
    ranked_role_names[#ranked_role_names + 1] = name
  end
  table.sort(ranked_role_names, function(a, b)
    if roles[a].rank == roles[b].rank then
      return a < b
    end
    return roles[a].rank > roles[b].rank
  end)

  local function object_for(role_name)
    return ns .. ":" .. role_name
  end

  -- ========== Sub-objects ==========

  local p = {}

  -- ========== p.policy ==========

  p.policy = {}

  function p.policy:roles()
    local out = {}
    for i, name in ipairs(ranked_role_names) do
      out[i] = name
    end
    return out
  end

  function p.policy:get(role_name)
    local def = roles[role_name]
    if not def then return nil end
    local caps = {}
    for c, _ in pairs(def.capabilities) do
      caps[#caps + 1] = c
    end
    table.sort(caps)
    return { rank = def.rank, capabilities = caps }
  end

  -- ========== p.users ==========

  p.users = {}

  -- Get all role names the user holds, sorted by rank (highest first).
  -- Only roles defined in the policy are returned; tuples that reference
  -- unknown role names are silently ignored so an out-of-date Keto row
  -- can't grant an undefined capability.
  function p.users:roles(user_id)
    local subject = ensure_user_subject(user_id)
    local tuples = keto.roles:user_roles(subject, KETO_NAMESPACE)
    local seen = {}
    local held = {}
    for _, t in ipairs(tuples) do
      local role_name = t.object:match("^" .. ns:gsub("%-", "%%-") .. ":(.+)$")
      if role_name and roles[role_name] and not seen[role_name] then
        seen[role_name] = true
        held[#held + 1] = role_name
      end
    end
    table.sort(held, function(a, b)
      if roles[a].rank == roles[b].rank then
        return a < b
      end
      return roles[a].rank > roles[b].rank
    end)
    return held
  end

  -- Highest-ranked role the user holds, or the configured default_role
  -- (or nil) if the user has none. Used for compact UI badges where
  -- only one label fits.
  function p.users:primary_role(user_id)
    local held = p.users:roles(user_id)
    if #held > 0 then return held[1] end
    return default_role
  end

  -- Union of capabilities from every role the user holds, returned as
  -- a set ({cap=true, ...}) for O(1) checks by the caller.
  function p.users:capabilities(user_id)
    local held = p.users:roles(user_id)
    local set = {}
    if #held == 0 and default_role then
      local def = roles[default_role]
      if def then
        for c, _ in pairs(def.capabilities) do set[c] = true end
      end
      return set
    end
    for _, role_name in ipairs(held) do
      for c, _ in pairs(roles[role_name].capabilities) do
        set[c] = true
      end
    end
    return set
  end

  function p.users:has_capability(user_id, cap)
    return p.users:capabilities(user_id)[cap] == true
  end

  -- ========== p.members ==========

  p.members = {}

  -- Add a user to a role. Idempotent: if the user is already a member,
  -- this is a no-op. Requires the Keto client to be configured with a
  -- write_url.
  function p.members:add(user_id, role_name)
    if not roles[role_name] then
      error("assay.ory.rbac: unknown role " .. tostring(role_name))
    end
    local members_list = p.members:list(role_name)
    local target = strip_user_prefix(ensure_user_subject(user_id))
    for _, existing in ipairs(members_list) do
      if existing == target then return end
    end
    keto.tuples:create({
      namespace = KETO_NAMESPACE,
      object = object_for(role_name),
      relation = "members",
      subject_id = ensure_user_subject(user_id),
    })
  end

  -- Remove a user from a role. Idempotent: if the user isn't a member,
  -- this is a no-op.
  function p.members:remove(user_id, role_name)
    if not roles[role_name] then
      error("assay.ory.rbac: unknown role " .. tostring(role_name))
    end
    local ok, err = pcall(function()
      keto.tuples:delete({
        namespace = KETO_NAMESPACE,
        object = object_for(role_name),
        relation = "members",
        subject_id = ensure_user_subject(user_id),
      })
    end)
    if not ok and not tostring(err):match("404") then
      error(err)
    end
  end

  -- List every user (without the "user:" prefix) assigned to a role.
  function p.members:list(role_name)
    if not roles[role_name] then
      error("assay.ory.rbac: unknown role " .. tostring(role_name))
    end
    local result = keto.tuples:list({
      namespace = KETO_NAMESPACE,
      object = object_for(role_name),
      relation = "members",
    })
    local out = {}
    local seen = {}
    for _, t in ipairs(result.relation_tuples or {}) do
      local uid = strip_user_prefix(t.subject_id)
      if uid and not seen[uid] then
        seen[uid] = true
        out[#out + 1] = uid
      end
    end
    return out
  end

  -- Snapshot of every role and its members. Handy for admin UIs.
  function p.members:list_all()
    local out = {}
    for _, name in ipairs(ranked_role_names) do
      out[name] = p.members:list(name)
    end
    return out
  end

  -- Delete every member of a role. Used by bootstrap/seed scripts that
  -- want to reset the policy to a known state. Keto's PUT is not
  -- idempotent at the tuple level so re-running a seed without a reset
  -- creates duplicates.
  function p.members:reset(role_name)
    if not roles[role_name] then
      error("assay.ory.rbac: unknown role " .. tostring(role_name))
    end
    keto.tuples:delete_all({
      namespace = KETO_NAMESPACE,
      object = object_for(role_name),
      relation = "members",
    })
  end

  -- ========== p.middleware ==========

  p.middleware = {}

  -- Wrap an http.serve handler so the request is rejected (with the
  -- configured HTTP status, default 403) if the authenticated user
  -- doesn't hold the required capability. The caller is responsible
  -- for setting `req.user_id` on the request table before this runs
  -- (e.g. via an earlier auth middleware).
  function p.middleware:require_capability(cap, handler)
    return function(req)
      local user_id = req.user_id
      if not user_id or user_id == "" then
        return { status = 401, json = { error = "Authentication required" } }
      end
      if not p.users:has_capability(user_id, cap) then
        return { status = 403, json = { error = cap .. " capability required" } }
      end
      return handler(req)
    end
  end

  return p
end

return M
