--! /auth/logout — revoke the server-side refresh token (if any) and
--! clear the session cookie. 302 back to "/" so the next request goes
--! through the unauthenticated path (and, in practice, /auth/login).

local ctx     = require("sysops.ctx")
local session = require("sysops.session")

local M = {}

function M.page(req)
  if not ctx.session_signer or not ctx.session_store then
    return { status = 503, body = "auth gateway not configured" }
  end

  local headers = (req and req.headers) or {}
  local cookie_header = headers.cookie or headers.Cookie or ""
  local cookie_val = session.parse_cookie_header(cookie_header, ctx.session_signer.cookie_name)

  if cookie_val then
    local claims = ctx.session_signer:verify(cookie_val)
    if claims and claims.sub then
      ctx.session_store:revoke(claims.sub)
    end
  end

  return {
    status  = 302,
    headers = {
      Location     = "/",
      ["Set-Cookie"] = ctx.session_signer.cookie_name
        .. "=; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=0",
    },
  }
end

return M
