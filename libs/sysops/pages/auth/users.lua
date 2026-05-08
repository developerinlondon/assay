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

local function id_short(id)
  if not id then return "?" end
  local s = tostring(id)
  if #s > 8 then return s:sub(1, 8) .. "…" end
  return s
end

function M.page(req)
  local q      = (req and req.params) or {}
  local search = q.search or ""
  local sdk    = auth.new(ctx.engine).users
  local data, err = sdk.list({ search = search ~= "" and search or nil })
  local users = {}
  if data and type(data.users) == "table" then
    for _, u in ipairs(data.users) do
      users[#users + 1] = {
        id            = u.id,
        id_short      = id_short(u.id),
        email         = u.email,
        display_name  = u.display_name,
        email_verified = u.email_verified,
        has_passkey   = u.has_passkey,
        created_at    = u.created_at,
      }
    end
  end
  local error_msg = q.error and q.error:gsub("^%d+:", "") or nil
  local ok_msg    = q.ok    and urlenc(q.ok)              or nil
  if q.ok and q.ok ~= "" then ok_msg = q.ok end
  return render.render("auth/users", {
    nav_active  = "auth:users",
    title       = "Users · auth",
    page_title  = "Users",
    users       = users,
    total       = (data and data.total) or #users,
    search      = search,
    error       = err,
    status      = err and err.status or 200,
    error_msg   = q.error and q.error or nil,
    ok_msg      = q.ok    and q.ok    or nil,
  }, req)
end

function M.create(req)
  local f   = form.parse(req)
  local sdk = auth.new(ctx.engine).users
  if not nz(f.email) then
    return { status = 303, headers = { Location = "/auth/users?error=400:email+required" } }
  end
  local fields = {
    email        = f.email,
    display_name = nz(f.display_name),
  }
  if nz(f.initial_password) then fields.password = f.initial_password end
  local _, err = sdk.create(fields)
  if err then
    return { status = 303, headers = { Location = "/auth/users?error=" .. urlenc(tostring(err.status) .. ":create failed") } }
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
