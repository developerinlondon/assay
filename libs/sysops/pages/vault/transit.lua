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

local function keys_page(req, extra)
  local q   = (req and req.params) or {}
  local sdk = vault.new(ctx.engine).transit
  local data, err = sdk.keys()
  local keys = {}
  if data and type(data.items) == "table" then
    keys = data.items
  elseif data and type(data.keys) == "table" then
    keys = data.keys
  elseif type(data) == "table" and not data.keys then
    for _, v in pairs(data) do
      if type(v) == "table" then keys[#keys + 1] = v end
    end
  end
  local ctx_tbl = {
    nav_active = "vault:transit",
    title      = "Transit · Vault",
    page_title = "Vault transit",
    keys       = keys,
    error      = err,
    status     = err and err.status or 200,
    error_msg  = q.error or nil,
    ok_msg     = q.ok    or nil,
  }
  if extra then
    for k, v in pairs(extra) do ctx_tbl[k] = v end
  end
  return render.render("vault/transit", ctx_tbl, req)
end

function M.page(req)
  return keys_page(req)
end

function M.create(req)
  local f   = form.parse(req)
  local sdk = vault.new(ctx.engine).transit
  if not nz(f.name) then
    return { status = 303, headers = { Location = "/vault/transit?error=400:name+required" } }
  end
  local _, err = sdk.create(f.name, nz(f.algo))
  if err then
    return {
      status  = 303,
      headers = {
        Location = "/vault/transit"
          .. "?error=" .. urlenc(("create failed (status %s)"):format(err.status or "?"))
          .. "&form_name=" .. urlenc(f.name or "")
          .. "&form_algo=" .. urlenc(f.algo or ""),
      },
    }
  end
  return { status = 303, headers = { Location = "/vault/transit?ok=created" } }
end

function M.rotate(req)
  local f   = form.parse(req)
  local sdk = vault.new(ctx.engine).transit
  if not nz(f.name) then
    return { status = 303, headers = { Location = "/vault/transit?error=400:name+required" } }
  end
  local _, err = sdk.rotate(f.name)
  if err then
    return { status = 303, headers = { Location = "/vault/transit?error=" .. urlenc(tostring(err.status) .. ":rotate failed") } }
  end
  return { status = 303, headers = { Location = "/vault/transit?ok=rotated" } }
end

function M.encrypt(req)
  local f   = form.parse(req)
  local sdk = vault.new(ctx.engine).transit
  if not nz(f.name)      then return { status = 303, headers = { Location = "/vault/transit?error=400:name+required" } } end
  if not nz(f.plaintext) then return { status = 303, headers = { Location = "/vault/transit?error=400:plaintext+required" } } end
  local data, err = sdk.encrypt(f.name, f.plaintext)
  if err then
    return { status = 303, headers = { Location = "/vault/transit?error=" .. urlenc(tostring(err.status) .. ":encrypt failed") } }
  end
  local ciphertext = (type(data) == "table" and data.ciphertext) or ""
  return render.render("vault/transit_op_result", {
    nav_active  = "vault:transit",
    title       = "Transit encrypt result · Vault",
    page_title  = "Transit encrypt result",
    op          = "encrypt",
    name        = f.name,
    input_kind  = "plaintext (" .. #f.plaintext .. " bytes)",
    output_kind = "ciphertext",
    output_val  = ciphertext,
    redacted    = false,
  }, req)
end

function M.decrypt(req)
  local f   = form.parse(req)
  local sdk = vault.new(ctx.engine).transit
  if not nz(f.name)       then return { status = 303, headers = { Location = "/vault/transit?error=400:name+required" } } end
  if not nz(f.ciphertext) then return { status = 303, headers = { Location = "/vault/transit?error=400:ciphertext+required" } } end
  local data, err = sdk.decrypt(f.name, f.ciphertext)
  if err then
    return { status = 303, headers = { Location = "/vault/transit?error=" .. urlenc(tostring(err.status) .. ":decrypt failed") } }
  end
  local plaintext = (type(data) == "table" and data.plaintext) or ""
  return render.render("vault/transit_op_result", {
    nav_active  = "vault:transit",
    title       = "Transit decrypt result · Vault",
    page_title  = "Transit decrypt result",
    op          = "decrypt",
    name        = f.name,
    input_kind  = "ciphertext",
    output_kind = "plaintext (redacted; " .. #plaintext .. " bytes)",
    output_val  = plaintext,
    redacted    = true,
  }, req)
end

return M
