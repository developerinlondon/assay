--- @module assay.keto
--- @description Ory Keto authorization — relation-tuple CRUD, permission checks, role membership queries. Implements relationship-based access control (ReBAC, Google Zanzibar-style).
--- @keywords keto, ory, authorization, authz, rbac, rebac, permissions, roles, relation-tuples, zanzibar, access-control, members, groups
--- @quickref keto.client(read_url, opts?) -> client | Create a Keto client (read endpoint)
--- @quickref c:list(opts) -> {relation_tuples, next_page_token} | List relation tuples matching filters
--- @quickref c:check(namespace, object, relation, subject) -> bool | Check if a subject has a relation to an object
--- @quickref c:expand(namespace, object, relation, depth?) -> tree | Expand a relation to see all members
--- @quickref c:get_user_roles(user_id, namespace?) -> [{object, relation}] | Get all role memberships for a user
--- @quickref c:create(tuple) -> nil | Create a relation tuple (requires write_url)
--- @quickref c:delete(tuple) -> nil | Delete a relation tuple (requires write_url)
--- @quickref c:delete_all(filters) -> nil | Delete all matching relation tuples (requires write_url)

local M = {}

-- Create a Keto client.
-- Pass a single URL for read-only, or opts.write_url for write operations.
-- Example:
--   local k = keto.client("http://keto-read:4466")
--   local k = keto.client("http://keto-read:4466", { write_url = "http://keto-write:4467" })
function M.client(read_url, opts)
  opts = opts or {}
  local c = {
    read_url = read_url:gsub("/+$", ""),
    write_url = opts.write_url and opts.write_url:gsub("/+$", "") or nil,
  }

  local function build_query(params)
    local parts = {}
    for k, v in pairs(params) do
      if v ~= nil and v ~= "" then
        parts[#parts + 1] = k .. "=" .. v
      end
    end
    if #parts == 0 then return "" end
    return "?" .. table.concat(parts, "&")
  end

  local function read_get(self, path_str)
    local resp = http.get(self.read_url .. path_str)
    if resp.status ~= 200 then
      error("keto: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function require_write(self)
    if not self.write_url then
      error("keto: write_url not configured — pass opts.write_url to keto.client()")
    end
  end

  -- List relation tuples. Filters: namespace, object, relation, subject_id, subject_set_namespace, subject_set_object, subject_set_relation, page_size, page_token
  function c:list(filters)
    return read_get(self, "/relation-tuples" .. build_query(filters or {}))
  end

  -- Check if a subject has a given relation to an object.
  -- subject can be a string ("user:abc") or a table ({ namespace="...", object="...", relation="..." })
  -- Returns true if allowed, false otherwise.
  function c:check(namespace, object, relation, subject)
    local params = {
      namespace = namespace,
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
    local resp = http.get(self.read_url .. "/relation-tuples/check" .. build_query(params))
    if resp.status == 200 then
      local data = json.parse(resp.body)
      return data.allowed == true
    elseif resp.status == 403 then
      return false
    end
    error("keto: check failed HTTP " .. resp.status .. ": " .. resp.body)
  end

  -- Expand a relation to see all direct and transitive members.
  function c:expand(namespace, object, relation, depth)
    local params = {
      namespace = namespace,
      object = object,
      relation = relation,
      ["max-depth"] = depth or 3,
    }
    return read_get(self, "/relation-tuples/expand" .. build_query(params))
  end

  -- Helper: get all role memberships for a user.
  -- By default queries the "Role" namespace, the standard convention for
  -- modelling RBAC-style memberships in Keto. Pass a different namespace
  -- if your application uses one.
  -- Returns a list of { object, relation } entries, e.g.
  --   { {object="app:role-a", relation="members"}, {object="app:role-b", relation="members"} }
  function c:get_user_roles(user_id, namespace)
    local subject = user_id
    if not user_id:match("^user:") then
      subject = "user:" .. user_id
    end
    local result = self:list({
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
  function c:user_has_any_role(user_id, role_objects, namespace)
    local roles = self:get_user_roles(user_id, namespace)
    local set = {}
    for _, r in ipairs(roles) do set[r.object] = true end
    for _, target in ipairs(role_objects) do
      if set[target] then return true end
    end
    return false
  end

  -- Create a relation tuple (requires write_url).
  -- tuple: { namespace, object, relation, subject_id (or subject_set) }
  function c:create(tuple)
    require_write(self)
    local resp = http.put(self.write_url .. "/admin/relation-tuples", tuple)
    if resp.status ~= 201 and resp.status ~= 200 then
      error("keto: create tuple HTTP " .. resp.status .. ": " .. resp.body)
    end
  end

  -- Delete a specific relation tuple (requires write_url).
  function c:delete(tuple)
    require_write(self)
    local params = {
      namespace = tuple.namespace,
      object = tuple.object,
      relation = tuple.relation,
      subject_id = tuple.subject_id,
    }
    local resp = http.delete(self.write_url .. "/admin/relation-tuples" .. build_query(params))
    if resp.status ~= 204 and resp.status ~= 200 then
      error("keto: delete tuple HTTP " .. resp.status .. ": " .. resp.body)
    end
  end

  -- Delete all relation tuples matching the filters (requires write_url).
  function c:delete_all(filters)
    require_write(self)
    local resp = http.delete(self.write_url .. "/admin/relation-tuples" .. build_query(filters))
    if resp.status ~= 204 and resp.status ~= 200 then
      error("keto: delete_all HTTP " .. resp.status .. ": " .. resp.body)
    end
  end

  return c
end

return M
