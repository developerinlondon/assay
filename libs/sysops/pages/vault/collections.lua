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
  local q      = (req and req.params) or {}
  local org_id = q.org_id or ""
  local sdk    = vault.new(ctx.engine).collections
  local data, err = sdk.list()
  local collections = {}
  if data and type(data.items) == "table" then
    collections = data.items
  elseif data and type(data.collections) == "table" then
    collections = data.collections
  elseif data and type(data.folders) == "table" then
    collections = data.folders
  elseif type(data) == "table" and not data.collections and not data.folders then
    collections = data
  end
  if org_id ~= "" then
    local filtered = {}
    for _, c in ipairs(collections) do
      if c.org_id == org_id then filtered[#filtered + 1] = c end
    end
    collections = filtered
  end
  return render.render("vault/collections", {
    nav_active  = "vault:collections",
    title       = "Collections · Vault",
    page_title  = "Vault collections",
    org_id      = org_id,
    collections = collections,
    error       = err,
    status      = err and err.status or 200,
    error_msg   = q.error or nil,
    ok_msg      = q.ok    or nil,
  }, req)
end

function M.create(req)
  local f   = form.parse(req)
  local sdk = vault.new(ctx.engine).collections
  if not nz(f.name) then
    return { status = 303, headers = { Location = "/vault/collections?error=400:name+required" } }
  end
  local _, err = sdk.create(f.name, nz(f.description))
  if err then
    return {
      status  = 303,
      headers = {
        Location = "/vault/collections"
          .. "?error=" .. urlenc(("create failed (status %s)"):format(err.status or "?"))
          .. "&form_name=" .. urlenc(f.name or "")
          .. "&form_description=" .. urlenc(f.description or ""),
      },
    }
  end
  return { status = 303, headers = { Location = "/vault/collections?ok=created" } }
end

return M
