local p = require("api.packages")
local M = {}
M.packages_catalog       = p.catalog
M.packages_templates     = p.templates
M.packages_state         = function(req)
  if req.method == "POST" then return p.mutate_state(req) else return p.get_state(req) end
end
M.packages_reconcile     = p.reconcile
M.packages_check_updates = p.check_updates
M.packages_update_all    = p.update_all
return M
