--- @module assay.kratos
--- @description Ory Kratos identity management — login/registration/recovery flows, identity CRUD via admin API, session introspection, schemas.
--- @keywords kratos, ory, identity, authentication, login, registration, recovery, settings, sessions, identities, schemas, whoami
--- @quickref kratos.client(opts) -> client | Create a Kratos client. opts: {public_url, admin_url}
--- @quickref c:whoami(cookie) -> {identity, expires_at, ...} | Check if the current session is valid
--- @quickref c:create_login_flow(opts?) -> {id, ui, return_to, ...} | Create a browser login flow
--- @quickref c:get_login_flow(id, cookie?) -> flow | Fetch an existing login flow
--- @quickref c:submit_login_flow(flow_id, payload, cookie?) -> {session, ...} | Submit a login flow
--- @quickref c:create_registration_flow() -> flow | Create a registration flow
--- @quickref c:get_identity(id) -> identity | Get an identity by ID (admin API)
--- @quickref c:list_identities(opts?) -> [identity] | List all identities (admin API)
--- @quickref c:create_identity(spec) -> identity | Create an identity (admin API)
--- @quickref c:update_identity(id, spec) -> identity | Update an identity (admin API)
--- @quickref c:delete_identity(id) -> nil | Delete an identity (admin API)
--- @quickref c:list_sessions(id) -> [session] | List active sessions for an identity
--- @quickref c:delete_sessions(id) -> nil | Revoke all sessions for an identity
--- @quickref c:list_schemas() -> [schema] | List identity schemas

local M = {}

local function urlencode(s)
  return (tostring(s):gsub("([^%w%-%.%_%~])", function(c)
    return string.format("%%%02X", string.byte(c))
  end))
end

-- Create a Kratos client. Pass opts.public_url for public API (login flows, whoami)
-- and opts.admin_url for admin API (identity CRUD, session management).
function M.client(opts)
  opts = opts or {}
  local c = {
    public_url = opts.public_url and opts.public_url:gsub("/+$", "") or nil,
    admin_url = opts.admin_url and opts.admin_url:gsub("/+$", "") or nil,
  }

  local function require_public(self)
    if not self.public_url then
      error("kratos: public_url not configured")
    end
  end

  local function require_admin(self)
    if not self.admin_url then
      error("kratos: admin_url not configured")
    end
  end

  local function public_get(self, path_str, cookie)
    require_public(self)
    local headers = {}
    if cookie then headers["Cookie"] = cookie end
    local resp = http.get(self.public_url .. path_str, { headers = headers })
    if resp.status ~= 200 then
      error("kratos: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function admin_get(self, path_str)
    require_admin(self)
    local resp = http.get(self.admin_url .. path_str)
    if resp.status ~= 200 and resp.status ~= 404 then
      error("kratos: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    if resp.status == 404 then return nil end
    return json.parse(resp.body)
  end

  local function admin_post(self, path_str, payload)
    require_admin(self)
    local resp = http.post(self.admin_url .. path_str, payload)
    if resp.status ~= 200 and resp.status ~= 201 then
      error("kratos: POST " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function admin_put(self, path_str, payload)
    require_admin(self)
    local resp = http.put(self.admin_url .. path_str, payload)
    if resp.status ~= 200 then
      error("kratos: PUT " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  -- ========== Session ==========

  -- Check if the current session is valid. Pass the user's cookie header.
  -- Returns the session object or nil if not authenticated.
  function c:whoami(cookie)
    require_public(self)
    local headers = {}
    if cookie then headers["Cookie"] = cookie end
    local resp = http.get(self.public_url .. "/sessions/whoami", { headers = headers })
    if resp.status == 200 then
      return json.parse(resp.body)
    elseif resp.status == 401 then
      return nil
    end
    error("kratos: whoami HTTP " .. resp.status .. ": " .. resp.body)
  end

  -- ========== Login Flows ==========

  -- Create a login flow. opts: { return_to, refresh, login_challenge, aal }
  function c:create_login_flow(opts)
    opts = opts or {}
    local params = {}
    if opts.return_to then params[#params + 1] = "return_to=" .. urlencode(opts.return_to) end
    if opts.refresh then params[#params + 1] = "refresh=true" end
    if opts.login_challenge then params[#params + 1] = "login_challenge=" .. urlencode(opts.login_challenge) end
    if opts.aal then params[#params + 1] = "aal=" .. urlencode(opts.aal) end
    local qs = ""
    if #params > 0 then qs = "?" .. table.concat(params, "&") end
    return public_get(self, "/self-service/login/browser" .. qs)
  end

  function c:get_login_flow(flow_id, cookie)
    return public_get(self, "/self-service/login/flows?id=" .. urlencode(flow_id), cookie)
  end

  function c:submit_login_flow(flow_id, payload, cookie)
    require_public(self)
    local headers = { ["Content-Type"] = "application/json" }
    if cookie then headers["Cookie"] = cookie end
    local resp = http.post(self.public_url .. "/self-service/login?flow=" .. urlencode(flow_id), payload, {
      headers = headers,
    })
    if resp.status ~= 200 then
      error("kratos: submit login HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  -- ========== Registration Flows ==========

  function c:create_registration_flow(opts)
    opts = opts or {}
    local qs = ""
    if opts.return_to then qs = "?return_to=" .. urlencode(opts.return_to) end
    return public_get(self, "/self-service/registration/browser" .. qs)
  end

  function c:get_registration_flow(flow_id, cookie)
    return public_get(self, "/self-service/registration/flows?id=" .. urlencode(flow_id), cookie)
  end

  -- ========== Identity CRUD (Admin API) ==========

  function c:get_identity(id)
    return admin_get(self, "/admin/identities/" .. urlencode(id))
  end

  function c:list_identities(opts)
    opts = opts or {}
    local params = {}
    if opts.per_page then params[#params + 1] = "per_page=" .. opts.per_page end
    if opts.page then params[#params + 1] = "page=" .. opts.page end
    if opts.credentials_identifier then
      params[#params + 1] = "credentials_identifier=" .. urlencode(opts.credentials_identifier)
    end
    local qs = ""
    if #params > 0 then qs = "?" .. table.concat(params, "&") end
    return admin_get(self, "/admin/identities" .. qs)
  end

  function c:create_identity(spec)
    return admin_post(self, "/admin/identities", spec)
  end

  function c:update_identity(id, spec)
    return admin_put(self, "/admin/identities/" .. urlencode(id), spec)
  end

  function c:delete_identity(id)
    require_admin(self)
    local resp = http.delete(self.admin_url .. "/admin/identities/" .. urlencode(id))
    if resp.status ~= 204 and resp.status ~= 200 then
      error("kratos: delete identity HTTP " .. resp.status .. ": " .. resp.body)
    end
  end

  -- ========== Session Management (Admin API) ==========

  function c:list_sessions(identity_id)
    return admin_get(self, "/admin/identities/" .. urlencode(identity_id) .. "/sessions")
  end

  function c:delete_sessions(identity_id)
    require_admin(self)
    local resp = http.delete(self.admin_url .. "/admin/identities/" .. urlencode(identity_id) .. "/sessions")
    if resp.status ~= 204 and resp.status ~= 200 then
      error("kratos: delete sessions HTTP " .. resp.status .. ": " .. resp.body)
    end
  end

  -- ========== Schemas ==========

  function c:list_schemas()
    require_public(self)
    local resp = http.get(self.public_url .. "/schemas")
    if resp.status ~= 200 then
      error("kratos: list schemas HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  function c:get_schema(schema_id)
    return public_get(self, "/schemas/" .. urlencode(schema_id))
  end

  return c
end

return M
