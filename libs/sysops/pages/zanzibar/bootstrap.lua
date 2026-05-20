local render = require("pages.render")
local ctx    = require("sysops.ctx")
local form   = require("pages.form")
local auth   = require("sysops.auth")
local authz  = require("sysops.authz")

local M = {}

local function url(p)
  return (ctx.url and ctx.url(p)) or p
end

local function urlenc(s)
  return (tostring(s or "")):gsub("([^%w%-_%.~])", function(c)
    return string.format("%%%02X", string.byte(c))
  end)
end

-- Canonical admin tuples come from sysops.authz so this manual
-- bootstrap path and the first-OIDC-user path (pages/auth/bootstrap.lua)
-- always grant the same five resources.
local BOOTSTRAP_TUPLES = authz.CANONICAL_TUPLES

local function tuple_label(t)
  return t.object_type .. ":" .. t.object_id .. "#" .. t.relation
end

function M.page(req)
  local q     = (req and req.params) or {}
  local sdk   = auth.new(ctx.engine)
  local data, err = sdk.users.list({ limit = 200 })
  local users = {}
  if data and type(data.items) == "table" then
    users = data.items
  end

  local rows = {}
  for _, t in ipairs(BOOTSTRAP_TUPLES) do
    rows[#rows + 1] = { label = tuple_label(t) }
  end

  return render.render("zanzibar/bootstrap", {
    nav_active   = "zanzibar:bootstrap",
    title        = "Bootstrap admin · zanzibar · auth",
    page_title   = "Bootstrap admin",
    users        = users,
    users_err    = err,
    tuples       = rows,
    granted_for  = q.granted_for or nil,
    written      = tonumber(q.written or "") or nil,
    failed       = tonumber(q.failed or "") or nil,
  }, req)
end

function M.grant(req)
  local f       = form.parse(req)
  local user_id = f.user_id or ""
  if user_id == "" then
    return {
      status  = 303,
      headers = { Location = url("/zanzibar/bootstrap") .. "?written=0&failed=0" },
    }
  end
  local sdk     = auth.new(ctx.engine).zanzibar
  authz.ensure_required_namespaces(sdk)
  local written = 0
  local failed  = 0
  for _, t in ipairs(BOOTSTRAP_TUPLES) do
    local body = {
      object_type  = t.object_type,
      object_id    = t.object_id,
      relation     = t.relation,
      subject_type = "user",
      subject_id   = user_id,
      subject_rel  = "",
    }
    local _, err = sdk.write_tuple(body)
    if err then
      failed = failed + 1
    else
      written = written + 1
    end
  end
  -- Invalidate sysops.authz's per-(sub, tuple) cache so the granted
  -- user's new permissions take effect immediately (not after the 30s
  -- TTL). pcall keeps the path safe when the auth gateway isn't wired.
  if written > 0 then
    local ok, authz = pcall(require, "sysops.authz")
    if ok and authz and authz.invalidate then authz.invalidate() end
  end
  return {
    status  = 303,
    headers = {
      Location = url("/zanzibar/bootstrap") .. "?granted_for=" .. urlenc(user_id)
        .. "&written=" .. tostring(written)
        .. "&failed=" .. tostring(failed),
    },
  }
end

return M
