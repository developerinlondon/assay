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
local authz   = require("sysops.authz")

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
--- cookie name the engine sets — the consumer app never sees that cookie).
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
      body    = json.encode({ error = tostring(err) }),
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

-- SSE detection. The dashboard SPAs open one EventSource at boot:
-- /api/v1/engine/workflow/events/stream. We pattern-match the suffix
-- so any future /events/stream endpoint also streams transparently.
-- Anything else falls through to the buffered forward.
local function is_sse_path(path)
  if type(path) ~= "string" then return false end
  return path:find("/events/stream", 1, true) ~= nil
end

--- Build the SSE-response shape http.serve understands: a table with an
--- `sse` function that drives `send(evt)` until upstream closes. We
--- open the upstream stream via http.get + on_event callback, which
--- the host's http builtin special-cases when the upstream
--- Content-Type is text/event-stream.
local function forward_sse(upstream_url, headers)
  return {
    status  = 200,
    headers = {
      ["Content-Type"]  = "text/event-stream",
      ["Cache-Control"] = "no-cache",
      ["Connection"]    = "keep-alive",
    },
    sse = function(send)
      http.get(upstream_url, {
        headers  = headers,
        on_event = function(evt)
          send(evt)
        end,
      })
    end,
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
    -- Per-resource authz. authz.is_allowed maps the path to a
    -- (resource, relation) and asks Zanzibar whether claims.sub holds
    -- it (cached per-tuple for 30s). Public-after-auth paths pass
    -- through without an engine call.
    local ok, reason = authz.is_allowed(claims.sub, req.path)
    if not ok then
      return {
        status  = 403,
        headers = { ["Content-Type"] = "application/json" },
        body    = json.encode({ error = "forbidden", reason = tostring(reason) }),
      }
    end

    fwd.authorization        = "Bearer " .. ctx.gateway_admin_bearer
    fwd["X-Forwarded-User"]  = claims.sub
    fwd["X-User-Id"]         = claims.sub
    if is_sse_path(req.path) then
      return forward_sse(upstream, fwd)
    end
    return forward(req.method or "GET", upstream, req.body, fwd)
  end

  -- No valid session — fall back to bearer pass-through if caller has one.
  local incoming_auth = headers.authorization or headers.Authorization
  if type(incoming_auth) == "string" and incoming_auth:match("^[Bb]earer ") then
    fwd.authorization = incoming_auth
    if is_sse_path(req.path) then
      return forward_sse(upstream, fwd)
    end
    return forward(req.method or "GET", upstream, req.body, fwd)
  end

  return { status = 401, body = '{"error":"unauthenticated"}' }
end

return M
