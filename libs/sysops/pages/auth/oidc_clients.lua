local render = require("pages.render")
local ctx    = require("sysops.ctx")
local auth   = require("sysops.auth")
local form   = require("pages.form")

local M = {}

local function urlenc(s)
  return (tostring(s or "")):gsub("([^%w%-_%.~])", function(c)
    return string.format("%%%02X", string.byte(c))
  end)
end

local function nz(s)
  if s == nil or s == "" then return nil end
  return s
end

local function split_csv(s)
  if not s or s == "" then return nil end
  local t = {}
  for v in s:gmatch("[^,]+") do
    local trimmed = v:match("^%s*(.-)%s*$")
    if trimmed ~= "" then t[#t + 1] = trimmed end
  end
  return #t > 0 and t or nil
end

function M.page(req)
  local q   = (req and req.params) or {}
  local sdk = auth.new(ctx.engine).oidc
  local data, err = sdk.clients()
  -- Engine returns either a bare JSON array or {items:[...]} depending on
  -- version. Accept both shapes.
  local clients = {}
  if type(data) == "table" then
    if type(data.items) == "table" then
      clients = data.items
    elseif #data > 0 or next(data) == nil then
      clients = data
    end
  end
  return render.render("auth/oidc_clients", {
    nav_active  = "auth:oidc_clients",
    title       = "OIDC clients · auth",
    page_title  = "OIDC clients",
    clients     = clients,
    error       = err,
    status      = err and err.status or 200,
    error_msg   = q.error and q.error or nil,
    ok_msg      = q.ok    and q.ok    or nil,
  }, req)
end

function M.create(req)
  local f   = form.parse(req)
  local sdk = auth.new(ctx.engine).oidc
  if not nz(f.redirect_uris) then
    return { status = 303, headers = { Location = "/auth/oidc-clients?error=400:redirect_uris+required" } }
  end
  local fields = {
    client_id                  = nz(f.client_id),
    name                       = nz(f.name),
    redirect_uris              = split_csv(f.redirect_uris),
    grant_types                = split_csv(f.grant_types),
    token_endpoint_auth_method = nz(f.token_endpoint_auth_method) or "none",
    is_public                  = (f.is_public == "on"),
  }
  local _, err = sdk.create_client(fields)
  if err then
    return { status = 303, headers = { Location = "/auth/oidc-clients?error=" .. urlenc(tostring(err.status) .. ":create failed") } }
  end
  return { status = 303, headers = { Location = "/auth/oidc-clients?ok=client+created" } }
end

function M.delete(req)
  local path = (req and req.path) or ""
  local id   = path:match("^/auth/oidc%-clients/([^/]+)/delete$")
  if not id then return { status = 404, body = "not found" } end
  local sdk = auth.new(ctx.engine).oidc
  local _, err = sdk.delete_client(id)
  if err then
    return { status = 303, headers = { Location = "/auth/oidc-clients?error=" .. urlenc(tostring(err.status) .. ":delete failed") } }
  end
  return { status = 303, headers = { Location = "/auth/oidc-clients?ok=client+deleted" } }
end

function M.rotate(req)
  local path = (req and req.path) or ""
  local id   = path:match("^/auth/oidc%-clients/([^/]+)/rotate%-secret$")
  if not id then return { status = 404, body = "not found" } end
  local sdk = auth.new(ctx.engine).oidc
  local data, err = sdk.rotate_client_secret(id)
  if err then
    return { status = 303, headers = { Location = "/auth/oidc-clients?error=" .. urlenc(tostring(err.status) .. ":rotate failed") } }
  end
  return render.render("auth/oidc_client_secret_result", {
    nav_active  = "auth:oidc_clients",
    title       = "New client secret · auth",
    page_title  = "New client secret",
    client_id   = data and data.client_id or id,
    new_secret  = data and data.client_secret or "",
  }, req)
end

return M
