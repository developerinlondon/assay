local render = require("pages.render")
local ctx    = require("sysops.ctx")
local vault  = require("sysops.vault")

local M = {}

local function nz(s)
  if s == nil then return nil end
  if type(s) == "string" and s == "" then return nil end
  return s
end

function M.page(req)
  local q       = (req and req.params) or {}
  local user_id = q.user_id or ""
  local meta, items, err, status

  if nz(user_id) then
    local sdk = vault.new(ctx.engine).me
    local data
    data, err = sdk.sync(user_id)
    if err then
      status = err.status
      meta   = nil
      items  = {}
    else
      meta   = (type(data) == "table" and data.profile) or data or {}
      items  = (type(data) == "table" and type(data.ciphers) == "table" and data.ciphers) or {}
      status = 200
    end
  else
    status = 200
    meta   = nil
    items  = {}
  end

  return render.render("vault/me", {
    nav_active = "vault:me",
    title      = "My vault · Vault",
    page_title = "My vault",
    user_id    = user_id,
    meta       = meta,
    items      = items,
    error      = err,
    status     = status,
  }, req)
end

return M
