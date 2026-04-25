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
--- @quickref c.zanzibar:delete(tuple) -> nil | Admin remove a relation tuple
--- @quickref c.jwks:get() -> {keys} | Admin JWKS proxy
--- @quickref c.oidc_provider:discovery() -> table | Public OIDC discovery
--- @quickref c.oidc_provider:jwks() -> {keys} | Public JWKS
--- @quickref c.oidc_provider:authorize_url(params) -> URL string | Build /auth/authorize URL
--- @quickref c.oidc_provider:token(body) -> {access_token, ...} | RFC 6749 token exchange
--- @quickref c.oidc_provider:userinfo({access_token}) -> claims | OIDC userinfo
--- @quickref c.oidc_provider:revoke(body) -> ok | RFC 7009 revoke
--- @quickref c.oidc_provider:introspect(token) -> {active, ...} | RFC 7662 introspect
--- @quickref c.oidc_provider:consent(body) -> ok | Record consent + resume authorize
--- @quickref c.oidc_provider:logout() -> {redirect_url} | RP-initiated logout
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

  local function del(path, admin, body)
    -- DELETE with a body is required by `/admin/auth/zanzibar/tuples`
    -- (the row to remove is identified by JSON, not a path param). The
    -- assay http binding accepts `opts.body` (string OR table) and
    -- auto-sets Content-Type when a table is passed; we route table
    -- bodies through that path so the wire shape matches POST.
    local opts = { headers = build_headers(admin) }
    if body ~= nil then opts.body = body end
    return decode(http.delete(engine_url .. path, opts), true)
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

  --- Remove a relation tuple. The body shape matches `:write` — same
  --- {object_type, object_id, relation, subject_type, subject_id,
  --- subject_rel?} record. Returns nil on 204; raises on 404 / 5xx.
  function c.zanzibar:delete(tuple)
    return del("/auth/admin/auth/zanzibar/tuples", true, tuple)
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

  -- ===== Admin: JWKS =====

  c.jwks = {}

  --- GET /auth/admin/auth/jwks — admin-gated JWKS proxy. The same key
  --- material is also available unauthenticated via the OIDC discovery
  --- endpoint (`c.oidc_provider:jwks()`); the admin path is provided
  --- so dashboards can fetch keys behind the admin auth boundary.
  function c.jwks:get()
    return get("/auth/admin/auth/jwks", true)
  end

  -- ===== OIDC provider (public spec endpoints) =====
  --
  -- These are the OIDC-spec endpoints the engine implements as a
  -- *provider* (RFC 6749 / OIDC Core 1.0). They're typically called
  -- by external clients out-of-band, but exposing them in Lua lets
  -- assay scripts probe a deployment for spec conformance and drive
  -- end-to-end OIDC flows from a test harness.

  c.oidc_provider = {}

  --- GET /auth/.well-known/openid-configuration — discovery document
  --- (issuer, endpoint URLs, supported scopes/algos/etc).
  function c.oidc_provider:discovery()
    return get("/auth/.well-known/openid-configuration", false)
  end

  --- GET /auth/.well-known/jwks.json — public JWKS (no auth).
  function c.oidc_provider:jwks()
    return get("/auth/.well-known/jwks.json", false)
  end

  --- Build the `/auth/authorize` URL for a redirect-based flow. Returns
  --- the URL string — callers send the user-agent there. We don't
  --- follow the redirect here because authorize lands in HTML/consent
  --- UIs that aren't useful from a script context.
  --- @param params table {client_id, redirect_uri, response_type?, scope?, state?, code_challenge?, code_challenge_method?, nonce?, prompt?}
  function c.oidc_provider:authorize_url(params)
    params = params or {}
    local parts = {}
    -- url_encode is local — encode each value to dodge `&`/`=`/spaces.
    local function enc(s)
      return (tostring(s):gsub("([^A-Za-z0-9%-_.~])", function(ch)
        return string.format("%%%02X", string.byte(ch))
      end))
    end
    for k, v in pairs(params) do
      if v ~= nil and v ~= "" then
        parts[#parts + 1] = enc(k) .. "=" .. enc(v)
      end
    end
    local q = (#parts > 0) and ("?" .. table.concat(parts, "&")) or ""
    return engine_url .. "/auth/authorize" .. q
  end

  --- POST /auth/token — token exchange (authorization_code, refresh_token,
  --- client_credentials). Body is a table of form params; we send it as
  --- JSON. Most IdPs accept either application/json or
  --- application/x-www-form-urlencoded — assay's provider accepts both.
  function c.oidc_provider:token(body)
    return post("/auth/token", body, false)
  end

  --- GET /auth/userinfo — userinfo endpoint. Pass the access_token via
  --- `opts.access_token`; the wrapper threads it through the
  --- `Authorization: Bearer` header.
  function c.oidc_provider:userinfo(opts)
    opts = opts or {}
    local headers = build_headers(false)
    if opts.access_token and opts.access_token ~= "" then
      headers["Authorization"] = "Bearer " .. opts.access_token
    end
    return decode(http.get(engine_url .. "/auth/userinfo", { headers = headers }))
  end

  --- POST /auth/revoke — RFC 7009 token revocation. Body is
  --- `{token, token_type_hint?}`.
  function c.oidc_provider:revoke(body)
    return post("/auth/revoke", body, false)
  end

  --- POST /auth/introspect — RFC 7662 token introspection.
  --- Returns `{active = bool, ...}`.
  function c.oidc_provider:introspect(token)
    return post("/auth/introspect", { token = token }, false)
  end

  --- POST /auth/authorize/consent — record the user's consent decision
  --- and resume the authorize flow. Body shape is provider-specific
  --- (typically `{state, decision = "allow"|"deny"}`).
  function c.oidc_provider:consent(body)
    return post("/auth/authorize/consent", body, false)
  end

  --- GET /auth/logout — RP-initiated logout endpoint.
  function c.oidc_provider:logout()
    -- Mirrors the upstream-OIDC pattern: surface the redirect URL so
    -- the caller can follow it out-of-band.
    local resp = http.get(engine_url .. "/auth/logout", {
      headers = build_headers(false),
    })
    if resp.status == 302 or resp.status == 301 then
      return { redirect_url = resp.headers and resp.headers.location, status = resp.status }
    end
    return { status = resp.status, body = resp.body, headers = resp.headers }
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
