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

function M.page(req)
  local q = (req and req.params) or {}
  return render.render("zanzibar/define", {
    nav_active   = "zanzibar:define",
    title        = "Define namespace · zanzibar · auth",
    page_title   = "Define namespace",
    schema_text  = q.form_schema or "",
    saved_name   = q.saved or nil,
    parse_err    = q.parse_err or nil,
    submit_err   = q.submit_err or nil,
    submit_body  = q.submit_body or nil,
  }, req)
end

function M.submit(req)
  local f      = form.parse(req)
  local schema = f.schema or ""
  local ok, parsed = pcall(json.decode, schema)
  if not ok or type(parsed) ~= "table" then
    return {
      status  = 303,
      headers = { Location = "/zanzibar/define?parse_err=1&form_schema=" .. urlenc(schema) },
    }
  end
  local sdk = auth.new(ctx.engine).zanzibar
  local _, err = sdk.define_namespace(parsed)
  if err then
    local body_snippet = err.body and tostring(err.body):sub(1, 200) or ""
    return {
      status  = 303,
      headers = {
        Location = "/zanzibar/define?submit_err=" .. tostring(err.status)
          .. "&submit_body=" .. urlenc(body_snippet)
          .. "&form_schema=" .. urlenc(schema),
      },
    }
  end
  return {
    status  = 303,
    headers = { Location = "/zanzibar/define?saved=" .. urlenc(parsed.name or "") },
  }
end

return M
