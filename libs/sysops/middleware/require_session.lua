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
local authz   = require("sysops.authz")

local M = {}

-- Prefix-safe URL builder. ctx.url is populated by mount.lua and
-- prepends the configured mount prefix (e.g. /host) to every absolute
-- path. Fall back to identity when no prefix is set (legacy / tests).
local function u(path)
  if ctx.url then return ctx.url(path) end
  return path
end

local function render_forbidden(claims, reason)
  return {
    status = 403,
    headers = { ["Content-Type"] = "text/html; charset=utf-8" },
    body = table.concat({
      "<!doctype html><html><head><title>Forbidden</title>",
      "<link rel='stylesheet' href='", u("/static/styles.css"), "'></head><body>",
      "<div style='max-width:560px;margin:6rem auto;padding:2rem;",
      "font-family:system-ui;text-align:center'>",
      "<h1 style='margin:0 0 1rem'>Access denied</h1>",
      "<p>You're signed in as <code>", tostring(claims.email or claims.sub),
      "</code>, but you don't have permission to view this page.</p>",
      "<p style='color:var(--fg-2);font-size:0.9em'>Reason: ",
      tostring(reason), "</p>",
      "<p><a href='", u("/"), "'>← Back to dashboard</a> &nbsp;",
      "<a href='", u("/auth/logout"), "'>Sign out</a></p>",
      "</div></body></html>",
    }),
  }
end

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
        headers = { Location = u("/auth/login") .. "?return_to=" .. urlencode(return_to) },
      }
    end

    -- Per-resource authz: ask Zanzibar whether this user is allowed to
    -- access req.path. Public-after-auth paths (/, /auth/login, …)
    -- short-circuit inside authz.is_allowed without an engine call.
    local ok, reason = authz.is_allowed(claims.sub, (req and req.path) or "")
    if not ok then return render_forbidden(claims, reason) end

    -- Attach claims so inner handlers can read who's calling.
    req = req or {}
    req.session_claims = claims
    return inner_handler(req)
  end
end

return M
