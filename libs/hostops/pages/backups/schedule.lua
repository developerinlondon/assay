-- pages/backups_schedule.lua
--
-- GET  /backups/schedule       — schedule editor
-- POST /api/backups/schedule   — update timer

local render   = require("pages.render")
local form     = require("pages.form")
local state    = require("services.state")
local backups  = require("services.host.backups")
local schedule = require("services.host.backup_schedule")

local M = {}

local function actor_from(req)
  local h = (req and req.headers) or {}
  return h["Cf-Access-Authenticated-User-Email"]
      or h["cf-access-authenticated-user-email"]
      or "local-dev"
end

function M.editor(req)
  local snap = state.snapshot()
  if not backups.read_profile() then
    return { status = 303, headers = { ["Location"] = "/backups" } }
  end
  return render.render("backups/schedule", {
    nav_active = "backups",
    host = snap.host,
    machines = snap.machines,
    schedule_hour = schedule.read_hour() or 2,
    schedule_status = schedule.status(),
  }, req)
end

function M.update(req)
  local f = form.parse(req)
  local res = backups.update_schedule({
    actor = actor_from(req),
    enabled = f.enabled ~= "off",
    hour = tonumber(f.hour),
    jitter_s = tonumber(f.jitter_s),
  })
  if not res.ok then
    return { status = 400, body = "schedule update failed: " .. (res.error or "?") }
  end
  return { status = 303, headers = { ["Location"] = "/backups?schedule=ok" } }
end

return M
