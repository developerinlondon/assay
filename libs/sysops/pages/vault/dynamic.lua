local render = require("pages.render")
local ctx    = require("sysops.ctx")
local vault  = require("sysops.vault")
local form   = require("pages.form")

local M = {}

local function nz(s)
  if s == nil then return nil end
  if type(s) == "string" and s == "" then return nil end
  return s
end

local function urlenc(s)
  return (tostring(s or "")):gsub("([^%w%-_%.~])", function(c)
    return string.format("%%%02X", string.byte(c))
  end)
end

function M.page(req)
  local q        = (req and req.params) or {}
  local provider = q.provider or ""
  local sdk      = vault.new(ctx.engine).dynamic
  local data, err = sdk.list(nz(provider))
  local leases = {}
  if data and type(data.items) == "table" then
    leases = data.items
  elseif data and type(data.leases) == "table" then
    leases = data.leases
  elseif type(data) == "table" and not data.leases then
    leases = data
  end
  return render.render("vault/dynamic", {
    nav_active = "vault:dynamic",
    title      = "Leases · Vault",
    page_title = "Dynamic credentials",
    provider   = provider,
    leases     = leases,
    error      = err,
    status     = err and err.status or 200,
    error_msg  = q.error or nil,
    ok_msg     = q.ok    or nil,
  }, req)
end

function M.lease(req)
  local f   = form.parse(req)
  local sdk = vault.new(ctx.engine).dynamic
  if not nz(f.provider) then
    return { status = 303, headers = { Location = "/vault/dynamic?error=400:provider+required" } }
  end
  if not nz(f.role) then
    return { status = 303, headers = { Location = "/vault/dynamic?error=400:role+required" } }
  end
  local _, err = sdk.lease(f.provider, f.role)
  if err then
    return {
      status  = 303,
      headers = {
        Location = "/vault/dynamic"
          .. "?error=" .. urlenc(("lease failed (status %s)"):format(err.status or "?"))
          .. "&form_provider=" .. urlenc(f.provider or "")
          .. "&form_role=" .. urlenc(f.role or ""),
      },
    }
  end
  return { status = 303, headers = { Location = "/vault/dynamic?ok=leased" } }
end

function M.revoke(req)
  local path = (req and req.path) or ""
  local id   = path:match("^/vault/dynamic/leases/([^/]+)/revoke$")
  if not id then
    local f = form.parse(req)
    id = f.id
  end
  if not nz(id) then
    return { status = 303, headers = { Location = "/vault/dynamic?error=400:lease+id+required" } }
  end
  local sdk = vault.new(ctx.engine).dynamic
  local _, err = sdk.revoke(id)
  if err then
    return { status = 303, headers = { Location = "/vault/dynamic?error=" .. urlenc(tostring(err.status) .. ":revoke failed") } }
  end
  return { status = 303, headers = { Location = "/vault/dynamic?ok=revoked" } }
end

return M
