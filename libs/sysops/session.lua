--! sysops.session - HMAC-signed session cookie (compact JWT-like format)
--! plus an in-memory ephemeral store for OIDC pending state + refresh
--! tokens.
--!
--! Cookie format: `<b64url(json(claims))>.<b64url(hmac_sha256(key, b64url(json(claims))))>`
--!
--! HS256-style. We don't use crypto.jwt_sign because that only supports
--! RS256/384/512 (RSA PEM). HMAC-on-bytes via crypto.hmac is sufficient
--! and matches every OIDC BFF pattern (xandar-ui does the same).
--!
--! Usage:
--!   local session = require("sysops.session")
--!   local s = session.new({
--!     signing_key = "<32+ byte secret>",
--!     ttl_seconds = 86400,
--!     cookie_name = "gondor_session",   -- default "sysops_session"
--!   })
--!   local cookie = s:issue({ sub = "alice@example", email = "alice@example" })
--!   local claims, err = s:verify(cookie)
--!
--! Store:
--!   local store = session.store_new()
--!   store:put_pending(state, { verifier = ..., return_to = "/" })
--!   local pending = store:take_pending(state)   -- one-shot
--!   store:put_refresh(sub, refresh_token)
--!   store:revoke(sub)

local codec = require("sysops.codec")
local M     = {}

----------------------------------------------------------------------
-- Cookie signer
----------------------------------------------------------------------

function M.new(opts)
  if type(opts) ~= "table" then
    error("session.new: opts table required", 2)
  end
  if type(opts.signing_key) ~= "string" or #opts.signing_key < 32 then
    error("session.new: signing_key must be a string of ≥32 bytes", 2)
  end

  local self = {}
  self.signing_key = opts.signing_key
  self.ttl_seconds = opts.ttl_seconds or 86400
  self.cookie_name = opts.cookie_name or "sysops_session"

  --- Mint a signed cookie value. If claims.exp is absent, it's set to
  --- now + ttl_seconds. Returns the cookie value (NOT the full
  --- Set-Cookie header — callers add HttpOnly/Secure/Path/...).
  function self:issue(claims)
    if type(claims) ~= "table" then
      error("session:issue: claims table required", 2)
    end
    if not claims.exp then claims.exp = os.time() + self.ttl_seconds end
    if not claims.iat then claims.iat = os.time() end
    local payload = codec.b64url(json.encode(claims))
    local sig_bytes = crypto.hmac(self.signing_key, payload, "sha256", true)
    return payload .. "." .. codec.b64url(sig_bytes)
  end

  --- Verify a cookie value. Returns (claims, nil) on success or
  --- (nil, err_string) for malformed/bad-signature/expired.
  function self:verify(cookie_value)
    if type(cookie_value) ~= "string" then return nil, "missing" end
    local payload, sig = cookie_value:match("^([^.]+)%.([^.]+)$")
    if not payload then return nil, "malformed" end

    local expected_bytes = crypto.hmac(self.signing_key, payload, "sha256", true)
    local got_bytes, derr = codec.b64url_decode(sig)
    if not got_bytes then return nil, "bad signature" end
    if not codec.consteq(got_bytes, expected_bytes) then
      return nil, "bad signature"
    end

    local payload_json, jerr = codec.b64url_decode(payload)
    if not payload_json then return nil, "bad payload" end
    local ok, claims = pcall(json.parse, payload_json)
    if not ok or type(claims) ~= "table" then return nil, "bad payload" end

    if claims.exp and claims.exp < os.time() then return nil, "expired" end
    return claims
  end

  return self
end

----------------------------------------------------------------------
-- In-memory ephemeral store
--
-- Holds:
--   - pending OIDC state {state -> {verifier, return_to, created_at}}
--   - per-user refresh tokens {sub -> {refresh_token, ...}}
--
-- Refresh tokens are lost on sysops restart. v1 trade-off: users
-- re-login. Multi-replica deployments need an external store
-- (engine vault, postgres) — out of scope for the initial cut.
----------------------------------------------------------------------

local PENDING_TTL_SECONDS = 300 -- 5 min: long enough for any login UX

function M.store_new()
  local store = {
    _pending  = {},  -- state -> { verifier, return_to, created_at }
    _refresh  = {},  -- sub   -> { refresh_token, expires_at }
  }

  local function gc_pending(self)
    local cutoff = os.time() - PENDING_TTL_SECONDS
    for k, v in pairs(self._pending) do
      if (v.created_at or 0) < cutoff then self._pending[k] = nil end
    end
  end

  function store:put_pending(state, value)
    if type(state) ~= "string" or #state < 8 then
      error("session.store:put_pending: state must be ≥8 chars", 2)
    end
    value.created_at = os.time()
    gc_pending(self)
    self._pending[state] = value
  end

  function store:take_pending(state)
    gc_pending(self)
    local v = self._pending[state]
    self._pending[state] = nil -- one-shot
    return v
  end

  function store:put_refresh(sub, refresh_token, expires_at)
    self._refresh[sub] = {
      refresh_token = refresh_token,
      expires_at    = expires_at,
    }
  end

  function store:get_refresh(sub)
    return self._refresh[sub]
  end

  function store:revoke(sub)
    self._refresh[sub] = nil
  end

  return store
end

----------------------------------------------------------------------
-- Safe-return helper. Open-redirect mitigation for /auth/login's
-- return_to param and /auth/callback's resume target. Accepts only
-- relative paths anchored at /; anything that looks like an
-- absolute URL, protocol-relative ("//evil"), backslash-mixed
-- ("/\evil"), or contains CR/LF gets clamped to "/".
----------------------------------------------------------------------

function M.safe_return_to(s)
  if type(s) ~= "string" or s == "" then return "/" end
  if s:find("[\r\n%z]") then return "/" end             -- header smuggling
  if s:find("://", 1, true) then return "/" end          -- http://evil, javascript://
  if s:sub(1, 1) ~= "/" then return "/" end              -- must be absolute path
  if s:sub(1, 2) == "//" then return "/" end             -- protocol-relative
  if s:sub(2, 2) == "\\" then return "/" end             -- "/\evil"
  return s
end

----------------------------------------------------------------------
-- Cookie-header parsing helper. Sysops handlers see a single
-- `Cookie:` header (or `cookie:`); pulling out one named cookie value
-- belongs here because session.lua owns the cookie domain.
----------------------------------------------------------------------

--- Extract a named cookie value from a Cookie header. Returns nil if
--- the header is empty or the cookie isn't present.
function M.parse_cookie_header(header, name)
  if type(header) ~= "string" or header == "" or type(name) ~= "string" then
    return nil
  end
  for kv in header:gmatch("([^;]+)") do
    local k, v = kv:match("^%s*([^=]+)%s*=%s*(.*)%s*$")
    if k == name then
      return v
    end
  end
  return nil
end

return M
