--! sysops.gateway - HTTP gateway that sits between the browser and
--! the assay-engine. Two responsibilities (Task 5 will add the proxy):
--!
--!   1. INTERCEPT  /api/v1/engine/auth/whoami — answer locally from the
--!      session cookie. The dashboard SPAs (auth, engine consoles) call
--!      this at boot to detect "am I signed in"; suppressing their
--!      token-banner branch requires a 200 response with the session
--!      identity, NOT forwarding to the engine.
--!
--!   2. PROXY      everything else under /api/v1/engine/* (and the
--!      /workflow, /vault, /engine/console, /shared/* asset paths).
--!      Adds in Task 5.
--!
--! Each handler is exposed as `M.<slug>(req)` so mount.lua's pages
--! registry can map routes → handlers the same way every other sysops
--! page does.

local ctx     = require("sysops.ctx")
local session = require("sysops.session")

local M = {}

----------------------------------------------------------------------
-- whoami intercept
----------------------------------------------------------------------

--- GET /api/v1/engine/auth/whoami
---
--- Reads the session cookie, validates it, returns { sub, email, user_id }
--- as JSON. The dashboard SPAs use this exact endpoint to detect a
--- live session at boot; we answer it here instead of forwarding to
--- the engine (whose own /whoami requires `assay_session`, a different
--- cookie name the engine sets — gondor never sees that cookie).
function M.whoami(req)
  if not ctx.session_signer then
    return { status = 503, body = '{"error":"auth gateway not configured"}' }
  end
  local headers = (req and req.headers) or {}
  local cookie_header = headers.cookie or headers.Cookie or ""
  local cookie_val = session.parse_cookie_header(cookie_header, ctx.session_signer.cookie_name)
  if not cookie_val then
    return {
      status  = 401,
      headers = { ["Content-Type"] = "application/json" },
      body    = '{"error":"no session"}',
    }
  end
  local claims, err = ctx.session_signer:verify(cookie_val)
  if not claims then
    return {
      status  = 401,
      headers = { ["Content-Type"] = "application/json" },
      body    = '{"error":"' .. tostring(err) .. '"}',
    }
  end
  return {
    status  = 200,
    headers = { ["Content-Type"] = "application/json" },
    body    = json.encode({
      sub     = claims.sub,
      email   = claims.email,
      user_id = claims.sub,
    }),
  }
end

----------------------------------------------------------------------
-- Dual-mode reverse proxy
--
-- Two paths depending on the caller's credentials:
--
--   1. Authorization: Bearer <…> incoming → forward as-is.
--      Engine validates (admin bearer or trusted-issuer JWT). Preserves
--      ssh+curl, CI scripts, the dashboard SPA's paste-a-token banner
--      mode, and customer-IdP-JWT direct calls.
--
--   2. Valid session cookie, no Authorization → inject the gateway's
--      configured admin bearer + X-User-Id. Optional Zanzibar role
--      check (off by default in v1).
--
--   3. Otherwise → 401.
----------------------------------------------------------------------

-- RFC 7230 hop-by-hop headers + auth-sensitive ones we must NOT forward.
local STRIP_HEADERS = {
  ["connection"]          = true,
  ["keep-alive"]          = true,
  ["proxy-authenticate"]  = true,
  ["proxy-authorization"] = true,
  ["te"]                  = true,
  ["trailer"]             = true,
  ["transfer-encoding"]   = true,
  ["upgrade"]             = true,
  ["host"]                = true, -- http client sets this
  ["cookie"]              = true, -- never leak sysops session to engine
}

local function clean_headers(headers)
  local out = {}
  for k, v in pairs(headers or {}) do
    if not STRIP_HEADERS[tostring(k):lower()] then
      out[k] = v
    end
  end
  return out
end

local function build_upstream_url(base, path, raw_query)
  base = (base or ""):gsub("/$", "")
  path = path or "/"
  if path:sub(1, 1) ~= "/" then path = "/" .. path end
  local u = base .. path
  if raw_query and raw_query ~= "" then u = u .. "?" .. raw_query end
  return u
end

local function dispatch(method, url_str, body, headers)
  method = (method or "GET"):lower()
  if method == "get" or method == "delete" then
    return http[method](url_str, { headers = headers })
  end
  return http[method](url_str, body, { headers = headers })
end

local function forward(method, url_str, body, headers)
  local resp = dispatch(method, url_str, body, headers) or {}
  return {
    status  = resp.status or 502,
    headers = clean_headers(resp.headers or {}),
    body    = resp.body or "",
  }
end

--- ANY /api/v1/engine/* (and /workflow, /vault, /engine/console, /shared)
---
--- Resolution priority (deliberate — see commit history for the
--- session-first switch):
---
---   1. Valid session cookie  → inject the gateway's admin bearer.
---      Ignores any Authorization header from the caller — dashboard
---      SPAs ship a stale `assay-admin-token` from localStorage and
---      forwarding it 401s the browser flow.
---   2. No (or invalid) session, but caller has a bearer
---      → pass through unchanged. Lets CI / curl / customer-IdP-JWT
---      paths reach the engine without a sysops session.
---   3. Otherwise → 401.
function M.proxy(req)
  if not ctx.session_signer then
    return { status = 503, body = '{"error":"auth gateway not configured"}' }
  end
  if not ctx.engine_upstream_url or not ctx.gateway_admin_bearer then
    return { status = 503, body = '{"error":"gateway not configured"}' }
  end

  req = req or {}
  local headers = req.headers or {}
  local upstream = build_upstream_url(ctx.engine_upstream_url, req.path, req.raw_query)
  local fwd = clean_headers(headers)

  -- Try session-injection first.
  local cookie_header = headers.cookie or headers.Cookie or ""
  local cookie_val = session.parse_cookie_header(cookie_header, ctx.session_signer.cookie_name)
  local claims = nil
  if cookie_val then
    claims = (ctx.session_signer:verify(cookie_val))
  end

  if claims then
    if ctx.authz_require_admin then
      if type(ctx.zanzibar_check) ~= "function" then
        return { status = 503, body = '{"error":"authz_require_admin set but no zanzibar_check"}' }
      end
      if not ctx.zanzibar_check(claims.sub) then
        return { status = 403, body = '{"error":"forbidden"}' }
      end
    end
    fwd.authorization        = "Bearer " .. ctx.gateway_admin_bearer
    fwd["X-Forwarded-User"]  = claims.sub
    fwd["X-User-Id"]         = claims.sub
    return forward(req.method or "GET", upstream, req.body, fwd)
  end

  -- No valid session — fall back to bearer pass-through if caller has one.
  local incoming_auth = headers.authorization or headers.Authorization
  if type(incoming_auth) == "string" and incoming_auth:match("^[Bb]earer ") then
    fwd.authorization = incoming_auth
    return forward(req.method or "GET", upstream, req.body, fwd)
  end

  return { status = 401, body = '{"error":"unauthenticated"}' }
end

return M
