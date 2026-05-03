-- pages/backups_job.lua
--
-- GET /backups/job/:id          — job detail + live log page
-- GET /api/backups/jobs/:id     — JSON status (poll endpoint)

local render = require("pages.render")
local ctx = require("hostops.ctx")
local M = {}

local function path_id(req)
  local p = req and req.path or ""
  return p:match("^/backups/job/([^/?]+)$")
      or p:match("^/api/backups/jobs/([^/?]+)$")
end

function M.detail(req)
  local snap = ctx.state.snapshot()
  local id = path_id(req)
  local job = id and ctx.jobs.get(id) or nil
  return render.render("backups/job", {
    nav_active = "backups",
    host = snap.host, machines = snap.machines,
    job = job,
    job_id = id,
    not_found = job == nil,
  }, req)
end

function M.status(req)
  local id = path_id(req)
  local job = id and ctx.jobs.get(id) or nil
  if not job then
    return {
      status = 404,
      body = json.encode({ error = "no job " .. tostring(id) }),
      headers = { ["Content-Type"] = "application/json" },
    }
  end
  return {
    status = 200,
    body = json.encode(job),
    headers = { ["Content-Type"] = "application/json" },
  }
end

return M
