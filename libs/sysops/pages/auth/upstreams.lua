local render = require("pages.render")
local ctx    = require("sysops.ctx")
local auth   = require("sysops.auth")

local M = {}

function M.page(req)
  local q   = (req and req.params) or {}
  local sdk = auth.new(ctx.engine).oidc
  local data, err = sdk.upstreams()
  local upstreams = (data and type(data.upstreams) == "table") and data.upstreams or {}
  return render.render("auth/upstreams", {
    nav_active  = "auth:upstreams",
    title       = "Upstreams · auth",
    page_title  = "Upstreams",
    upstreams   = upstreams,
    error       = err,
    status      = err and err.status or 200,
    error_msg   = q.error and q.error or nil,
    ok_msg      = q.ok    and q.ok    or nil,
  }, req)
end

return M
