-- /machines — Containers landing page.
-- Lists every nspawn machine as a card with state + a link to its
-- detail page. Machines come from the same cached state.snapshot()
-- that drives the sidebar's CONTAINERS section.

local render = require("pages.render")
local jobs   = require("services.nspawn.jobs")

local ctx = require("sysops.ctx")
return function(req)
  local ok, snap = pcall(ctx.state.snapshot)
  local machines = (ok and snap and snap.machines) or {}
  local active_jobs = jobs.active()

  -- Flash banner from a redirect after provision/destroy.
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

  return render.render("machines/index", {
    nav_active   = "machines",
    page_title   = "Containers",
    machines     = machines,
    total        = #machines,
    flash        = flash,
    active_jobs  = active_jobs,
  }, req)
end
