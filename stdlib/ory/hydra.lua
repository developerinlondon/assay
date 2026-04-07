--- @module assay.ory.hydra
--- @description Ory Hydra OAuth2 and OpenID Connect — client CRUD, authorize URL builder, token exchange, login/consent/logout challenges, introspection, JWK endpoint.
--- @keywords hydra, ory, oauth2, oidc, openid, authentication, clients, tokens, login_challenge, consent_challenge, logout_challenge, jwk, authorize, introspect
--- @quickref hydra.client(opts) -> client | Create a Hydra client. opts: {public_url, admin_url}
--- @quickref c:list_clients(opts?) -> [{client_id, ...}] | List registered OAuth2 clients
--- @quickref c:get_client(client_id) -> client | Get a registered OAuth2 client
--- @quickref c:create_client(spec) -> client | Create/register an OAuth2 client
--- @quickref c:update_client(client_id, spec) -> client | Upsert an OAuth2 client (PUT)
--- @quickref c:delete_client(client_id) -> nil | Delete an OAuth2 client
--- @quickref c:build_authorize_url(client_id, opts) -> url | Build the authorize URL for a browser redirect
--- @quickref c:exchange_code(opts) -> {access_token, id_token, refresh_token} | Exchange auth code for tokens (authorization_code grant)
--- @quickref c:refresh_token(client_id, client_secret, refresh_token) -> tokens | Refresh an access token
--- @quickref c:introspect(token) -> {active, sub, scope, ...} | Token introspection
--- @quickref c:revoke_token(client_id, client_secret, token) -> nil | Revoke a token
--- @quickref c:get_login_request(challenge) -> {challenge, subject, client, ...} | Fetch a pending login challenge
--- @quickref c:accept_login(challenge, subject, opts?) -> {redirect_to} | Accept a login challenge
--- @quickref c:reject_login(challenge, error) -> {redirect_to} | Reject a login challenge
--- @quickref c:get_consent_request(challenge) -> {challenge, subject, requested_scope, ...} | Fetch a pending consent challenge
--- @quickref c:accept_consent(challenge, opts) -> {redirect_to} | Accept a consent challenge (with claims)
--- @quickref c:reject_consent(challenge, error) -> {redirect_to} | Reject a consent challenge
--- @quickref c:get_logout_request(challenge) -> {request_url, rp_initiated, sid, subject, client} | Fetch a pending logout challenge
--- @quickref c:accept_logout(challenge) -> {redirect_to} | Accept a logout challenge (invalidates session)
--- @quickref c:reject_logout(challenge) -> nil | Reject a logout challenge (user stays signed in)
--- @quickref c:well_known() -> {issuer, authorization_endpoint, ...} | Fetch OIDC discovery document
--- @quickref c:jwks() -> {keys} | Fetch JSON Web Key Set

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
  local c = {
    public_url = opts.public_url and opts.public_url:gsub("/+$", "") or nil,
    admin_url = opts.admin_url and opts.admin_url:gsub("/+$", "") or nil,
  }

  local function require_admin(self)
    if not self.admin_url then
      error("hydra: admin_url not configured")
    end
  end

  local function require_public(self)
    if not self.public_url then
      error("hydra: public_url not configured")
    end
  end

  local function admin_get(self, path_str)
    require_admin(self)
    local resp = http.get(self.admin_url .. path_str)
    if resp.status ~= 200 then
      error("hydra: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function admin_put(self, path_str, payload)
    require_admin(self)
    local resp = http.put(self.admin_url .. path_str, payload)
    if resp.status ~= 200 and resp.status ~= 201 then
      error("hydra: PUT " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function admin_post(self, path_str, payload)
    require_admin(self)
    local resp = http.post(self.admin_url .. path_str, payload)
    if resp.status ~= 200 and resp.status ~= 201 then
      error("hydra: POST " .. path_str .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  -- ========== OAuth2 Client CRUD ==========

  function c:list_clients(opts)
    opts = opts or {}
    local qs = ""
    if opts.page_size then qs = "?page_size=" .. opts.page_size end
    return admin_get(self, "/admin/clients" .. qs)
  end

  function c:get_client(client_id)
    return admin_get(self, "/admin/clients/" .. client_id)
  end

  function c:create_client(spec)
    return admin_post(self, "/admin/clients", spec)
  end

  -- Upsert: creates or updates an OAuth2 client (idempotent). Recommended for GitOps workflows.
  function c:update_client(client_id, spec)
    spec.client_id = client_id
    return admin_put(self, "/admin/clients/" .. client_id, spec)
  end

  function c:delete_client(client_id)
    require_admin(self)
    local resp = http.delete(self.admin_url .. "/admin/clients/" .. client_id)
    if resp.status ~= 204 and resp.status ~= 200 then
      error("hydra: DELETE client HTTP " .. resp.status .. ": " .. resp.body)
    end
  end

  -- ========== OAuth2 Flow Helpers ==========

  -- Build the authorize URL for a browser redirect.
  -- opts: { redirect_uri, scope, state, response_type (default "code"), extra (table of extra params) }
  function c:build_authorize_url(client_id, opts)
    require_public(self)
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
    return self.public_url .. "/oauth2/auth?" .. table.concat(params, "&")
  end

  -- Exchange an authorization code for tokens (authorization_code grant).
  -- opts: { code, redirect_uri, client_id, client_secret }
  function c:exchange_code(opts)
    require_public(self)
    local body = "grant_type=authorization_code"
      .. "&code=" .. urlencode(opts.code)
      .. "&redirect_uri=" .. urlencode(opts.redirect_uri)
      .. "&client_id=" .. urlencode(opts.client_id)
      .. "&client_secret=" .. urlencode(opts.client_secret)
    local resp = http.post(self.public_url .. "/oauth2/token", body, {
      headers = { ["Content-Type"] = "application/x-www-form-urlencoded" },
    })
    if resp.status ~= 200 then
      error("hydra: token exchange HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  -- Refresh an access token.
  function c:refresh_token(client_id, client_secret, refresh_token)
    require_public(self)
    local body = "grant_type=refresh_token"
      .. "&refresh_token=" .. urlencode(refresh_token)
      .. "&client_id=" .. urlencode(client_id)
      .. "&client_secret=" .. urlencode(client_secret)
    local resp = http.post(self.public_url .. "/oauth2/token", body, {
      headers = { ["Content-Type"] = "application/x-www-form-urlencoded" },
    })
    if resp.status ~= 200 then
      error("hydra: refresh token HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  -- Introspect a token via admin API.
  function c:introspect(token)
    require_admin(self)
    local resp = http.post(self.admin_url .. "/admin/oauth2/introspect",
      "token=" .. urlencode(token),
      { headers = { ["Content-Type"] = "application/x-www-form-urlencoded" } })
    if resp.status ~= 200 then
      error("hydra: introspect HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  -- Revoke a token.
  function c:revoke_token(client_id, client_secret, token)
    require_public(self)
    local body = "token=" .. urlencode(token)
      .. "&client_id=" .. urlencode(client_id)
      .. "&client_secret=" .. urlencode(client_secret)
    local resp = http.post(self.public_url .. "/oauth2/revoke", body, {
      headers = { ["Content-Type"] = "application/x-www-form-urlencoded" },
    })
    if resp.status ~= 200 then
      error("hydra: revoke HTTP " .. resp.status .. ": " .. resp.body)
    end
  end

  -- ========== Login/Consent Challenges ==========

  function c:get_login_request(challenge)
    return admin_get(self, "/admin/oauth2/auth/requests/login?login_challenge=" .. urlencode(challenge))
  end

  -- Accept a login challenge. opts: { remember=bool, remember_for=seconds, acr=string, amr=[string], context=table }
  function c:accept_login(challenge, subject, opts)
    opts = opts or {}
    local payload = {
      subject = subject,
      remember = opts.remember,
      remember_for = opts.remember_for,
      acr = opts.acr,
      amr = opts.amr,
      context = opts.context,
    }
    return admin_put(self, "/admin/oauth2/auth/requests/login/accept?login_challenge=" .. urlencode(challenge), payload)
  end

  function c:reject_login(challenge, err)
    return admin_put(self, "/admin/oauth2/auth/requests/login/reject?login_challenge=" .. urlencode(challenge), err or { error = "access_denied" })
  end

  function c:get_consent_request(challenge)
    return admin_get(self, "/admin/oauth2/auth/requests/consent?consent_challenge=" .. urlencode(challenge))
  end

  -- Accept a consent challenge.
  -- opts: { grant_scope=[string], grant_access_token_audience=[string], remember=bool, remember_for=seconds,
  --         session={id_token=table, access_token=table} }
  function c:accept_consent(challenge, opts)
    opts = opts or {}
    local payload = {
      grant_scope = opts.grant_scope or { "openid", "profile", "email" },
      grant_access_token_audience = opts.grant_access_token_audience or {},
      remember = opts.remember,
      remember_for = opts.remember_for,
      session = opts.session,
    }
    return admin_put(self, "/admin/oauth2/auth/requests/consent/accept?consent_challenge=" .. urlencode(challenge), payload)
  end

  function c:reject_consent(challenge, err)
    return admin_put(self, "/admin/oauth2/auth/requests/consent/reject?consent_challenge=" .. urlencode(challenge), err or { error = "access_denied" })
  end

  -- ========== Logout Challenges ==========
  -- When an OAuth2 client triggers an OIDC logout (typically by hitting
  -- /oauth2/sessions/logout with id_token_hint and post_logout_redirect_uri),
  -- Hydra creates a logout request and redirects the user to the configured
  -- urls.logout endpoint. The logout handler accepts or rejects the request
  -- via the admin API and gets back a redirect_to URL to send the browser to.

  function c:get_logout_request(challenge)
    return admin_get(self, "/admin/oauth2/auth/requests/logout?logout_challenge=" .. urlencode(challenge))
  end

  -- Accept a logout challenge. Hydra invalidates the user's Hydra session
  -- (and any backing Kratos session) and returns a redirect_to URL pointing
  -- at the post_logout_redirect_uri the client requested.
  function c:accept_logout(challenge)
    return admin_put(self, "/admin/oauth2/auth/requests/logout/accept?logout_challenge=" .. urlencode(challenge), {})
  end

  -- Reject a logout challenge (for example, if the user clicks "stay
  -- signed in" on a logout confirmation page). Returns nothing meaningful;
  -- the handler should redirect the browser back to the application.
  function c:reject_logout(challenge)
    return admin_put(self, "/admin/oauth2/auth/requests/logout/reject?logout_challenge=" .. urlencode(challenge), {})
  end

  -- ========== OIDC Discovery ==========

  function c:well_known()
    require_public(self)
    local resp = http.get(self.public_url .. "/.well-known/openid-configuration")
    if resp.status ~= 200 then
      error("hydra: well-known HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  function c:jwks()
    require_public(self)
    local resp = http.get(self.public_url .. "/.well-known/jwks.json")
    if resp.status ~= 200 then
      error("hydra: jwks HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  return c
end

return M
