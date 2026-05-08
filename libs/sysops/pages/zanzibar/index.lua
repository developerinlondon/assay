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
    -- The engine returns `definitions` as a map { relation_name → definition }.
    -- Build a comma-separated list of relation names for the table cell.
    local defs = n.definitions
    local rel_names = {}
    local rel_count = 0
    if type(defs) == "table" then
      for k, _ in pairs(defs) do
        if type(k) == "string" then
          rel_names[#rel_names + 1] = k
          rel_count = rel_count + 1
        end
      end
      table.sort(rel_names)
    end
    namespaces[#namespaces + 1] = {
      name           = n.name,
      relations      = #rel_names > 0 and table.concat(rel_names, ", ") or "—",
      relation_count = rel_count,
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
