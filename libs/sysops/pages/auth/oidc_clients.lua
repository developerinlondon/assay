local render = require("pages.render")
local ctx    = require("sysops.ctx")
local auth   = require("sysops.auth")

local M = {}

function M.page(req)
  local q   = (req and req.params) or {}
  local sdk = auth.new(ctx.engine).oidc
  local data, err = sdk.clients()
  local clients = (data and type(data.clients) == "table") and data.clients or {}
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

return M
