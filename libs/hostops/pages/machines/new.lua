-- Provision-new-machine page.
--
--   GET  /machines/new            renders the form
--   POST /api/machines  (separate handler)  → calls provision service,
--                                              redirects back here on
--                                              error or to /machines/<name>
--                                              on success.

local render = require("pages.render")
local pkgs   = require("services.host.packages")

local M = {}

function M.page(req)
  local catalog  = pkgs.catalog()
  local tpls     = pkgs.templates(catalog.entries)
  -- Provisioning templates: those with a [template.rootfs] section. We
  -- explicitly filter out packages-only templates because they can't drive
  -- a provision run (they'd error out).
  local options = {}
  for _, t in pairs(tpls.entries) do
    if t.rootfs and t.nspawn then
      local r = t.resources or {}
      options[#options+1] = {
        id = t.id, display_name = t.display_name,
        description = t.description,
        packages = t.packages or {},
        resources = {
          cpu_cores = r.cpu_cores,
          memory_gb = r.memory_gb,
        },
      }
    end
  end
  table.sort(options, function(a, b) return a.id < b.id end)

  -- Flash banner from a redirect after attempted provision.
  local q = (req and req.params) or {}
  local flash
  if q.msg and q.msg ~= "" then
    local kind = q.kind or "info"
    local pill_for = {
      ok = "pill-ok", info = "pill-info",
      warn = "pill-warn", err = "pill-err",
    }
    local label_for = { ok = "●", info = "●", warn = "⚠", err = "✗" }
    flash = {
      kind = kind, msg = q.msg,
      pill = pill_for[kind] or "pill-info",
      label = label_for[kind] or "●",
    }
  end

  return render.render("machines/new", {
    nav_active = "machines",
    page_title = "Provision new machine",
    templates  = options,
    flash      = flash,
  }, req)
end

return M
