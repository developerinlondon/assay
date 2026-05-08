local render = require("pages.render")
local ctx    = require("sysops.ctx")
local form   = require("pages.form")
local auth   = require("sysops.auth")

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
  local path = (req and req.path) or ""
  local id   = path:match("^/auth/users/([^/]+)/edit$")
  if not id then return { status = 404, body = "not found" } end
  local sdk = auth.new(ctx.engine).users
  local data, err = sdk.get(id)
  local user = (data and type(data) == "table") and data or { id = id }
  local q = (req and req.params) or {}
  return render.render("auth/user_edit", {
    nav_active  = "auth:users",
    title       = "Edit user · auth",
    page_title  = "Edit user",
    user        = user,
    error       = err,
    status      = err and err.status or 200,
    error_msg   = q.error and q.error or nil,
  }, req)
end

function M.save(req)
  local path = (req and req.path) or ""
  local id   = path:match("^/auth/users/([^/]+)/edit$")
  if not id then return { status = 404, body = "not found" } end
  local f   = form.parse(req)
  local sdk = auth.new(ctx.engine).users
  local fields = {
    email          = nz(f.email),
    display_name   = nz(f.display_name),
    email_verified = (f.email_verified == "on" or f.email_verified == "true"),
  }
  local _, err = sdk.update(id, fields)
  if err then
    return { status = 303, headers = { Location = "/auth/users/" .. id .. "/edit?error=" .. urlenc(tostring(err.status) .. ":save failed") } }
  end
  return { status = 303, headers = { Location = "/auth/users" } }
end

function M.delete(req)
  local path = (req and req.path) or ""
  local id   = path:match("^/auth/users/([^/]+)/delete$")
  if not id then return { status = 404, body = "not found" } end
  local sdk = auth.new(ctx.engine).users
  local _, err = sdk.delete(id)
  if err then
    return { status = 303, headers = { Location = "/auth/users?error=" .. urlenc(tostring(err.status) .. ":delete failed") } }
  end
  return { status = 303, headers = { Location = "/auth/users" } }
end

return M
