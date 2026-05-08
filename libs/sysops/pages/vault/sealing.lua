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

local function kid_short(seal)
  if seal and type(seal.kid) == "string" and seal.kid ~= "" then
    local k = seal.kid
    if #k > 12 then return k:sub(1, 12) .. "…" end
    return k
  end
  return "—"
end

function M.page(req)
  local q    = (req and req.params) or {}
  local sdk  = vault.new(ctx.engine).sealing
  local seal, err = sdk.status()
  -- Tag the response with `_status` so the template's
  -- `{% if seal._status == 0 or seal._status == nil %}` branch resolves
  -- to the unavailable banner only when the call actually failed.
  if seal and not err then seal._status = 200 end
  return render.render("vault/sealing", {
    nav_active = "vault:sealing",
    title      = "Sealing · Vault",
    page_title = "Vault sealing",
    seal       = seal or { _status = (err and err.status) or 0 },
    kid_short  = kid_short(seal),
    error      = err,
    status     = err and err.status or 200,
    error_msg  = q.error or nil,
    ok_msg     = q.ok    or nil,
  }, req)
end

function M.seal(_req)
  local sdk = vault.new(ctx.engine).sealing
  local _, err = sdk.seal()
  if err then
    return { status = 303, headers = { Location = "/vault/sealing?error=" .. urlenc(tostring(err.status) .. ":seal failed") } }
  end
  return { status = 303, headers = { Location = "/vault/sealing?ok=sealed" } }
end

function M.unseal(req)
  local f   = form.parse(req)
  local sdk = vault.new(ctx.engine).sealing
  if not nz(f.share_b64) then
    return { status = 303, headers = { Location = "/vault/sealing?error=400:share_b64+required" } }
  end
  local _, err = sdk.unseal(f.share_b64)
  if err then
    return { status = 303, headers = { Location = "/vault/sealing?error=" .. urlenc(tostring(err.status) .. ":unseal failed") } }
  end
  return { status = 303, headers = { Location = "/vault/sealing" } }
end

function M.init(req)
  local f = form.parse(req)
  local n = tonumber(f.shares_count)
  local t = tonumber(f.threshold)
  if not n or not t then
    return { status = 303, headers = { Location = "/vault/sealing?error=400:shares_count+and+threshold+required" } }
  end
  local sdk = vault.new(ctx.engine).sealing
  local data, err = sdk.init(n, t)
  if err then
    return { status = 303, headers = { Location = "/vault/sealing?error=" .. urlenc(tostring(err.status) .. ":init failed") } }
  end
  local b = type(data) == "table" and data or {}
  return render.render("vault/sealing_init_result", {
    nav_active   = "vault:sealing",
    title        = "Init result · Vault sealing",
    page_title   = "Vault sealing — init result",
    kid          = b.kid or "—",
    threshold    = b.threshold or t,
    shares_count = b.shares_count or n,
    shares       = b.shares_b64 or b.shares or {},
  }, req)
end

return M
