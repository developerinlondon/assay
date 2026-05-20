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

return M
