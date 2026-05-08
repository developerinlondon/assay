local render = require("pages.render")
local ctx    = require("sysops.ctx")
local auth   = require("sysops.auth")

local M = {}

local function short(s, n)
  if not s then return nil end
  s = tostring(s)
  n = n or 32
  if #s > n then return s:sub(1, n) .. "…" end
  return s
end

function M.page(req)
  local sdk = auth.new(ctx.engine).biscuit
  local data, err = sdk.info()
  local raw_keys = {}
  if data then
    if type(data.keys) == "table" then
      raw_keys = data.keys
    elseif type(data.kid) == "string" then
      raw_keys = { data }
    end
  end
  local keys = {}
  for _, k in ipairs(raw_keys) do
    keys[#keys + 1] = {
      kid          = k.kid,
      algorithm    = k.algorithm or k.alg,
      public_full  = k.public_key or k.x,
      public_short = short(k.public_key or k.x),
      created_at   = k.created_at,
      status       = k.status or "active",
    }
  end
  return render.render("auth/biscuit", {
    nav_active  = "auth:biscuit",
    title       = "Biscuit · auth",
    page_title  = "Biscuit",
    keys        = keys,
    error       = err,
    status      = err and err.status or 200,
  }, req)
end

return M
