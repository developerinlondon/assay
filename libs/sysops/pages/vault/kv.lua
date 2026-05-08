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
  local prefix = q.prefix or ""
  local sdk    = vault.new(ctx.engine).kv
  local data, err = sdk.list(prefix)
  local entries = {}
  if data and type(data.items) == "table" then
    entries = data.items
  elseif data and type(data.entries) == "table" then
    entries = data.entries
  elseif data and type(data.keys) == "table" then
    for _, k in ipairs(data.keys) do
      entries[#entries + 1] = { path = k }
    end
  end
  return render.render("vault/kv", {
    nav_active = "vault:kv",
    title      = "KV · Vault",
    page_title = "Vault KV",
    prefix     = prefix,
    entries    = entries,
    error      = err,
    status     = err and err.status or 200,
    error_msg  = q.error or nil,
    ok_msg     = q.ok    or nil,
  }, req)
end

function M.put(req)
  local f   = form.parse(req)
  local sdk = vault.new(ctx.engine).kv
  if not nz(f.path) then
    return { status = 303, headers = { Location = "/vault/kv?error=400:path+required" } }
  end
  local _, err = sdk.put(f.path, f.data or "")
  if err then
    return { status = 303, headers = { Location = "/vault/kv?error=" .. urlenc(tostring(err.status) .. ":put failed") } }
  end
  return { status = 303, headers = { Location = "/vault/kv?ok=put&path=" .. urlenc(f.path) } }
end

function M.delete(req)
  local f   = form.parse(req)
  local sdk = vault.new(ctx.engine).kv
  if not nz(f.path) then
    return { status = 303, headers = { Location = "/vault/kv?error=400:path+required" } }
  end
  local version = tonumber(f.version)
  local _, err = sdk.delete(f.path, version)
  if err then
    return { status = 303, headers = { Location = "/vault/kv?error=" .. urlenc(tostring(err.status) .. ":delete failed") } }
  end
  return { status = 303, headers = { Location = "/vault/kv?ok=deleted" } }
end

return M
