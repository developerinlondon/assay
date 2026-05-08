local render = require("pages.render")
local ctx    = require("sysops.ctx")
local form   = require("pages.form")
local auth   = require("sysops.auth")

local M = {}

function M.page(req)
  return render.render("zanzibar/check", {
    nav_active  = "zanzibar:check",
    title       = "Check · zanzibar · auth",
    page_title  = "Zanzibar check",
    submitted   = false,
    form        = {},
  }, req)
end

function M.run(req)
  local f   = form.parse(req)
  local sdk = auth.new(ctx.engine).zanzibar
  local body = {
    resource_type = f.resource_type or "",
    resource_id   = f.resource_id   or "",
    permission    = f.permission    or "",
    subject_type  = f.subject_type  or "",
    subject_id    = f.subject_id    or "",
    subject_rel   = f.subject_rel   or "",
  }
  local subject = (f.subject_type or "") .. ":" .. (f.subject_id or "")
  if (f.subject_rel or "") ~= "" then subject = subject .. "#" .. f.subject_rel end
  local object   = (f.resource_type or "") .. ":" .. (f.resource_id or "")
  local relation = f.permission or ""
  local data, err = sdk.check(subject, relation, object)
  local allowed, result_label, err_msg
  if err then
    err_msg = "check failed: status " .. tostring(err.status)
  else
    local rb = (type(data) == "table") and data or {}
    allowed     = rb.allowed and true or false
    result_label = rb.result or (allowed and "Allowed" or "Denied")
  end
  return render.render("zanzibar/check", {
    nav_active  = "zanzibar:check",
    title       = "Check · zanzibar · auth",
    page_title  = "Zanzibar check",
    submitted   = true,
    form        = body,
    allowed     = allowed,
    result      = result_label,
    err         = err_msg,
  }, req)
end

return M
