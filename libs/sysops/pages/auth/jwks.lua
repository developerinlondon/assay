local render = require("pages.render")
local ctx    = require("sysops.ctx")
local auth   = require("sysops.auth")

local M = {}

local function short(s, n)
  if not s then return nil end
  s = tostring(s)
  n = n or 24
  if #s > n then return s:sub(1, n) .. "…" end
  return s
end

function M.page(req)
  local q   = (req and req.params) or {}
  local sdk = auth.new(ctx.engine).oidc
  local data, err = sdk.jwks()
  local raw_keys = (data and type(data.keys) == "table") and data.keys or {}
  local keys = {}
  for _, k in ipairs(raw_keys) do
    keys[#keys + 1] = {
      kid       = k.kid,
      alg       = k.alg,
      kty       = k.kty,
      use       = k.use,
      crv       = k.crv,
      n_full    = k.n, n_short = short(k.n),
      e_full    = k.e, e_short = short(k.e, 8),
      x_full    = k.x, x_short = short(k.x),
      y_full    = k.y, y_short = short(k.y),
      created_at = k.created_at,
    }
  end
  local source = (data and data.source) or "engine"
  return render.render("auth/jwks", {
    nav_active  = "auth:jwks",
    title       = "JWKS · auth",
    page_title  = "JWKS",
    keys        = keys,
    source      = source,
    error       = err,
    status      = err and err.status or 200,
  }, req)
end

return M
