--- @module assay.ory.keto
--- @description Ory Keto authorization — relation-tuple CRUD, permission checks (including OPL permits), role membership queries. Implements relationship-based access control (ReBAC, Google Zanzibar-style).
--- @keywords keto, ory, authorization, authz, rbac, rebac, permissions, roles, relation-tuples, zanzibar, access-control, members, groups, opl, permits
--- @quickref keto.client(read_url, opts?) -> client | Create a Keto client (read endpoint)
--- @quickref c.tuples:list(opts) -> {relation_tuples, next_page_token} | List relation tuples matching filters
--- @quickref c.tuples:create(tuple) -> nil | Create a relation tuple (requires write_url)
--- @quickref c.tuples:delete(tuple) -> nil | Delete a relation tuple (requires write_url)
--- @quickref c.tuples:delete_all(filters) -> nil | Delete all matching relation tuples (requires write_url)
--- @quickref c.permissions:check(ns, obj, rel, subject) -> bool | Check if a subject has a relation (or OPL permit) on an object
--- @quickref c.permissions:check({namespace, object, relation, subject_id}) -> bool | Check (table form)
--- @quickref c.permissions:batch_check(tuples) -> [bool] | Check multiple tuples in one call
--- @quickref c.permissions:expand(namespace, object, relation, depth?) -> tree | Expand a relation to see all members
--- @quickref c.roles:user_roles(user_id, namespace?) -> [{object, relation}] | Get all role memberships for a user
--- @quickref c.roles:has_any(user_id, role_objects, namespace?) -> bool | Check if a user has any of the given roles

local M = {}

-- Create a Keto client.
-- Pass a single URL for read-only, or opts.write_url for write operations.
-- Example:
--   local k = keto.client("http://keto-read:4466")
--   local k = keto.client("http://keto-read:4466", { write_url = "http://keto-write:4467" })
function M.client(read_url, opts)
  opts = opts or {}
  local read = read_url:gsub("/+$", "")
  local write = opts.write_url and opts.write_url:gsub("/+$", "") or nil

  local function urlencode(s)
    return (tostring(s):gsub("([^%w%-%.%_%~])", function(ch)
      return string.format("%%%02X", string.byte(ch))
    end))
  end

  local function build_query(params)
    local parts = {}
    for k, v in pairs(params) do
      if v ~= nil and v ~= "" then
        parts[#parts + 1] = urlencode(k) .. "=" .. urlencode(v)
      end
    end
    if #parts == 0 then return "" end
    return "?" .. table.concat(parts, "&")
  end

  local function read_get(path_str)
    local resp = http.get(read .. path_str)
    if resp.status ~= 200 then
      error("keto: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function require_write()
    if not write then
      error("keto: write_url not configured — pass opts.write_url to keto.client()")
    end
  end

  -- Build check query params from either positional or table-style args.
  local function build_check_params(namespace_or_table, object, relation, subject)
    local params
    if type(namespace_or_table) == "table" then
      local t = namespace_or_table
      params = {
        namespace = t.namespace,
        object = t.object,
        relation = t.relation,
      }
      local subj = t.subject_id or t.subject
      if type(subj) == "string" then
        params.subject_id = subj
      elseif type(subj) == "table" then
        params.subject_set_namespace = subj.namespace
        params.subject_set_object = subj.object
        params.subject_set_relation = subj.relation
      end
    else
      params = {
        namespace = namespace_or_table,
        object = object,
        relation = relation,
      }
      if type(subject) == "string" then
        params.subject_id = subject
      elseif type(subject) == "table" then
        params.subject_set_namespace = subject.namespace
        params.subject_set_object = subject.object
        params.subject_set_relation = subject.relation
      end
    end
    return params
  end

  -- ========== Sub-objects ==========

  local c = {}

  -- ========== c.tuples ==========

  c.tuples = {}

  -- List relation tuples.
  -- Filters: namespace, object, relation, subject_id, subject_set_namespace,
  -- subject_set_object, subject_set_relation, page_size, page_token
  function c.tuples:list(filters)
    return read_get("/relation-tuples" .. build_query(filters or {}))
  end

  -- Create a relation tuple (requires write_url).
  -- tuple: { namespace, object, relation, subject_id } or with subject_set
  function c.tuples:create(tuple)
    require_write()
    local resp = http.put(write .. "/admin/relation-tuples", tuple)
    if resp.status ~= 201 and resp.status ~= 200 then
      error("keto: create tuple HTTP " .. resp.status .. ": " .. resp.body)
    end
  end

  -- Delete a specific relation tuple (requires write_url).
  -- Supports both subject_id and subject_set tuples.
  function c.tuples:delete(tuple)
    require_write()
    local params = {
      namespace = tuple.namespace,
      object = tuple.object,
      relation = tuple.relation,
    }
    if tuple.subject_id then
      params.subject_id = tuple.subject_id
    elseif tuple.subject_set then
      params.subject_set_namespace = tuple.subject_set.namespace
      params.subject_set_object = tuple.subject_set.object
      params.subject_set_relation = tuple.subject_set.relation
    end
    local resp = http.delete(write .. "/admin/relation-tuples" .. build_query(params))
    if resp.status ~= 204 and resp.status ~= 200 then
      error("keto: delete tuple HTTP " .. resp.status .. ": " .. resp.body)
    end
  end

  -- Delete all relation tuples matching the filters (requires write_url).
  function c.tuples:delete_all(filters)
    require_write()
    local resp = http.delete(write .. "/admin/relation-tuples" .. build_query(filters))
    if resp.status ~= 204 and resp.status ~= 200 then
      error("keto: delete_all HTTP " .. resp.status .. ": " .. resp.body)
    end
  end

  -- ========== c.permissions ==========

  c.permissions = {}

  -- Check if a subject has a given relation to an object.
  -- Two call styles:
  --   c.permissions:check(namespace, object, relation, subject)             -- positional
  --   c.permissions:check({ namespace=..., object=..., relation=..., subject_id=... })  -- table
  -- subject (positional) can be a string ("user:abc") or a subject_set table.
  -- Works with OPL permits: if the relation names a permit, Keto evaluates
  -- the rewrite rules and returns true/false.
  function c.permissions:check(namespace_or_table, object, relation, subject)
    local params = build_check_params(namespace_or_table, object, relation, subject)
    local resp = http.get(read .. "/relation-tuples/check" .. build_query(params))
    if resp.status == 200 then
      local data = json.parse(resp.body)
      return data.allowed == true
    elseif resp.status == 403 then
      return false
    end
    error("keto: check failed HTTP " .. resp.status .. ": " .. resp.body)
  end

  -- Batch check: check multiple tuples in one call.
  -- Each entry can be positional-style {namespace, object, relation, subject_id}
  -- or table-style (same as single check).
  -- Returns a list of booleans in the same order.
  function c.permissions:batch_check(tuples)
    local results = {}
    for _, t in ipairs(tuples) do
      local params = build_check_params(t)
      local resp = http.get(read .. "/relation-tuples/check" .. build_query(params))
      if resp.status == 200 then
        local data = json.parse(resp.body)
        results[#results + 1] = data.allowed == true
      elseif resp.status == 403 then
        results[#results + 1] = false
      else
        error("keto: batch_check failed HTTP " .. resp.status .. ": " .. resp.body)
      end
    end
    return results
  end

  -- Expand a relation to see all direct and transitive members.
  function c.permissions:expand(namespace, object, relation, depth)
    local params = {
      namespace = namespace,
      object = object,
      relation = relation,
      ["max-depth"] = tostring(depth or 3),
    }
    return read_get("/relation-tuples/expand" .. build_query(params))
  end

  -- ========== c.roles ==========

  c.roles = {}

  -- Helper: get all role memberships for a user.
  -- By default queries the "Role" namespace. Pass a different namespace
  -- if your application uses a native OPL namespace.
  -- Returns a list of { object, relation } entries.
  function c.roles:user_roles(user_id, namespace)
    local subject = user_id
    if not user_id:match("^user:") then
      subject = "user:" .. user_id
    end
    local result = c.tuples:list({
      namespace = namespace or "Role",
      relation = "members",
      subject_id = subject,
    })
    local roles = {}
    for _, tuple in ipairs(result.relation_tuples or {}) do
      roles[#roles + 1] = { object = tuple.object, relation = tuple.relation }
    end
    return roles
  end

  -- Helper: check if a user has any of the given role objects (e.g. {"app:admin", "app:operator"})
  function c.roles:has_any(user_id, role_objects, namespace)
    local roles = c.roles:user_roles(user_id, namespace)
    local set = {}
    for _, r in ipairs(roles) do set[r.object] = true end
    for _, target in ipairs(role_objects) do
      if set[target] then return true end
    end
    return false
  end

  return c
end

return M
