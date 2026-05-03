-- pages/backups_run.lua
--
-- POST /api/backups/run — start a manual "Run backup now" job.

local backups = require("services.host.backups")

local M = {}

local function actor_from(req)
  local h = (req and req.headers) or {}
  return h["Cf-Access-Authenticated-User-Email"]
      or h["cf-access-authenticated-user-email"]
      or "local-dev"
end

function M.run(req)
  local res = backups.run_now({ actor = actor_from(req) })
  if not res.ok then
    return { status = 400, body = "run failed: " .. (res.error or "?") }
  end
  return { status = 303,
    headers = { ["Location"] = "/backups/job/" .. res.job_id } }
end

return M
