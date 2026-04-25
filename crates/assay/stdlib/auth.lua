--- @module assay.auth
--- @description Lua wrapper for assay-engine's auth module — login/whoami, passkey, OIDC, biscuit, zanzibar, and admin (users, sessions, OIDC clients).
--- @keywords auth, login, session, passkey, oidc, biscuit, zanzibar, rebac, admin, users, sessions
--- @quickref auth.client(opts) -> client | Construct an auth client (engine_url + optional api_key)
--- @quickref c:login(email, password) -> {session_id, csrf_token, user} | Password login
--- @quickref c:logout() -> nil | Revoke the current session cookie
--- @quickref c:whoami() -> User|nil | Resolve the current session
--- @quickref c.passkey:start_register(user_id, user_name, display) | Start a passkey registration ceremony
--- @quickref c.passkey:finish_register(reg_response, state) | Complete a passkey registration
--- @quickref c.passkey:start_auth(user_id, passkeys) | Start a passkey authentication ceremony
--- @quickref c.passkey:finish_auth(response, state) | Complete a passkey authentication
--- @quickref c.oidc:start(provider_slug) -> redirect URL | Start federated SSO
--- @quickref c.oidc:complete(provider_slug, code, state) | Complete federated SSO
--- @quickref c.biscuit:public_pem() -> string | Fetch the engine's biscuit root public key (PEM)
--- @quickref c.zanzibar:check(resource_type, resource_id, perm, subject_type, subject_id) -> bool | Permission check
--- @quickref c.zanzibar:expand(resource_type, resource_id, relation) -> tree | Userset expand
--- @quickref c.zanzibar:write(tuple) -> ok | Admin write a relation tuple
--- @quickref c.users:list({limit, offset, search}) -> {items, total, ...} | Admin list users
--- @quickref c.users:create({email, display_name, password, email_verified}) -> User | Admin create
--- @quickref c.users:get(id) -> {user, passkeys, sessions, upstream} | Admin get user detail
--- @quickref c.users:update(id, body) -> User | Admin update user
--- @quickref c.users:delete(id) -> nil | Admin hard-delete a user (cascades)
--- @quickref c.sessions:list_for_user(user_id) -> {items, total, ...} | Admin list sessions for one user
--- @quickref c.sessions:revoke(session_id) -> nil | Admin revoke a single session
--- @quickref c.sessions:revoke_all_for_user(user_id) -> {revoked} | Admin revoke every session for a user
--- @quickref c.oidc_clients:list() -> [client] | Admin list OIDC consumer apps
--- @quickref c.oidc_clients:create(body) -> {client, client_secret} | Admin register a new OIDC client (secret returned ONCE)
--- @quickref c.oidc_clients:rotate_secret(id) -> {client_id, client_secret} | Rotate client_secret (returned ONCE)

local M = {}

local function trim_slash(s)
  return (s or ""):gsub("/+$", "")
end

--- Build an auth client.
---
--- opts:
---   engine_url       (string, required)  base URL of the assay-engine, e.g. "http://localhost:3000"
---   api_key          (string, optional)  admin bearer token; if absent and ASSAY_API_KEY is set, that's used
---   session_cookie   (string, optional)  pre-existing session cookie value to send on user-facing routes
function M.client(opts)
  opts = opts or {}
  local engine_url = trim_slash(opts.engine_url or env.get("ASSAY_ENGINE_URL") or "")
  if engine_url == "" then
    error("assay.auth: engine_url required (or set ASSAY_ENGINE_URL)")
  end
  local api_key = opts.api_key or env.get("ASSAY_API_KEY")
  local session_cookie = opts.session_cookie

  local function build_headers(admin)
    local h = { ["Content-Type"] = "application/json" }
    if admin and api_key and api_key ~= "" then
      h["Authorization"] = "Bearer " .. api_key
    end
    if session_cookie and session_cookie ~= "" then
      h["Cookie"] = "assay_session=" .. session_cookie
    end
    return h
  end

  local function decode(resp, allow_empty)
    if resp.status >= 200 and resp.status < 300 then
      if allow_empty and (resp.status == 204 or resp.body == "" or resp.body == nil) then
        return nil
      end
      if resp.body == nil or resp.body == "" then
        return nil
      end
      return json.parse(resp.body)
    end
    error("assay.auth: HTTP " .. tostring(resp.status) .. ": " .. (resp.body or ""))
  end

  local function get(path, admin)
    return decode(http.get(engine_url .. path, { headers = build_headers(admin) }))
  end

  local function post(path, body, admin)
    return decode(http.post(engine_url .. path, body or {}, { headers = build_headers(admin) }), true)
  end

  local function put(path, body, admin)
    return decode(http.put(engine_url .. path, body or {}, { headers = build_headers(admin) }), true)
  end

  local function del(path, admin)
    return decode(http.delete(engine_url .. path, { headers = build_headers(admin) }), true)
  end

  local c = {}

  -- ===== Auth flow =====

  function c:login(email, password)
    local result = post("/auth/login", { email = email, password = password })
    if result and result.csrf_token then
      -- Capture session cookie from Set-Cookie if available — http.post
      -- doesn't currently surface response headers in a structured way,
      -- so callers should pass session_cookie back via opts on the next
      -- client construction.
      session_cookie = result.session_id or session_cookie
    end
    return result
  end

  function c:logout()
    return del("/auth/session")
  end

  function c:whoami()
    local resp = http.get(engine_url .. "/auth/whoami", { headers = build_headers(false) })
    if resp.status == 401 or resp.status == 404 then return nil end
    return decode(resp)
  end

  -- ===== Passkey =====

  c.passkey = {}

  function c.passkey:start_register(user_id, user_name, display_name)
    return post("/auth/passkey/register/start", {
      user_id = user_id,
      user_name = user_name or user_id,
      display_name = display_name or user_id,
    })
  end

  function c.passkey:finish_register(user_id, reg_response, state)
    return post("/auth/passkey/register/finish", {
      user_id = user_id,
      response = reg_response,
      state = state,
    })
  end

  function c.passkey:start_auth(user_id, passkeys)
    return post("/auth/passkey/auth/start", {
      user_id = user_id,
      passkeys = passkeys or {},
    })
  end

  function c.passkey:finish_auth(response, state)
    return post("/auth/passkey/auth/finish", {
      response = response,
      state = state,
    })
  end

  -- ===== OIDC client (federated SSO consumer side) =====

  c.oidc = {}

  function c.oidc:start(provider_slug)
    -- /auth/oidc/upstream/{slug}/start performs a 302 redirect — we
    -- expose the redirect URL via the Location header so callers can
    -- complete the flow out-of-band.
    local resp = http.get(engine_url .. "/auth/oidc/upstream/" .. provider_slug .. "/start", {
      headers = build_headers(false),
    })
    if resp.status == 302 or resp.status == 301 then
      return { redirect_url = resp.headers and resp.headers.location, status = resp.status }
    end
    return decode(resp)
  end

  function c.oidc:complete(provider_slug, code, state)
    -- The callback typically lands in the browser and redirects again
    -- — expose the raw response so callers can inspect it.
    local resp = http.get(engine_url .. "/auth/oidc/upstream/" .. provider_slug .. "/callback?code="
      .. code .. "&state=" .. state, { headers = build_headers(false) })
    if resp.status >= 200 and resp.status < 400 then
      return { status = resp.status, headers = resp.headers, body = resp.body }
    end
    error("assay.auth.oidc.complete: HTTP " .. resp.status .. ": " .. (resp.body or ""))
  end

  -- ===== Biscuit =====

  c.biscuit = {}

  -- Fetch the engine's biscuit root public key (PEM). Cached on the
  -- client object so subsequent calls don't re-hit the engine.
  function c.biscuit:public_pem()
    if self._public_pem then return self._public_pem end
    local info = get("/auth/admin/auth/biscuit", true)
    self._public_pem = info.public_pem
    return info.public_pem
  end

  function c.biscuit:active_kid()
    if self._kid then return self._kid end
    local info = get("/auth/admin/auth/biscuit", true)
    self._kid = info.kid
    self._public_pem = info.public_pem
    return info.kid
  end

  -- Note: biscuit `verify` and `attenuate` are local-only operations
  -- in the Rust crate but require linking against `biscuit-auth`.
  -- assay-lua doesn't carry that dep; instead we expose the engine's
  -- public material so callers can verify out-of-band with a Lua
  -- biscuit binding when one is added. For now `verify` round-trips
  -- to the engine's introspect endpoint when available.
  function c.biscuit:verify(token)
    local resp = http.post(engine_url .. "/auth/introspect", { token = token }, {
      headers = build_headers(false),
    })
    if resp.status >= 200 and resp.status < 300 then
      local r = json.parse(resp.body)
      return r.active == true, r
    end
    return false, { error = resp.body, status = resp.status }
  end

  -- ===== Zanzibar =====

  c.zanzibar = {}

  function c.zanzibar:check(resource_type, resource_id, permission, subject_type, subject_id, subject_rel)
    local r = post("/auth/admin/auth/zanzibar/check", {
      resource_type = resource_type,
      resource_id = resource_id,
      permission = permission,
      subject_type = subject_type,
      subject_id = subject_id,
      subject_rel = subject_rel,
    }, true)
    return r and r.allowed == true, r
  end

  function c.zanzibar:expand(resource_type, resource_id, relation, depth)
    return post("/auth/admin/auth/zanzibar/expand", {
      resource_type = resource_type,
      resource_id = resource_id,
      relation = relation,
      depth_limit = depth,
    }, true)
  end

  function c.zanzibar:write(tuple)
    return post("/auth/admin/auth/zanzibar/tuples", tuple, true)
  end

  function c.zanzibar:list_namespaces()
    return get("/auth/admin/auth/zanzibar/namespaces", true)
  end

  function c.zanzibar:get_namespace(name)
    return get("/auth/admin/auth/zanzibar/namespaces/" .. name, true)
  end

  -- ===== Admin: users =====

  c.users = {}

  function c.users:list(opts)
    opts = opts or {}
    local q = "?limit=" .. (opts.limit or 50) .. "&offset=" .. (opts.offset or 0)
    if opts.search and opts.search ~= "" then
      q = q .. "&search=" .. opts.search
    end
    return get("/auth/admin/auth/users" .. q, true)
  end

  function c.users:get(id)
    return get("/auth/admin/auth/users/" .. id, true)
  end

  function c.users:create(body)
    return post("/auth/admin/auth/users", body, true)
  end

  function c.users:update(id, body)
    return put("/auth/admin/auth/users/" .. id, body, true)
  end

  function c.users:delete(id)
    return del("/auth/admin/auth/users/" .. id, true)
  end

  function c.users:reset_password(id, password)
    return post("/auth/admin/auth/users/" .. id .. "/password-reset",
      { password = password }, true)
  end

  -- ===== Admin: sessions =====

  c.sessions = {}

  function c.sessions:list(opts)
    opts = opts or {}
    local q = "?limit=" .. (opts.limit or 50) .. "&offset=" .. (opts.offset or 0)
    if opts.user_id and opts.user_id ~= "" then
      q = q .. "&user_id=" .. opts.user_id
    end
    return get("/auth/admin/auth/sessions" .. q, true)
  end

  function c.sessions:list_for_user(user_id)
    return c.sessions:list({ user_id = user_id })
  end

  function c.sessions:revoke(session_id)
    return del("/auth/admin/auth/sessions/" .. session_id, true)
  end

  function c.sessions:revoke_all_for_user(user_id)
    return del("/auth/admin/auth/sessions/by-user/" .. user_id, true)
  end

  -- ===== Admin: OIDC clients =====

  c.oidc_clients = {}

  function c.oidc_clients:list()
    return get("/auth/admin/oidc/clients", true)
  end

  function c.oidc_clients:get(id)
    return get("/auth/admin/oidc/clients/" .. id, true)
  end

  function c.oidc_clients:create(body)
    return post("/auth/admin/oidc/clients", body, true)
  end

  function c.oidc_clients:update(id, body)
    return put("/auth/admin/oidc/clients/" .. id, body, true)
  end

  function c.oidc_clients:delete(id)
    return del("/auth/admin/oidc/clients/" .. id, true)
  end

  function c.oidc_clients:rotate_secret(id)
    return post("/auth/admin/oidc/clients/" .. id .. "/rotate-secret", nil, true)
  end

  -- ===== Admin: OIDC upstream providers =====

  c.oidc_upstream = {}

  function c.oidc_upstream:list()
    return get("/auth/admin/oidc/upstream", true)
  end

  function c.oidc_upstream:get(slug)
    return get("/auth/admin/oidc/upstream/" .. slug, true)
  end

  function c.oidc_upstream:upsert(body)
    return post("/auth/admin/oidc/upstream", body, true)
  end

  function c.oidc_upstream:delete(slug)
    return del("/auth/admin/oidc/upstream/" .. slug, true)
  end

  -- ===== Admin: audit =====

  c.audit = {}

  function c.audit:list(opts)
    opts = opts or {}
    local q = "?limit=" .. (opts.limit or 50) .. "&offset=" .. (opts.offset or 0)
    if opts.actor then q = q .. "&actor=" .. opts.actor end
    if opts.action then q = q .. "&action=" .. opts.action end
    if opts.since then q = q .. "&since=" .. opts.since end
    if opts.until_ then q = q .. "&until=" .. opts.until_ end
    return get("/auth/admin/auth/audit" .. q, true)
  end

  return c
end

return M
