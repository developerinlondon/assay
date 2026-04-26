--- @module assay.engine.auth
--- @description Lua client for assay-engine's auth module — login/whoami, passkey, OIDC client + provider, biscuit, zanzibar, and admin (users, sessions, OIDC clients/upstream, JWKS, audit).
--- @keywords auth, login, session, passkey, oidc, biscuit, zanzibar, rebac, admin, users, sessions
--- @quickref auth.client(opts) -> client | Build an auth client (engine_url + optional api_key)
--- @quickref c:login(email, password) -> {user_id, email, csrf_token} | Password login
--- @quickref c:logout() -> nil | Revoke the current session cookie
--- @quickref c:whoami() -> User|nil | Resolve the current session
--- @quickref c.passkey:start_register(...) | Start a passkey registration
--- @quickref c.passkey:finish_register(...) | Complete a passkey registration
--- @quickref c.passkey:start_auth(...) | Start a passkey authentication
--- @quickref c.passkey:finish_auth(...) | Complete a passkey authentication
--- @quickref c.oidc:start(provider_slug) -> {redirect_url} | Start federated SSO
--- @quickref c.oidc:complete(provider_slug, code, state) | Complete federated SSO
--- @quickref c.biscuit:public_pem() -> string | Engine's biscuit root public key (PEM)
--- @quickref c.biscuit:active_kid() -> string | Currently-active biscuit key id
--- @quickref c.zanzibar:check(rt, rid, perm, st, sid, srel?) -> bool, detail | Permission check
--- @quickref c.zanzibar:expand(rt, rid, relation, depth?) -> tree | Userset expand
--- @quickref c.zanzibar:write(tuple) -> ok | Admin write a relation tuple
--- @quickref c.zanzibar:delete(tuple) -> nil | Admin remove a relation tuple
--- @quickref c.jwks:get() -> {keys} | Admin JWKS proxy
--- @quickref c.oidc_provider:discovery() -> table | Public OIDC discovery
--- @quickref c.oidc_provider:jwks() -> {keys} | Public JWKS
--- @quickref c.oidc_provider:authorize_url(params) -> URL | Build /auth/authorize URL
--- @quickref c.oidc_provider:token(body) -> {access_token, ...} | RFC 6749 token exchange
--- @quickref c.oidc_provider:userinfo({access_token}) -> claims | OIDC userinfo
--- @quickref c.oidc_provider:revoke(body) -> ok | RFC 7009 revoke
--- @quickref c.oidc_provider:introspect(token) -> {active, ...} | RFC 7662 introspect
--- @quickref c.users:list({limit, offset, search}) -> {items, total, ...} | Admin list users
--- @quickref c.users:create({email, display_name, password, email_verified}) -> User | Admin create
--- @quickref c.users:get(id) -> {user, passkeys, sessions, upstream} | Admin get user detail
--- @quickref c.users:update(id, body) -> User | Admin update user
--- @quickref c.users:delete(id) -> nil | Admin hard-delete (cascades)
--- @quickref c.users:reset_password(id, password) -> nil | Admin set password
--- @quickref c.sessions:list({limit, offset, user_id}) -> {items, total, ...} | Admin list sessions
--- @quickref c.sessions:revoke(session_id) -> nil | Admin revoke a single session
--- @quickref c.sessions:revoke_all_for_user(user_id) -> {revoked} | Admin revoke every session
--- @quickref c.oidc_clients:list() -> [client] | Admin list OIDC consumer apps
--- @quickref c.oidc_clients:create(body) -> {client, client_secret} | Admin register (secret returned ONCE)
--- @quickref c.oidc_clients:rotate_secret(id) -> {client_id, client_secret} | Rotate secret (returned ONCE)
--- @quickref c.oidc_upstream:list() -> [provider] | Admin list upstream IdPs
--- @quickref c.oidc_upstream:upsert(body) -> provider | Admin add/update upstream IdP
--- @quickref c.audit:list({actor, action, since, until, limit, offset}) -> {items, total, ...} | Admin audit log

local M = {}

local function trim_slash(s) return (s or ""):gsub("/+$", "") end

local function url_encode(s)
  return (tostring(s):gsub("([^A-Za-z0-9%-_.~])", function(ch)
    return string.format("%%%02X", string.byte(ch))
  end))
end

--- Build an auth client.
---
--- opts:
---   engine_url       (string, required)  base URL of the assay-engine
---   api_key          (string, optional)  admin bearer; ASSAY_ADMIN_KEY fallback
---   session_cookie   (string, optional)  pre-existing session cookie value to send on user-facing routes
function M.client(opts)
  opts = opts or {}
  local engine_url = trim_slash(opts.engine_url or env.get("ASSAY_ENGINE_URL") or "")
  if engine_url == "" then
    error("assay.engine.auth: engine_url required (or set ASSAY_ENGINE_URL)")
  end
  local api_key = opts.api_key or env.get("ASSAY_ADMIN_KEY")
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
      if resp.body == nil or resp.body == "" then return nil end
      return json.parse(resp.body)
    end
    error("assay.engine.auth: HTTP " .. tostring(resp.status) .. ": " .. (resp.body or ""))
  end

  local function get(path, admin)
    return decode(http.get(engine_url .. path, { headers = build_headers(admin) }))
  end

  local function post(path, body, admin)
    return decode(
      http.post(engine_url .. path, body or {}, { headers = build_headers(admin) }),
      true
    )
  end

  local function put(path, body, admin)
    return decode(
      http.put(engine_url .. path, body or {}, { headers = build_headers(admin) }),
      true
    )
  end

  -- DELETE with a body is required by `/admin/zanzibar/tuples` (the row
  -- to remove is identified by JSON, not a path param). The assay http
  -- binding accepts `opts.body` (string OR table) and auto-sets
  -- Content-Type when a table is passed.
  local function del(path, admin, body)
    local o = { headers = build_headers(admin) }
    if body ~= nil then o.body = body end
    return decode(http.delete(engine_url .. path, o), true)
  end

  -- Engine-internal paths under /api/v1/engine/auth/*
  local AUTH = "/api/v1/engine/auth"
  -- OIDC spec paths kept under /auth/* (well-known, authorize, token, ...).
  local SPEC = "/auth"

  local c = {}

  -- ===== Auth flow (sessions) =====

  --- POST /api/v1/engine/auth/login — email + password → session cookie.
  function c:login(email, password)
    local result = post(AUTH .. "/login", { email = email, password = password })
    if result and result.csrf_token then
      session_cookie = result.session_id or session_cookie
    end
    return result
  end

  --- DELETE /api/v1/engine/auth/session — revoke the current session.
  function c:logout() return del(AUTH .. "/session") end

  --- GET /api/v1/engine/auth/whoami — resolve session → user.
  function c:whoami()
    local resp = http.get(engine_url .. AUTH .. "/whoami", { headers = build_headers(false) })
    if resp.status == 401 or resp.status == 404 then return nil end
    return decode(resp)
  end

  -- ===== Passkey =====

  c.passkey = {}

  function c.passkey:start_register(user_id, user_name, display_name)
    return post(AUTH .. "/passkey/register/start", {
      user_id = user_id,
      user_name = user_name or user_id,
      display_name = display_name or user_id,
    })
  end

  function c.passkey:finish_register(user_id, reg_response, state)
    return post(AUTH .. "/passkey/register/finish", {
      user_id = user_id,
      response = reg_response,
      state = state,
    })
  end

  function c.passkey:start_auth(user_id, passkeys)
    return post(AUTH .. "/passkey/auth/start", {
      user_id = user_id,
      passkeys = passkeys or {},
    })
  end

  function c.passkey:finish_auth(response, state)
    return post(AUTH .. "/passkey/auth/finish", { response = response, state = state })
  end

  -- ===== OIDC client (federated SSO consumer side) =====

  c.oidc = {}

  --- GET /auth/oidc/upstream/{slug}/start — federation kickoff. The
  --- engine 302s to the upstream IdP; we expose the redirect URL via
  --- the Location header so callers can complete out-of-band.
  function c.oidc:start(provider_slug)
    local resp = http.get(
      engine_url .. SPEC .. "/oidc/upstream/" .. url_encode(provider_slug) .. "/start",
      { headers = build_headers(false) }
    )
    if resp.status == 302 or resp.status == 301 then
      return { redirect_url = resp.headers and resp.headers.location, status = resp.status }
    end
    return decode(resp)
  end

  --- GET /auth/oidc/upstream/{slug}/callback — federation completion.
  function c.oidc:complete(provider_slug, code, state)
    local resp = http.get(
      engine_url .. SPEC .. "/oidc/upstream/" .. url_encode(provider_slug)
        .. "/callback?code=" .. url_encode(code) .. "&state=" .. url_encode(state),
      { headers = build_headers(false) }
    )
    if resp.status >= 200 and resp.status < 400 then
      return { status = resp.status, headers = resp.headers, body = resp.body }
    end
    error("assay.engine.auth.oidc.complete: HTTP " .. resp.status .. ": " .. (resp.body or ""))
  end

  -- ===== Biscuit =====

  c.biscuit = {}

  -- Cached on the client object; the underlying material is stable
  -- across requests so re-fetching every call wastes round-trips.
  function c.biscuit:public_pem()
    if self._public_pem then return self._public_pem end
    local info = get(AUTH .. "/admin/biscuit", true)
    self._public_pem = info.public_pem
    self._kid = info.kid
    return info.public_pem
  end

  function c.biscuit:active_kid()
    if self._kid then return self._kid end
    local info = get(AUTH .. "/admin/biscuit", true)
    self._public_pem = info.public_pem
    self._kid = info.kid
    return info.kid
  end

  --- Round-trips to /auth/introspect to verify a biscuit-style bearer.
  --- Returns `(active_bool, full_response)`.
  function c.biscuit:verify(token)
    local resp = http.post(engine_url .. SPEC .. "/introspect", { token = token }, {
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
    local r = post(AUTH .. "/admin/zanzibar/check", {
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
    return post(AUTH .. "/admin/zanzibar/expand", {
      resource_type = resource_type,
      resource_id = resource_id,
      relation = relation,
      depth_limit = depth,
    }, true)
  end

  function c.zanzibar:write(tuple) return post(AUTH .. "/admin/zanzibar/tuples", tuple, true) end

  --- Remove a relation tuple. Body matches `:write` shape. Returns nil
  --- on 204; raises on 404 / 5xx.
  function c.zanzibar:delete(tuple) return del(AUTH .. "/admin/zanzibar/tuples", true, tuple) end

  --- Persist (or replace) a namespace schema. Use this to seed the
  --- default `engine` / `auth` / `workflow` namespaces — see init.lua.
  function c.zanzibar:define_namespace(schema)
    return post(AUTH .. "/admin/zanzibar/namespaces", schema, true)
  end

  function c.zanzibar:list_namespaces() return get(AUTH .. "/admin/zanzibar/namespaces", true) end

  function c.zanzibar:get_namespace(name)
    return get(AUTH .. "/admin/zanzibar/namespaces/" .. url_encode(name), true)
  end

  -- ===== Admin: users =====

  c.users = {}

  function c.users:list(qopts)
    qopts = qopts or {}
    local q = "?limit=" .. (qopts.limit or 50) .. "&offset=" .. (qopts.offset or 0)
    if qopts.search and qopts.search ~= "" then
      q = q .. "&search=" .. url_encode(qopts.search)
    end
    return get(AUTH .. "/admin/users" .. q, true)
  end

  function c.users:get(id) return get(AUTH .. "/admin/users/" .. url_encode(id), true) end
  function c.users:create(body) return post(AUTH .. "/admin/users", body, true) end
  function c.users:update(id, body) return put(AUTH .. "/admin/users/" .. url_encode(id), body, true) end
  function c.users:delete(id) return del(AUTH .. "/admin/users/" .. url_encode(id), true) end

  function c.users:reset_password(id, password)
    return post(
      AUTH .. "/admin/users/" .. url_encode(id) .. "/password-reset",
      { password = password }, true
    )
  end

  -- ===== Admin: sessions =====

  c.sessions = {}

  function c.sessions:list(qopts)
    qopts = qopts or {}
    local q = "?limit=" .. (qopts.limit or 50) .. "&offset=" .. (qopts.offset or 0)
    if qopts.user_id and qopts.user_id ~= "" then
      q = q .. "&user_id=" .. url_encode(qopts.user_id)
    end
    return get(AUTH .. "/admin/sessions" .. q, true)
  end

  function c.sessions:list_for_user(user_id)
    return c.sessions:list({ user_id = user_id })
  end

  function c.sessions:revoke(session_id)
    return del(AUTH .. "/admin/sessions/" .. url_encode(session_id), true)
  end

  function c.sessions:revoke_all_for_user(user_id)
    return del(AUTH .. "/admin/sessions/by-user/" .. url_encode(user_id), true)
  end

  -- ===== Admin: OIDC clients =====

  c.oidc_clients = {}

  function c.oidc_clients:list() return get(AUTH .. "/admin/oidc/clients", true) end
  function c.oidc_clients:get(id) return get(AUTH .. "/admin/oidc/clients/" .. url_encode(id), true) end
  function c.oidc_clients:create(body) return post(AUTH .. "/admin/oidc/clients", body, true) end
  function c.oidc_clients:update(id, body) return put(AUTH .. "/admin/oidc/clients/" .. url_encode(id), body, true) end
  function c.oidc_clients:delete(id) return del(AUTH .. "/admin/oidc/clients/" .. url_encode(id), true) end

  function c.oidc_clients:rotate_secret(id)
    return post(AUTH .. "/admin/oidc/clients/" .. url_encode(id) .. "/rotate-secret", nil, true)
  end

  -- ===== Admin: OIDC upstream providers =====

  c.oidc_upstream = {}

  function c.oidc_upstream:list() return get(AUTH .. "/admin/oidc/upstream", true) end
  function c.oidc_upstream:get(slug) return get(AUTH .. "/admin/oidc/upstream/" .. url_encode(slug), true) end
  function c.oidc_upstream:upsert(body) return post(AUTH .. "/admin/oidc/upstream", body, true) end
  function c.oidc_upstream:delete(slug) return del(AUTH .. "/admin/oidc/upstream/" .. url_encode(slug), true) end

  -- ===== Admin: JWKS =====

  c.jwks = {}
  function c.jwks:get() return get(AUTH .. "/admin/jwks", true) end

  -- ===== OIDC provider (public spec endpoints) =====

  c.oidc_provider = {}

  function c.oidc_provider:discovery()
    return get(SPEC .. "/.well-known/openid-configuration", false)
  end

  function c.oidc_provider:jwks() return get(SPEC .. "/.well-known/jwks.json", false) end

  --- Build the `/auth/authorize` URL for a redirect-based flow.
  --- @param params table {client_id, redirect_uri, response_type?, scope?, state?, code_challenge?, code_challenge_method?, nonce?, prompt?}
  function c.oidc_provider:authorize_url(params)
    params = params or {}
    local parts = {}
    for k, v in pairs(params) do
      if v ~= nil and v ~= "" then
        parts[#parts + 1] = url_encode(k) .. "=" .. url_encode(v)
      end
    end
    local q = (#parts > 0) and ("?" .. table.concat(parts, "&")) or ""
    return engine_url .. SPEC .. "/authorize" .. q
  end

  function c.oidc_provider:token(body) return post(SPEC .. "/token", body, false) end

  --- GET /auth/userinfo — pass `opts.access_token`; threaded through
  --- the Authorization: Bearer header.
  function c.oidc_provider:userinfo(uopts)
    uopts = uopts or {}
    local headers = build_headers(false)
    if uopts.access_token and uopts.access_token ~= "" then
      headers["Authorization"] = "Bearer " .. uopts.access_token
    end
    return decode(http.get(engine_url .. SPEC .. "/userinfo", { headers = headers }))
  end

  function c.oidc_provider:revoke(body) return post(SPEC .. "/revoke", body, false) end

  function c.oidc_provider:introspect(token)
    return post(SPEC .. "/introspect", { token = token }, false)
  end

  function c.oidc_provider:consent(body)
    return post(SPEC .. "/authorize/consent", body, false)
  end

  function c.oidc_provider:logout()
    local resp = http.get(engine_url .. SPEC .. "/logout", { headers = build_headers(false) })
    if resp.status == 302 or resp.status == 301 then
      return { redirect_url = resp.headers and resp.headers.location, status = resp.status }
    end
    return { status = resp.status, body = resp.body, headers = resp.headers }
  end

  -- ===== Admin: audit =====

  c.audit = {}

  function c.audit:list(qopts)
    qopts = qopts or {}
    local q = "?limit=" .. (qopts.limit or 50) .. "&offset=" .. (qopts.offset or 0)
    if qopts.actor then q = q .. "&actor=" .. url_encode(qopts.actor) end
    if qopts.action then q = q .. "&action=" .. url_encode(qopts.action) end
    if qopts.since then q = q .. "&since=" .. tostring(qopts.since) end
    local until_v = qopts["until"] or qopts.until_
    if until_v then q = q .. "&until=" .. tostring(until_v) end
    return get(AUTH .. "/admin/audit" .. q, true)
  end

  return c
end

return M
