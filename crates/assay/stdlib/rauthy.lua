--- @module assay.rauthy
--- @description Rauthy IdP admin API client. OAuth2 client reconciliation, secret rotation, discovery, health.
--- @keywords rauthy, oidc, oauth2, openid, idp, identity, sso, clients, client-credentials, pkce, jwks, discovery, authorization-server, rotation, reconcile
--- @quickref c.sys:health() -> bool | Check Rauthy is reachable + healthy
--- @quickref c.sys:wait_healthy(timeout_secs?) -> true | Block until /health returns 2xx (default 120 s)
--- @quickref c.discovery:config() -> {issuer, endpoints…} | Get OIDC discovery configuration
--- @quickref c.discovery:jwks() -> {keys} | Get JSON Web Key Set
--- @quickref c.clients:list() -> [{id, name, …}] | List all OAuth2 clients
--- @quickref c.clients:get(id) -> client|nil | Read a client; nil on 404
--- @quickref c.clients:create(payload) -> nil | POST(subset)+PUT(full) — used by reconcile, rarely called directly
--- @quickref c.clients:put(id, payload) -> nil | In-place update without rotating client_secret
--- @quickref c.clients:delete(id) -> nil | Delete a client (404 is treated as success)
--- @quickref c.clients:rebuild(payload) -> nil | DELETE + POST + PUT; rotates secret for confidential clients
--- @quickref c.clients:rotate_secret(id) -> string | Regenerate and return the client_secret
--- @quickref c.clients:reconcile(payload) -> {action, secret?, drift_on?, reason?} | Idempotent reconcile; only rebuilds on `challenges` drift or 404
--- @quickref M.client_presets.openbao({host, id?, name?}) -> payload | OpenBao OAuth2 client payload (RS256 + S256 PKCE; required by Vault go-oidc)
--- @quickref M.client_presets.argocd({host, id?, name?}) -> payload | ArgoCD OAuth2 client payload (PKCE-public, EdDSA, CLI device-login redirect)
--- @quickref M.client_presets.outline({host, id?, name?}) -> payload | Outline wiki OAuth2 client payload (confidential, RS256, S256 PKCE)

local M = {}

-- ===== Private helpers =====

local function array_eq(a, b)
  if not a and not b then return true end
  if not a or not b then return false end
  if #a ~= #b then return false end
  local count = {}
  for _, v in ipairs(a) do count[v] = (count[v] or 0) + 1 end
  for _, v in ipairs(b) do
    if not count[v] or count[v] == 0 then return false end
    count[v] = count[v] - 1
  end
  return true
end

local function field_eq(want, got)
  if want == nil and got == nil then return true end
  if type(want) ~= type(got) then return false end
  if type(want) == "table" then return array_eq(want, got) end
  return want == got
end

local function drift_field(payload, got)
  for k, v in pairs(payload) do
    if not field_eq(v, got[k]) then return k end
  end
  -- Reverse direction: a field present in `got` but absent from `payload`
  -- means the caller wants it removed. Without this, a preset shipped today
  -- with `challenges = {"S256"}` and later overridden to omit `challenges`
  -- would silently noop because the forward loop never visits the missing
  -- key. Rauthy's PUT is a full-replacement, so a follow-up put() correctly
  -- clears the field.
  for k, v in pairs(got) do
    if payload[k] == nil and v ~= nil then return k end
  end
  return nil
end

-- ===== Client constructor =====

function M.client(url, api_key)
  local base_url = url:gsub("/+$", "")

  local function headers()
    return {
      ["Authorization"] = "API-Key " .. api_key,
      ["Content-Type"] = "application/json",
    }
  end

  local function ok(resp) return resp.status >= 200 and resp.status < 300 end

  local function api_call(method, path, body)
    local u = base_url .. path
    if method == "GET" then return http.get(u, { headers = headers() })
    elseif method == "DELETE" then return http.delete(u, { headers = headers() })
    elseif method == "POST" then return http.post(u, body, { headers = headers() })
    elseif method == "PUT" then return http.put(u, body, { headers = headers() })
    end
    error("rauthy: unsupported method " .. method)
  end

  local function require_ok(resp, method, path)
    if not ok(resp) then
      error(string.format("rauthy %s %s: HTTP %d: %s",
        method, path, resp.status, resp.body or ""))
    end
    return resp
  end

  local c = {}

  -- ===== System / Health =====

  c.sys = {}

  function c.sys:health()
    local resp = api_call("GET", "/health")
    return ok(resp)
  end

  function c.sys:wait_healthy(timeout_secs)
    timeout_secs = timeout_secs or 120
    local interval = 2
    local elapsed = 0
    while elapsed < timeout_secs do
      local resp = http.get(base_url .. "/health", { headers = headers(), timeout = 3 })
      if ok(resp) then return true end
      sleep(interval)
      elapsed = elapsed + interval
    end
    error("rauthy: never became reachable at " .. base_url .. "/health")
  end

  -- ===== Discovery =====

  c.discovery = {}

  function c.discovery:config()
    local resp = require_ok(
      http.get(base_url .. "/.well-known/openid-configuration", { headers = {} }),
      "GET", "/.well-known/openid-configuration"
    )
    return json.parse(resp.body)
  end

  function c.discovery:jwks()
    local cfg = c.discovery:config()
    if not cfg.jwks_uri then
      error("rauthy.discovery: response missing jwks_uri")
    end
    local resp = require_ok(http.get(cfg.jwks_uri, { headers = {} }), "GET", cfg.jwks_uri)
    return json.parse(resp.body)
  end

  -- ===== Clients =====

  c.clients = {}

  function c.clients:list()
    local resp = require_ok(api_call("GET", "/clients"), "GET", "/clients")
    return json.parse(resp.body)
  end

  function c.clients:get(id)
    local resp = api_call("GET", "/clients/" .. id)
    if resp.status == 404 then return nil end
    require_ok(resp, "GET", "/clients/" .. id)
    return json.parse(resp.body)
  end

  -- POST a NewClientRequest subset, then PUT the full UpdateClientRequest.
  -- Two-call create matches Rauthy's typed API surface (NewClientRequest is
  -- smaller than UpdateClientRequest).
  function c.clients:create(payload)
    local subset = {
      id = payload.id,
      name = payload.name,
      confidential = payload.confidential,
      redirect_uris = payload.redirect_uris,
      post_logout_redirect_uris = payload.post_logout_redirect_uris,
    }
    require_ok(api_call("POST", "/clients", subset), "POST", "/clients")
    require_ok(api_call("PUT", "/clients/" .. payload.id, payload), "PUT", "/clients/" .. payload.id)
  end

  function c.clients:put(id, payload)
    require_ok(api_call("PUT", "/clients/" .. id, payload), "PUT", "/clients/" .. id)
  end

  function c.clients:delete(id)
    local resp = api_call("DELETE", "/clients/" .. id)
    if not (ok(resp) or resp.status == 404) then
      error(string.format("rauthy DELETE /clients/%s: HTTP %d: %s",
        id, resp.status, resp.body or ""))
    end
  end

  -- DELETE + POST(subset) + PUT(full). Sidesteps a Rauthy 0.35 cache bug
  -- where `challenges` set via PUT-after-subset-POST reads back correctly
  -- via GET but stays invisible to the OIDC handler at login time
  -- (`self.challenge` cached as None). For confidential clients this
  -- rotates the secret as a side effect.
  function c.clients:rebuild(payload)
    self:delete(payload.id)
    self:create(payload)
  end

  function c.clients:rotate_secret(id)
    local resp = require_ok(
      api_call("POST", "/clients/" .. id .. "/secret"),
      "POST", "/clients/" .. id .. "/secret"
    )
    local secret = json.parse(resp.body).secret
    if not secret or secret == "" then
      error("rauthy: empty secret returned for " .. id)
    end
    return secret
  end

  -- Idempotent reconciler. Decision tree:
  --   • 404                                  → create + rotate (if confidential)
  --   • challenges declared but not stored   → rebuild + rotate (Rauthy cache workaround)
  --   • drift on any other field             → put-only (NO rotation)
  --   • no drift                             → noop
  --
  -- Returns:
  --   { action = "create"|"rebuild"|"put"|"noop",
  --     secret = string?,    -- present iff a rotation happened
  --     drift_on = string?,  -- present iff action == "put"
  --     reason = string? }   -- present iff action == "rebuild"
  function c.clients:reconcile(payload)
    local id = payload.id or error("rauthy.clients:reconcile: payload.id required")

    local got = self:get(id)

    if not got then
      self:create(payload)
      local secret = payload.confidential and self:rotate_secret(id) or nil
      return { action = "create", secret = secret }
    end

    if payload.challenges and #payload.challenges > 0
        and not (got.challenges and #got.challenges > 0) then
      self:rebuild(payload)
      local secret = payload.confidential and self:rotate_secret(id) or nil
      return { action = "rebuild", secret = secret, reason = "challenges-drift" }
    end

    local diff = drift_field(payload, got)
    if diff then
      self:put(id, payload)
      return { action = "put", drift_on = diff }
    end

    return { action = "noop" }
  end

  return c
end

-- ===== Client presets =====
--
-- Ready-to-use OAuth2 client payloads for common consumers. Each preset
-- bakes in the quirks of that consumer's OIDC verifier so a Rauthy-fronted
-- deployment doesn't have to rediscover them via failure logs.

M.client_presets = {}

-- OpenBao / Vault. Confidential client (shared client_secret).
--   * `id_token_alg = RS256` — upstream go-oidc rejects EdDSA with
--     `unsupported signing algorithm`.
--   * `challenges = [S256]` — OpenBao sends `code_challenge` in the
--     auth-code request even for confidential clients (OAuth 2.1 default);
--     Rauthy rejects PKCE flows whose client doesn't declare challenges.
--   * Redirect URIs cover both the UI callback and the
--     `bao login -method=oidc` device-login loopback.
function M.client_presets.openbao(opts)
  if not opts or not opts.host then
    error("rauthy.client_presets.openbao: opts.host required")
  end
  local host = opts.host
  return {
    id = opts.id or "openbao",
    name = opts.name or "OpenBao",
    confidential = true,
    enabled = true,
    redirect_uris = {
      "https://" .. host .. "/ui/vault/auth/oidc/oidc/callback",
      "http://localhost:8250/oidc/callback",
    },
    post_logout_redirect_uris = { "https://" .. host },
    allowed_origins = { "https://" .. host },
    flows_enabled = { "authorization_code", "refresh_token" },
    access_token_alg = "RS256",
    id_token_alg = "RS256",
    auth_code_lifetime = 60,
    access_token_lifetime = 1800,
    scopes = { "openid", "email", "profile", "groups" },
    default_scopes = { "openid", "email", "profile", "groups" },
    challenges = { "S256" },
    force_mfa = false,
  }
end

-- ArgoCD. PKCE-public client (no shared secret).
--   * EdDSA accepted by ArgoCD's OIDC stack.
--   * Loopback redirect covers `argocd login --sso` device-login.
function M.client_presets.argocd(opts)
  if not opts or not opts.host then
    error("rauthy.client_presets.argocd: opts.host required")
  end
  local host = opts.host
  return {
    id = opts.id or "argocd",
    name = opts.name or "ArgoCD",
    confidential = false,
    enabled = true,
    redirect_uris = {
      "https://" .. host .. "/auth/callback",
      "http://localhost:8085/auth/callback",
    },
    post_logout_redirect_uris = { "https://" .. host },
    allowed_origins = { "https://" .. host },
    flows_enabled = { "authorization_code", "refresh_token" },
    access_token_alg = "EdDSA",
    id_token_alg = "EdDSA",
    auth_code_lifetime = 60,
    access_token_lifetime = 1800,
    scopes = { "openid", "email", "profile", "groups" },
    default_scopes = { "openid", "email", "profile", "groups" },
    challenges = { "S256" },
    force_mfa = false,
  }
end

-- Outline wiki. Confidential client (shared client_secret).
--   * `id_token_alg = RS256` — Outline uses jose for JWT validation; RS256 is the
--     widely-supported default. EdDSA is not in jose's default key-resolution path.
--   * No `challenges` field — Outline 1.8.x sends authorize requests without
--     `code_challenge`, and Rauthy 400's "code_challenge missing" on any client
--     that declares challenges. Confidential client + client_secret carries the
--     authn weight; PKCE is redundant here.
--   * Redirect URI is `/auth/oidc.callback` (Outline's hardcoded OIDC callback path).
function M.client_presets.outline(opts)
  if not opts or not opts.host then
    error("rauthy.client_presets.outline: opts.host required")
  end
  local host = opts.host
  return {
    id = opts.id or "outline",
    name = opts.name or "Outline",
    confidential = true,
    enabled = true,
    redirect_uris = {
      "https://" .. host .. "/auth/oidc.callback",
    },
    post_logout_redirect_uris = { "https://" .. host },
    allowed_origins = { "https://" .. host },
    flows_enabled = { "authorization_code", "refresh_token" },
    access_token_alg = "RS256",
    id_token_alg = "RS256",
    auth_code_lifetime = 60,
    access_token_lifetime = 1800,
    scopes = { "openid", "email", "profile" },
    default_scopes = { "openid", "email", "profile" },
    force_mfa = false,
  }
end

-- Seafile (CE 11.0+, including 13.x). Confidential client.
--   * Redirect URI is `/oauth/callback/` — Seafile CE uses its own
--     OAuth handler (NOT python-social-auth's `openidconnect`), and the
--     callback path is whatever `OAUTH_REDIRECT_URL` in seahub_settings.py
--     points at. `/oauth/callback/` is the documented convention.
--   * `groups` scope is requested for general group sync.
--   * Note: Seafile CE has NO supported OIDC admin-claim mapping.
--     First admin must be promoted manually after the first OIDC login
--     (or via a custom hook in conf/seahub_custom_functions/__init__.py).
--   * No `challenges` — Seafile's OAuth handler doesn't initiate PKCE;
--     the client_secret carries authn.
function M.client_presets.seafile(opts)
  if not opts or not opts.host then
    error("rauthy.client_presets.seafile: opts.host required")
  end
  local host = opts.host
  return {
    id = opts.id or "seafile",
    name = opts.name or "Seafile",
    confidential = true,
    enabled = true,
    redirect_uris = {
      "https://" .. host .. "/oauth/callback/",
    },
    post_logout_redirect_uris = { "https://" .. host },
    allowed_origins = { "https://" .. host },
    flows_enabled = { "authorization_code", "refresh_token" },
    access_token_alg = "RS256",
    id_token_alg = "RS256",
    auth_code_lifetime = 60,
    access_token_lifetime = 1800,
    scopes = { "openid", "email", "profile", "groups" },
    default_scopes = { "openid", "email", "profile", "groups" },
    force_mfa = false,
  }
end

-- Paperless-ngx (2.0+). Confidential client, OIDC via django-allauth.
--   * Redirect URI is `/accounts/oidc/rauthy/login/callback/` —
--     django-allauth path pattern is
--     `/accounts/oidc/<provider_id>/login/callback/` and the deployer
--     pins `provider_id = "rauthy"` in PAPERLESS_SOCIALACCOUNT_PROVIDERS.
--   * `groups` scope passed through. Paperless syncs OIDC groups to its
--     own group table when PAPERLESS_SOCIAL_ACCOUNT_SYNC_GROUPS=true.
--     Note: superuser (admin) flag is NOT set from any OIDC claim —
--     must be granted in Django admin manually.
--   * `challenges = [S256]` — Paperless's docs explicitly recommend
--     PKCE (`OAUTH_PKCE_ENABLED: true` in the provider settings), so
--     Rauthy must accept S256.
function M.client_presets.paperless(opts)
  if not opts or not opts.host then
    error("rauthy.client_presets.paperless: opts.host required")
  end
  local host = opts.host
  return {
    id = opts.id or "paperless",
    name = opts.name or "Paperless-ngx",
    confidential = true,
    enabled = true,
    redirect_uris = {
      "https://" .. host .. "/accounts/oidc/rauthy/login/callback/",
    },
    post_logout_redirect_uris = { "https://" .. host },
    allowed_origins = { "https://" .. host },
    flows_enabled = { "authorization_code", "refresh_token" },
    access_token_alg = "RS256",
    id_token_alg = "RS256",
    auth_code_lifetime = 60,
    access_token_lifetime = 1800,
    scopes = { "openid", "email", "profile", "groups" },
    default_scopes = { "openid", "email", "profile", "groups" },
    challenges = { "S256" },
    force_mfa = false,
  }
end

-- Immich (1.91+) web client. Confidential.
--   * Redirect URIs cover Immich's two OIDC entry points:
--       /auth/login       — initial login redirect handler
--       /user-settings    — account-link flow from the settings page
--   * No `challenges` — Immich's OIDC flow does NOT initiate PKCE in
--     the official web release; the Authelia integration guide
--     explicitly sets `require_pkce: false` for Immich. Confidential
--     client + client_secret carries authn.
--   * `groups` scope is requested for general group sync, but note:
--     Immich's admin role is read from a SEPARATE claim called
--     `immich_role` (configured in the Immich admin UI under
--     "Role Claim"). To map a Rauthy "admin" group -> Immich admin,
--     configure Rauthy to emit `immich_role: "admin"` as a custom
--     claim for that group; the `groups` scope alone won't promote.
function M.client_presets.immich(opts)
  if not opts or not opts.host then
    error("rauthy.client_presets.immich: opts.host required")
  end
  local host = opts.host
  return {
    id = opts.id or "immich",
    name = opts.name or "Immich",
    confidential = true,
    enabled = true,
    redirect_uris = {
      "https://" .. host .. "/auth/login",
      "https://" .. host .. "/user-settings",
    },
    post_logout_redirect_uris = { "https://" .. host },
    allowed_origins = { "https://" .. host },
    flows_enabled = { "authorization_code", "refresh_token" },
    access_token_alg = "RS256",
    id_token_alg = "RS256",
    auth_code_lifetime = 60,
    access_token_lifetime = 1800,
    scopes = { "openid", "email", "profile", "groups" },
    default_scopes = { "openid", "email", "profile", "groups" },
    force_mfa = false,
  }
end

-- Immich mobile (Flutter app). PUBLIC client + mandatory PKCE.
--   * `confidential = false` — a mobile binary can't safely hold a
--     client_secret; PKCE replaces secret-based authn.
--   * Redirect URI is the `app.immich:///oauth-callback` deep link
--     (three slashes — the empty host segment matches the registered
--     scheme on Immich's Flutter app).
--   * `challenges = [S256]` mandatory for public clients.
function M.client_presets.immich_mobile(opts)
  if not opts or not opts.host then
    error("rauthy.client_presets.immich_mobile: opts.host required")
  end
  local host = opts.host
  return {
    id = opts.id or "immich-mobile",
    name = opts.name or "Immich (mobile)",
    confidential = false,
    enabled = true,
    redirect_uris = {
      "app.immich:///oauth-callback",
    },
    post_logout_redirect_uris = { "https://" .. host },
    allowed_origins = { "https://" .. host },
    flows_enabled = { "authorization_code", "refresh_token" },
    access_token_alg = "RS256",
    id_token_alg = "RS256",
    auth_code_lifetime = 60,
    access_token_lifetime = 1800,
    scopes = { "openid", "email", "profile", "groups" },
    default_scopes = { "openid", "email", "profile", "groups" },
    challenges = { "S256" },
    force_mfa = false,
  }
end

return M
