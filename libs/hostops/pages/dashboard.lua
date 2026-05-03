local render          = require("pages.render")
local state           = require("services.state")
local cron_status     = require("services.cron_status")
local services_status = require("services.services_status")
local cf_status       = require("services.cloudflared_status")
local journal         = require("services.journal")

local M = {}

-- Build the ctx the status_strip partial needs. Each probe is
-- pcall-friendly internally, but we still wrap the calls here so a
-- single misbehaving service can't 500 the whole strip.
local function status_ctx()
  local cron = (pcall(cron_status.summary) and cron_status.summary())   or { timers = 0, crontab_jobs = 0, total = 0 }
  local svc  = (pcall(services_status.summary) and services_status.summary()) or { running = 0, failed = 0, total = 0 }
  local cf   = (pcall(cf_status.summary) and cf_status.summary())       or { state = "unknown" }
  return { cron = cron, services = svc, cloudflared = cf }
end

function M.dashboard(req)
  local snap = state.snapshot()
  local ctx = {
    nav_active = "dashboard",
    host       = snap.host,
    machines   = snap.machines,
  }
  -- Pre-render partials so the page has content on first load.
  ctx.host_strip      = render.fragment("host_strip",      ctx).body
  ctx.machines_grid   = render.fragment("machines_grid",   ctx).body
  ctx.status_strip    = render.fragment("status_strip",    status_ctx()).body
  ctx.recent_activity = render.fragment("recent_activity", { entries = journal.recent(15) }).body
  return render.render("dashboard", ctx, req)
end

function M.host_strip(req)
  local snap = state.snapshot()
  return render.fragment("host_strip", { host = snap.host })
end

function M.machines_grid(req)
  local snap = state.snapshot()
  return render.fragment("machines_grid", { machines = snap.machines })
end

function M.status_strip(_req)
  return render.fragment("status_strip", status_ctx())
end

function M.recent_activity(_req)
  return render.fragment("recent_activity", { entries = journal.recent(15) })
end

return M
