local render = require("pages.render")
local ctx    = require("sysops.ctx")
local auth   = require("sysops.auth")

local M = {}

function M.page(req)
  local sdk = auth.new(ctx.engine).zanzibar
  local data, err = sdk.namespaces()
  local namespaces = {}
  local raw_ns = (data and type(data.items) == "table") and data.items
                 or (data and type(data.namespaces) == "table") and data.namespaces
                 or (type(data) == "table" and data[1] ~= nil) and data
                 or {}
  for _, n in ipairs(raw_ns) do
    local rels = n.relations
    if type(rels) == "table" then
      rels = table.concat(rels, ", ")
    end
    namespaces[#namespaces + 1] = {
      name           = n.name,
      relations      = rels,
      relation_count = n.relation_count or (type(n.relations) == "table" and #n.relations or 0),
    }
  end

  local status = err and err.status or 200
  return render.render("zanzibar/index", {
    nav_active   = "zanzibar:index",
    title        = "Zanzibar · auth",
    page_title   = "Zanzibar",
    namespaces   = namespaces,
    error        = err,
    status       = status,
  }, req)
end

return M
