--! sysops middleware — gate a page handler on a valid session cookie.
--!
--! Usage in pages.lua / mount.lua:
--!
--!   local require_session = require("sysops.middleware.require_session")
--!   local users_pg        = require("pages.auth.users")
--!
--!   routes.GET["/auth/users"] = require_session.wrap(users_pg.page)
--!
--! If the request carries a valid session cookie (gondor_session
--! HMAC-validated by ctx.session_signer), the inner handler runs and
--! receives req.session_claims = { sub, email, ... }. Otherwise:
--!
--!   302 → /auth/login?return_to=<encoded original path>
--!
--! The middleware is a no-op pass-through when the auth gateway isn't
--! configured (ctx.session_signer == nil) — this lets consumers that
--! don't opt into OIDC keep their existing sysops UX (admin-bearer at
--! the engine layer, no browser-side auth).

local ctx     = require("sysops.ctx")
local session = require("sysops.session")

local M = {}

local function urlencode(s)
  return (tostring(s or "/")):gsub("([^%w%-_%.~])", function(c)
    return string.format("%%%02X", string.byte(c))
  end)
end

--- Wrap a page handler. Returns a new function with the same shape that
--- redirects unauthenticated callers to /auth/login.
function M.wrap(inner_handler)
  return function(req)
    -- No signer configured → pass-through (backward compat).
    if not ctx.session_signer then
      return inner_handler(req)
    end

    local headers = (req and req.headers) or {}
    local cookie_header = headers.cookie or headers.Cookie or ""
    local cookie_val = session.parse_cookie_header(
      cookie_header, ctx.session_signer.cookie_name
    )
    local claims = cookie_val and ({ ctx.session_signer:verify(cookie_val) })[1]
    if not claims then
      local return_to = (req and req.path) or "/"
      return {
        status  = 302,
        headers = { Location = "/auth/login?return_to=" .. urlencode(return_to) },
      }
    end

    -- Attach claims so inner handlers can read who's calling.
    req = req or {}
    req.session_claims = claims
    return inner_handler(req)
  end
end

return M
