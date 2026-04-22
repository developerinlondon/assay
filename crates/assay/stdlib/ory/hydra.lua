--- @module assay.ory.hydra
--- @description Ory Hydra OAuth2 and OpenID Connect — client CRUD, authorize URL builder, token exchange, login/consent/logout challenges, introspection, JWK endpoint.
--- @keywords hydra, ory, oauth2, oidc, openid, authentication, clients, tokens, login_challenge, consent_challenge, logout_challenge, jwk, authorize, introspect
--- @quickref hydra.client(opts) -> client | Create a Hydra client. opts: {public_url, admin_url}
--- @quickref c.clients:list(opts?) -> [{client_id, ...}] | List registered OAuth2 clients
--- @quickref c.clients:get(client_id) -> client | Get a registered OAuth2 client
--- @quickref c.clients:create(spec) -> client | Create/register an OAuth2 client
--- @quickref c.clients:update(client_id, spec) -> client | Upsert an OAuth2 client (PUT)
--- @quickref c.clients:delete(client_id) -> nil | Delete an OAuth2 client
--- @quickref c.oauth2:authorize_url(client_id, opts) -> url | Build the authorize URL for a browser redirect
--- @quickref c.oauth2:exchange_code(opts) -> {access_token, id_token, refresh_token} | Exchange auth code for tokens (authorization_code grant)
--- @quickref c.oauth2:refresh_token(client_id, client_secret, refresh_token) -> tokens | Refresh an access token
--- @quickref c.oauth2:introspect(token) -> {active, sub, scope, ...} | Token introspection
--- @quickref c.oauth2:revoke_token(client_id, client_secret, token) -> nil | Revoke a token
--- @quickref c.login:get(challenge) -> {challenge, subject, client, ...} | Fetch a pending login challenge
--- @quickref c.login:accept(challenge, subject, opts?) -> {redirect_to} | Accept a login challenge
--- @quickref c.login:reject(challenge, error) -> {redirect_to} | Reject a login challenge
--- @quickref c.consent:get(challenge) -> {challenge, subject, requested_scope, ...} | Fetch a pending consent challenge
--- @quickref c.consent:accept(challenge, opts) -> {redirect_to} | Accept a consent challenge (with claims)
--- @quickref c.consent:reject(challenge, error) -> {redirect_to} | Reject a consent challenge
--- @quickref c.logout:get(challenge) -> {request_url, rp_initiated, sid, subject, client} | Fetch a pending logout challenge
--- @quickref c.logout:accept(challenge) -> {redirect_to} | Accept a logout challenge (invalidates session)
--- @quickref c.logout:reject(challenge) -> nil | Reject a logout challenge (user stays signed in)
--- @quickref c.discovery:openid_config() -> {issuer, authorization_endpoint, ...} | Fetch OIDC discovery document
--- @quickref c.discovery:jwks() -> {keys} | Fetch JSON Web Key Set

local M = {}

local function urlencode(s)
  return (tostring(s):gsub("([^%w%-%.%_%~])", function(c)
    return string.format("%%%02X", string.byte(c))
  end))
end

-- Create a Hydra client. Pass opts.public_url for public API, opts.admin_url for admin API.
-- For token exchange/authorize use public_url; for client CRUD and login/consent challenges use admin_url.
function M.client(opts)
  opts = opts or {}
  local public_url = opts.public_url and opts.public_url:gsub("/+$", "") or nil
  local admin_url = opts.admin_url and opts.admin_url:gsub("/+$", "") or nil

  local function require_admin()
    if not admin_url then
      error("hydra: admin_url not configured")
    end
  end

  local function require_public()
    if not public_url then
      error("hydra: public_url not configured")
    end
  end

  local function admin_get(path_str)
    require_admin()
    local resp = http.get(admin_url .. path_str)
    if resp.status ~= 200 then
      error("hydra: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function admin_put(path_str, payload)
    require_admin()
    local resp = http.put(admin_url .. path_str, payload)
    if resp.status ~= 200 and resp.status ~= 201 then
      error("hydra: PUT " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function admin_post(path_str, payload)
    require_admin()
    local resp = http.post(admin_url .. path_str, payload)
    if resp.status ~= 200 and resp.status ~= 201 then
      error("hydra: POST " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  -- ========== Sub-objects ==========

  local c = {}

  -- ========== c.clients ==========

  c.clients = {}

  function c.clients:list(opts)
    opts = opts or {}
    local qs = ""
    if opts.page_size then qs = "?page_size=" .. opts.page_size end
    return admin_get("/admin/clients" .. qs)
  end

  function c.clients:get(client_id)
    return admin_get("/admin/clients/" .. client_id)
  end

  function c.clients:create(spec)
    return admin_post("/admin/clients", spec)
  end

  -- Upsert: creates or updates an OAuth2 client (idempotent). Recommended for GitOps workflows.
  function c.clients:update(client_id, spec)
    spec.client_id = client_id
    return admin_put("/admin/clients/" .. client_id, spec)
  end

  function c.clients:delete(client_id)
    require_admin()
    local resp = http.delete(admin_url .. "/admin/clients/" .. client_id)
    if resp.status ~= 204 and resp.status ~= 200 then
      error("hydra: DELETE client HTTP " .. resp.status .. ": " .. resp.body)
    end
  end

  -- ========== c.oauth2 ==========

  c.oauth2 = {}

  -- Build the authorize URL for a browser redirect.
  -- opts: { redirect_uri, scope, state, response_type (default "code"), extra (table of extra params) }
  function c.oauth2:authorize_url(client_id, opts)
    require_public()
    opts = opts or {}
    local params = {
      "client_id=" .. urlencode(client_id),
      "response_type=" .. urlencode(opts.response_type or "code"),
      "scope=" .. urlencode(opts.scope or "openid profile email"),
      "redirect_uri=" .. urlencode(opts.redirect_uri or ""),
    }
    if opts.state then
      params[#params + 1] = "state=" .. urlencode(opts.state)
    end
    if opts.nonce then
      params[#params + 1] = "nonce=" .. urlencode(opts.nonce)
    end
    if opts.extra then
      for k, v in pairs(opts.extra) do
        params[#params + 1] = k .. "=" .. urlencode(v)
      end
    end
    return public_url .. "/oauth2/auth?" .. table.concat(params, "&")
  end

  -- Exchange an authorization code for tokens (authorization_code grant).
  -- opts: { code, redirect_uri, client_id, client_secret }
  function c.oauth2:exchange_code(opts)
    require_public()
    local body = "grant_type=authorization_code"
      .. "&code=" .. urlencode(opts.code)
      .. "&redirect_uri=" .. urlencode(opts.redirect_uri)
      .. "&client_id=" .. urlencode(opts.client_id)
      .. "&client_secret=" .. urlencode(opts.client_secret)
    local resp = http.post(public_url .. "/oauth2/token", body, {
      headers = { ["Content-Type"] = "application/x-www-form-urlencoded" },
    })
    if resp.status ~= 200 then
      error("hydra: token exchange HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  -- Refresh an access token.
  function c.oauth2:refresh_token(client_id, client_secret, refresh_token)
    require_public()
    local body = "grant_type=refresh_token"
      .. "&refresh_token=" .. urlencode(refresh_token)
      .. "&client_id=" .. urlencode(client_id)
      .. "&client_secret=" .. urlencode(client_secret)
    local resp = http.post(public_url .. "/oauth2/token", body, {
      headers = { ["Content-Type"] = "application/x-www-form-urlencoded" },
    })
    if resp.status ~= 200 then
      error("hydra: refresh token HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  -- Introspect a token via admin API.
  function c.oauth2:introspect(token)
    require_admin()
    local resp = http.post(admin_url .. "/admin/oauth2/introspect",
      "token=" .. urlencode(token),
      { headers = { ["Content-Type"] = "application/x-www-form-urlencoded" } })
    if resp.status ~= 200 then
      error("hydra: introspect HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  -- Revoke a token.
  function c.oauth2:revoke_token(client_id, client_secret, token)
    require_public()
    local body = "token=" .. urlencode(token)
      .. "&client_id=" .. urlencode(client_id)
      .. "&client_secret=" .. urlencode(client_secret)
    local resp = http.post(public_url .. "/oauth2/revoke", body, {
      headers = { ["Content-Type"] = "application/x-www-form-urlencoded" },
    })
    if resp.status ~= 200 then
      error("hydra: revoke HTTP " .. resp.status .. ": " .. resp.body)
    end
  end

  -- ========== c.login ==========

  c.login = {}

  function c.login:get(challenge)
    return admin_get("/admin/oauth2/auth/requests/login?login_challenge=" .. urlencode(challenge))
  end

  -- Accept a login challenge. opts: { remember=bool, remember_for=seconds, acr=string, amr=[string], context=table }
  function c.login:accept(challenge, subject, opts)
    opts = opts or {}
    local payload = {
      subject = subject,
      remember = opts.remember,
      remember_for = opts.remember_for,
      acr = opts.acr,
      amr = opts.amr,
      context = opts.context,
    }
    return admin_put("/admin/oauth2/auth/requests/login/accept?login_challenge=" .. urlencode(challenge), payload)
  end

  function c.login:reject(challenge, err)
    return admin_put("/admin/oauth2/auth/requests/login/reject?login_challenge=" .. urlencode(challenge), err or { error = "access_denied" })
  end

  -- ========== c.consent ==========

  c.consent = {}

  function c.consent:get(challenge)
    return admin_get("/admin/oauth2/auth/requests/consent?consent_challenge=" .. urlencode(challenge))
  end

  -- Accept a consent challenge.
  -- opts: { grant_scope=[string], grant_access_token_audience=[string], remember=bool, remember_for=seconds,
  --         session={id_token=table, access_token=table} }
  function c.consent:accept(challenge, opts)
    opts = opts or {}
    local payload = {
      grant_scope = opts.grant_scope or { "openid", "profile", "email" },
      grant_access_token_audience = opts.grant_access_token_audience or {},
      remember = opts.remember,
      remember_for = opts.remember_for,
      session = opts.session,
    }
    return admin_put("/admin/oauth2/auth/requests/consent/accept?consent_challenge=" .. urlencode(challenge), payload)
  end

  function c.consent:reject(challenge, err)
    return admin_put("/admin/oauth2/auth/requests/consent/reject?consent_challenge=" .. urlencode(challenge), err or { error = "access_denied" })
  end

  -- ========== c.logout ==========

  c.logout = {}

  function c.logout:get(challenge)
    return admin_get("/admin/oauth2/auth/requests/logout?logout_challenge=" .. urlencode(challenge))
  end

  -- Accept a logout challenge. Hydra invalidates the user's Hydra session
  -- (and any backing Kratos session) and returns a redirect_to URL pointing
  -- at the post_logout_redirect_uri the client requested.
  function c.logout:accept(challenge)
    return admin_put("/admin/oauth2/auth/requests/logout/accept?logout_challenge=" .. urlencode(challenge), {})
  end

  -- Reject a logout challenge (for example, if the user clicks "stay
  -- signed in" on a logout confirmation page). Returns nothing meaningful;
  -- the handler should redirect the browser back to the application.
  function c.logout:reject(challenge)
    return admin_put("/admin/oauth2/auth/requests/logout/reject?logout_challenge=" .. urlencode(challenge), {})
  end

  -- ========== c.discovery ==========

  c.discovery = {}

  function c.discovery:openid_config()
    require_public()
    local resp = http.get(public_url .. "/.well-known/openid-configuration")
    if resp.status ~= 200 then
      error("hydra: well-known HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  function c.discovery:jwks()
    require_public()
    local resp = http.get(public_url .. "/.well-known/jwks.json")
    if resp.status ~= 200 then
      error("hydra: jwks HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  return c
end

return M
