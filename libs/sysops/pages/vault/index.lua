local render = require("pages.render")
local ctx    = require("sysops.ctx")
local vault  = require("sysops.vault")

local M = {}

function M.page(req)
  local sdk = vault.new(ctx.engine)
  local seal, err = sdk.sealing.status()
  local kid_short = ""
  if seal and type(seal.kid) == "string" then
    kid_short = seal.kid:sub(1, 12) .. "…"
  end
  -- Tag the response with `_status` so the template's
  -- `{% if seal._status == 0 or seal._status == nil %}` branch resolves
  -- to the unavailable banner only when the call actually failed.
  if seal and not err then seal._status = 200 end
  return render.render("vault/index", {
    nav_active = "vault:overview",
    title      = "Vault",
    page_title = "Vault",
    seal       = seal or { _status = (err and err.status) or 0 },
    kid_short  = kid_short,
    error      = err,
    status     = err and err.status or 200,
  }, req)
end

return M
