local render = require("pages.render")
local ctx    = require("sysops.ctx")
local form   = require("pages.form")
local auth   = require("sysops.auth")

local M = {}

local function urlenc(s)
  return (tostring(s or "")):gsub("([^%w%-_%.~])", function(c)
    return string.format("%%%02X", string.byte(c))
  end)
end

local function tuple_body(f)
  return {
    object_type  = f.object_type  or "",
    object_id    = f.object_id    or "",
    relation     = f.relation     or "",
    subject_type = f.subject_type or "",
    subject_id   = f.subject_id   or "",
    subject_rel  = f.subject_rel  or "",
  }
end

function M.page(req)
  local q      = (req and req.params) or {}
  local sdk    = auth.new(ctx.engine).zanzibar
  local data, err = sdk.tuples()
  local tuples = {}
  local unsupported = false
  local status = err and err.status or 200
  if err and (err.status == 404 or err.status == 405 or err.status == 501) then
    unsupported = true
  elseif data and type(data.items) == "table" then
    tuples = data.items
  elseif data and type(data.tuples) == "table" then
    tuples = data.tuples
  end
  local filter = { limit = (data and data.limit) or 100, offset = (data and data.offset) or 0 }
  return render.render("zanzibar/tuples", {
    nav_active   = "zanzibar:tuples",
    title        = "Tuples · zanzibar · auth",
    page_title   = "Zanzibar tuples",
    tuples       = tuples,
    filter       = filter,
    unsupported  = unsupported,
    error        = (not unsupported) and err or nil,
    status       = status,
    saved        = q.saved == "1" and true or nil,
    deleted      = q.deleted == "1" and true or nil,
    write_err    = q.write_err or nil,
    delete_err   = q.delete_err or nil,
  }, req)
end

function M.write(req)
  local f   = form.parse(req)
  local sdk = auth.new(ctx.engine).zanzibar
  local _, err = sdk.write_tuple(tuple_body(f))
  if err then
    return {
      status  = 303,
      headers = {
        Location = "/zanzibar/tuples"
          .. "?write_err=" .. tostring(err.status)
          .. "&form_object_type=" .. urlenc(f.object_type or "")
          .. "&form_object_id=" .. urlenc(f.object_id or "")
          .. "&form_relation=" .. urlenc(f.relation or "")
          .. "&form_subject_type=" .. urlenc(f.subject_type or "")
          .. "&form_subject_id=" .. urlenc(f.subject_id or "")
          .. "&form_subject_rel=" .. urlenc(f.subject_rel or ""),
      },
    }
  end
  return { status = 303, headers = { Location = "/zanzibar/tuples?saved=1" } }
end

function M.delete(req)
  local f   = form.parse(req)
  local sdk = auth.new(ctx.engine).zanzibar
  local _, err = sdk.delete_tuple(tuple_body(f))
  if err then
    return { status = 303, headers = { Location = "/zanzibar/tuples?delete_err=" .. tostring(err.status) } }
  end
  return { status = 303, headers = { Location = "/zanzibar/tuples?deleted=1" } }
end

return M
