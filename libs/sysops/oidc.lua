--! sysops.oidc - OIDC Authorization Code + PKCE client.
--!
--! Discovery, authorize-URL builder, code exchange, refresh, and
--! id_token verification. All HTTP and crypto come from assay-lua's
--! Rust-bound globals (http, crypto, base64, json).
--!
--! Usage:
--!   local oidc = require("sysops.oidc")
--!   local client = oidc.new({
--!     issuer        = "https://gondor.fcar.ai/auth",
--!     client_id     = "sysops",
--!     client_secret = "...",         -- optional for public clients
--!     redirect_uri  = "https://gondor.fcar.ai/auth/callback",
--!     scopes        = { "openid", "profile", "email" },  -- optional
--!   })
--!   local state = crypto.random(32)
--!   local verifier = crypto.random(64)
--!   local authorize_url = client:authorize_url(state, verifier)
--!   -- ... redirect browser to authorize_url ...
--!   -- ... browser comes back to redirect_uri with ?code=...&state=...
--!   local tokens, err = client:exchange_code(code, verifier)
--!   local claims, verr = client:verify_id_token(tokens.id_token)
--!
--! All discovery-dependent calls cache the discovery document on the
--! client object after the first hit. Same for the JWKS used by
--! verify_id_token. Callers don't need to reinit.

local url = require("assay.url")
local M = {}

----------------------------------------------------------------------
-- Helpers
----------------------------------------------------------------------

local function rstrip_slash(s)
  if s and #s > 1 and s:sub(-1) == "/" then return s:sub(1, -2) end
  return s
end

local function hex_to_bytes(hex)
  return (hex:gsub("..", function(b) return string.char(tonumber(b, 16)) end))
end

-- URL-safe base64 (RFC 7636 PKCE / RFC 7515 JWT compact form). Implemented
-- in pure lua because the host's base64.encode binding enforces UTF-8 on
-- its String input — raw SHA-256 bytes don't satisfy that.
local B64U_CHARS = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_"
local function b64url(bytes)
  local out = {}
  local i = 1
  local len = #bytes
  while i <= len do
    local b1 = string.byte(bytes, i)
    local b2 = string.byte(bytes, i + 1) or 0
    local b3 = string.byte(bytes, i + 2) or 0
    local n = b1 * 65536 + b2 * 256 + b3
    out[#out + 1] = B64U_CHARS:sub(((n // 262144) % 64) + 1, ((n // 262144) % 64) + 1)
    out[#out + 1] = B64U_CHARS:sub(((n // 4096) % 64) + 1, ((n // 4096) % 64) + 1)
    if i + 1 <= len then
      out[#out + 1] = B64U_CHARS:sub(((n // 64) % 64) + 1, ((n // 64) % 64) + 1)
    end
    if i + 2 <= len then
      out[#out + 1] = B64U_CHARS:sub((n % 64) + 1, (n % 64) + 1)
    end
    i = i + 3
  end
  return table.concat(out)
end

-- assay-lua overrides the builtin `assert` global with a table
-- (assert.eq, assert.not_nil, ...). Provide a local "unwrap or error"
-- helper for the (value, err) → value pattern used throughout.
local function must(v, err)
  if not v then
    error(err or "assertion failed", 2)
  end
  return v
end

local function ok2xx(status)
  return type(status) == "number" and status >= 200 and status < 300
end

-- Wrap http.get/post errors into { status, body } pairs the caller can
-- inspect. Returns (response, nil) on 2xx, (nil, err) otherwise.
local function http_result(resp)
  if type(resp) ~= "table" then
    return nil, { status = 0, body = "no response from http" }
  end
  if not ok2xx(resp.status) then
    return nil, { status = resp.status, body = resp.body }
  end
  return resp, nil
end

----------------------------------------------------------------------
-- Module
----------------------------------------------------------------------

--- Create an OIDC client bound to one issuer.
--- @param opts table issuer, client_id required; client_secret, redirect_uri,
---             scopes optional. Tests can inject opts.http to override the
---             global http table.
function M.new(opts)
  if not (opts and type(opts.issuer) == "string") then
    error("oidc.new: opts.issuer required", 2)
  end
  if type(opts.client_id) ~= "string" then
    error("oidc.new: opts.client_id required", 2)
  end

  local self = {}
  self.issuer        = rstrip_slash(opts.issuer)
  self.client_id     = opts.client_id
  self.client_secret = opts.client_secret
  self.redirect_uri  = opts.redirect_uri
  self.scopes        = opts.scopes or { "openid", "profile", "email" }
  self._http         = opts.http or http
  self._discovery    = nil
  self._jwks         = nil

  --- Fetch and cache the OIDC discovery document.
  function self:discover()
    if self._discovery then return self._discovery end
    local discovery_url = self.issuer .. "/.well-known/openid-configuration"
    local resp, err = http_result(self._http.get(discovery_url))
    if err then return nil, err end
    local doc = type(resp.body) == "string" and json.parse(resp.body) or resp.body
    if type(doc) ~= "table" or not doc.authorization_endpoint then
      return nil, { status = resp.status, body = "discovery missing endpoints" }
    end
    self._discovery = doc
    return doc
  end

  --- Build the authorize URL (PKCE S256). Caller supplies state + verifier
  --- (random opaque strings; PKCE spec says verifier must be 43-128 chars
  --- from [A-Z][a-z][0-9]-._~). crypto.random(64) satisfies that already.
  function self:authorize_url(state, code_verifier, opt_overrides)
    local endpoints = must(self:discover())
    opt_overrides = opt_overrides or {}
    local code_challenge = b64url(hex_to_bytes(crypto.hash(code_verifier, "sha256")))
    local params = {
      response_type         = "code",
      client_id             = self.client_id,
      redirect_uri          = opt_overrides.redirect_uri or self.redirect_uri,
      scope                 = table.concat(opt_overrides.scopes or self.scopes, " "),
      state                 = state,
      code_challenge        = code_challenge,
      code_challenge_method = "S256",
    }
    return endpoints.authorization_endpoint .. "?" .. url.encode_form(params)
  end

  --- Exchange an authorization code for tokens. Returns the parsed JSON
  --- body on success (id_token, access_token, refresh_token, expires_in,
  --- token_type) or (nil, err) where err = { status, body }.
  function self:exchange_code(code, code_verifier, redirect_uri_override)
    local endpoints = must(self:discover())
    local body = url.encode_form({
      grant_type    = "authorization_code",
      code          = code,
      redirect_uri  = redirect_uri_override or self.redirect_uri,
      client_id     = self.client_id,
      client_secret = self.client_secret,
      code_verifier = code_verifier,
    })
    local resp = self._http.post(endpoints.token_endpoint, body, {
      headers = { ["Content-Type"] = "application/x-www-form-urlencoded" },
    })
    local ok, err = http_result(resp)
    if err then return nil, err end
    local tokens = type(ok.body) == "string" and json.parse(ok.body) or ok.body
    if type(tokens) ~= "table" or not tokens.access_token then
      return nil, { status = ok.status, body = "token response missing access_token" }
    end
    return tokens, nil
  end

  --- Refresh-token grant. Returns same shape as exchange_code.
  function self:refresh(refresh_token)
    local endpoints = must(self:discover())
    local body = url.encode_form({
      grant_type    = "refresh_token",
      refresh_token = refresh_token,
      client_id     = self.client_id,
      client_secret = self.client_secret,
    })
    local resp = self._http.post(endpoints.token_endpoint, body, {
      headers = { ["Content-Type"] = "application/x-www-form-urlencoded" },
    })
    local ok, err = http_result(resp)
    if err then return nil, err end
    local tokens = type(ok.body) == "string" and json.parse(ok.body) or ok.body
    if type(tokens) ~= "table" or not tokens.access_token then
      return nil, { status = ok.status, body = "refresh response missing access_token" }
    end
    return tokens, nil
  end

  --- Verify a JWT id_token. Issuer + audience checked; signature validated
  --- against the issuer's JWKS. Returns (claims, nil) on success or
  --- (nil, err) where err is a string from crypto.jwt_verify.
  function self:verify_id_token(id_token)
    local endpoints = must(self:discover())
    if not self._jwks then
      local resp, err = http_result(self._http.get(endpoints.jwks_uri))
      if err then return nil, "jwks fetch: " .. tostring(err.body) end
      self._jwks = type(resp.body) == "string" and json.parse(resp.body) or resp.body
    end
    -- With a JWKS table the algorithm is auto-derived from the JWK; only
    -- pass issuer + audience here.
    local ok, result = pcall(crypto.jwt_verify, id_token, self._jwks, {
      issuer   = self.issuer,
      audience = self.client_id,
    })
    if not ok then return nil, tostring(result) end
    return result.claims, nil
  end

  return self
end

return M
