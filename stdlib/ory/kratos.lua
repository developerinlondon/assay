--- @module assay.ory.kratos
--- @description Ory Kratos identity management — login/registration/recovery/settings flows, identity CRUD via admin API, session introspection, schemas.
--- @keywords kratos, ory, identity, authentication, login, registration, recovery, settings, sessions, identities, schemas, whoami
--- @quickref kratos.client(opts) -> client | Create a Kratos client. opts: {public_url, admin_url}
--- @quickref c.sessions:whoami(cookie) -> {identity, expires_at, ...} | Check if the current session is valid
--- @quickref c.sessions:list(identity_id) -> [session] | List active sessions for an identity
--- @quickref c.sessions:revoke(identity_id) -> nil | Revoke all sessions for an identity
--- @quickref c.flows:create_login(opts?) -> flow | Create a browser login flow
--- @quickref c.flows:get_login(id, cookie?) -> flow | Fetch an existing login flow (public API, needs CSRF cookie)
--- @quickref c.flows:get_login_admin(id) -> flow | Fetch a login flow via admin API (no cookies needed)
--- @quickref c.flows:submit_login(flow_id, payload, cookie?) -> {session, ...} | Submit a login flow
--- @quickref c.flows:create_registration(opts?) -> flow | Create a registration flow
--- @quickref c.flows:get_registration(id, cookie?) -> flow | Fetch a registration flow
--- @quickref c.flows:submit_registration(flow_id, payload, cookie?) -> {identity, session, ...} | Submit a registration flow
--- @quickref c.flows:create_recovery(opts?) -> flow | Create a recovery flow (password reset)
--- @quickref c.flows:get_recovery(id, cookie?) -> flow | Fetch a recovery flow
--- @quickref c.flows:submit_recovery(flow_id, payload, cookie?) -> flow | Submit a recovery flow
--- @quickref c.flows:create_settings(cookie) -> flow | Create a settings flow (profile/password change)
--- @quickref c.flows:get_settings(id, cookie?) -> flow | Fetch a settings flow
--- @quickref c.flows:submit_settings(flow_id, payload, cookie?) -> flow | Submit a settings flow
--- @quickref c.identities:get(id) -> identity | Get an identity by ID (admin API)
--- @quickref c.identities:list(opts?) -> [identity] | List all identities (admin API)
--- @quickref c.identities:create(spec) -> identity | Create an identity (admin API)
--- @quickref c.identities:update(id, spec) -> identity | Update an identity (admin API)
--- @quickref c.identities:delete(id) -> nil | Delete an identity (admin API)
--- @quickref c.schemas:list() -> [schema] | List identity schemas
--- @quickref c.schemas:get(id) -> schema | Get a specific identity schema

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
  local public_url = opts.public_url and opts.public_url:gsub("/+$", "") or nil
  local admin_url = opts.admin_url and opts.admin_url:gsub("/+$", "") or nil

  local function require_public()
    if not public_url then
      error("kratos: public_url not configured")
    end
  end

  local function require_admin()
    if not admin_url then
      error("kratos: admin_url not configured")
    end
  end

  local function public_get(path_str, cookie)
    require_public()
    local headers = {}
    if cookie then headers["Cookie"] = cookie end
    local resp = http.get(public_url .. path_str, { headers = headers })
    if resp.status ~= 200 then
      error("kratos: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function public_post(path_str, payload, cookie)
    require_public()
    local headers = { ["Content-Type"] = "application/json" }
    if cookie then headers["Cookie"] = cookie end
    local resp = http.post(public_url .. path_str, payload, { headers = headers })
    if resp.status ~= 200 and resp.status ~= 201 then
      -- Kratos returns 422 for browser flows that need a redirect (e.g. after registration)
      if resp.status == 422 then
        return json.parse(resp.body)
      end
      error("kratos: POST " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function admin_get(path_str)
    require_admin()
    local resp = http.get(admin_url .. path_str)
    if resp.status ~= 200 and resp.status ~= 404 then
      error("kratos: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    if resp.status == 404 then return nil end
    return json.parse(resp.body)
  end

  local function admin_post(path_str, payload)
    require_admin()
    local resp = http.post(admin_url .. path_str, payload)
    if resp.status ~= 200 and resp.status ~= 201 then
      error("kratos: POST " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function admin_put(path_str, payload)
    require_admin()
    local resp = http.put(admin_url .. path_str, payload)
    if resp.status ~= 200 then
      error("kratos: PUT " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  -- ========== Sub-objects ==========

  local c = {}

  -- ========== c.sessions ==========

  c.sessions = {}

  -- Check if the current session is valid. Pass the user's cookie header.
  -- Returns the session object or nil if not authenticated.
  function c.sessions:whoami(cookie)
    require_public()
    local headers = {}
    if cookie then headers["Cookie"] = cookie end
    local resp = http.get(public_url .. "/sessions/whoami", { headers = headers })
    if resp.status == 200 then
      return json.parse(resp.body)
    elseif resp.status == 401 then
      return nil
    end
    error("kratos: whoami HTTP " .. resp.status .. ": " .. resp.body)
  end

  -- List active sessions for an identity (admin API).
  function c.sessions:list(identity_id)
    return admin_get("/admin/identities/" .. urlencode(identity_id) .. "/sessions")
  end

  -- Revoke all sessions for an identity (admin API).
  function c.sessions:revoke(identity_id)
    require_admin()
    local resp = http.delete(admin_url .. "/admin/identities/" .. urlencode(identity_id) .. "/sessions")
    if resp.status ~= 204 and resp.status ~= 200 then
      error("kratos: delete sessions HTTP " .. resp.status .. ": " .. resp.body)
    end
  end

  -- ========== c.flows ==========

  c.flows = {}

  -- Create a login flow. opts: { return_to, refresh, login_challenge, aal }
  function c.flows:create_login(opts)
    opts = opts or {}
    local params = {}
    if opts.return_to then params[#params + 1] = "return_to=" .. urlencode(opts.return_to) end
    if opts.refresh then params[#params + 1] = "refresh=true" end
    if opts.login_challenge then params[#params + 1] = "login_challenge=" .. urlencode(opts.login_challenge) end
    if opts.aal then params[#params + 1] = "aal=" .. urlencode(opts.aal) end
    local qs = ""
    if #params > 0 then qs = "?" .. table.concat(params, "&") end
    return public_get("/self-service/login/browser" .. qs)
  end

  function c.flows:get_login(flow_id, cookie)
    return public_get("/self-service/login/flows?id=" .. urlencode(flow_id), cookie)
  end

  function c.flows:get_login_admin(flow_id)
    return admin_get("/admin/self-service/login/flows?id=" .. urlencode(flow_id))
  end

  function c.flows:submit_login(flow_id, payload, cookie)
    return public_post("/self-service/login?flow=" .. urlencode(flow_id), payload, cookie)
  end

  -- Create a registration flow. opts: { return_to }
  function c.flows:create_registration(opts)
    opts = opts or {}
    local qs = ""
    if opts.return_to then qs = "?return_to=" .. urlencode(opts.return_to) end
    return public_get("/self-service/registration/browser" .. qs)
  end

  function c.flows:get_registration(flow_id, cookie)
    return public_get("/self-service/registration/flows?id=" .. urlencode(flow_id), cookie)
  end

  -- Submit a registration flow. payload should include method and traits, e.g.:
  --   { method = "password", password = "...", traits = { email = "..." } }
  function c.flows:submit_registration(flow_id, payload, cookie)
    return public_post("/self-service/registration?flow=" .. urlencode(flow_id), payload, cookie)
  end

  -- Create a recovery flow. opts: { return_to }
  function c.flows:create_recovery(opts)
    opts = opts or {}
    local qs = ""
    if opts.return_to then qs = "?return_to=" .. urlencode(opts.return_to) end
    return public_get("/self-service/recovery/browser" .. qs)
  end

  function c.flows:get_recovery(flow_id, cookie)
    return public_get("/self-service/recovery/flows?id=" .. urlencode(flow_id), cookie)
  end

  -- Submit a recovery flow. payload should include method, e.g.:
  --   { method = "code", email = "user@example.com" }
  function c.flows:submit_recovery(flow_id, payload, cookie)
    return public_post("/self-service/recovery?flow=" .. urlencode(flow_id), payload, cookie)
  end

  -- Create a settings flow. Requires an active session (pass cookie).
  function c.flows:create_settings(cookie)
    return public_get("/self-service/settings/browser", cookie)
  end

  function c.flows:get_settings(flow_id, cookie)
    return public_get("/self-service/settings/flows?id=" .. urlencode(flow_id), cookie)
  end

  -- Submit a settings flow. payload depends on method, e.g.:
  --   { method = "password", password = "new-password" }
  --   { method = "profile", traits = { email = "new@example.com" } }
  function c.flows:submit_settings(flow_id, payload, cookie)
    return public_post("/self-service/settings?flow=" .. urlencode(flow_id), payload, cookie)
  end

  -- ========== c.identities ==========

  c.identities = {}

  function c.identities:get(id)
    return admin_get("/admin/identities/" .. urlencode(id))
  end

  function c.identities:list(opts)
    opts = opts or {}
    local params = {}
    if opts.per_page then params[#params + 1] = "per_page=" .. opts.per_page end
    if opts.page then params[#params + 1] = "page=" .. opts.page end
    if opts.credentials_identifier then
      params[#params + 1] = "credentials_identifier=" .. urlencode(opts.credentials_identifier)
    end
    local qs = ""
    if #params > 0 then qs = "?" .. table.concat(params, "&") end
    return admin_get("/admin/identities" .. qs)
  end

  function c.identities:create(spec)
    return admin_post("/admin/identities", spec)
  end

  function c.identities:update(id, spec)
    return admin_put("/admin/identities/" .. urlencode(id), spec)
  end

  function c.identities:delete(id)
    require_admin()
    local resp = http.delete(admin_url .. "/admin/identities/" .. urlencode(id))
    if resp.status ~= 204 and resp.status ~= 200 then
      error("kratos: delete identity HTTP " .. resp.status .. ": " .. resp.body)
    end
  end

  -- ========== c.schemas ==========

  c.schemas = {}

  function c.schemas:list()
    require_public()
    local resp = http.get(public_url .. "/schemas")
    if resp.status ~= 200 then
      error("kratos: list schemas HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  function c.schemas:get(schema_id)
    return public_get("/schemas/" .. urlencode(schema_id))
  end

  return c
end

return M
