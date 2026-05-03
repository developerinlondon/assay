-- pages/backups_restore.lua
--
-- GET  /backups/snapshot/:id   — snapshot detail page
-- POST /api/backups/restore    — start a restore job

local render  = require("pages.render")
local form    = require("pages.form")
local state   = require("services.state")
local backups = require("services.host.backups")

local M = {}

local function actor_from(req)
  local h = (req and req.headers) or {}
  return h["Cf-Access-Authenticated-User-Email"]
      or h["cf-access-authenticated-user-email"]
      or "local-dev"
end

local function path_id(req)
  local p = req and req.path or ""
  return p:match("^/backups/snapshot/([^/?]+)")
end

function M.detail(req)
  local snap = state.snapshot()
  local id = path_id(req)
  if not id then return { status = 400, body = "missing snapshot id" } end

  local detail, err = backups.snapshot_detail(id)
  if not detail then
    return render.render("backups/snapshot", {
      nav_active = "backups",
      host = snap.host, machines = snap.machines,
      snapshot_id = id,
      snapshot_id_short = id:sub(1, 12),
      not_found = true,
      error = err,
    }, req)
  end

  return render.render("backups/snapshot", {
    nav_active = "backups",
    host = snap.host, machines = snap.machines,
    snapshot_id = id,
    snapshot_id_short = id:sub(1, 12),
    detail = detail,
    -- Default destination prefilled in the wizard
    default_dest = "/var/lib/knowhere/restore/" .. os.date("!%Y-%m-%dT%H-%M-%S"),
  }, req)
end

function M.restore(req)
  local f = form.parse(req)
  local snap_id = f.snapshot_id
  local dest = f.dest
  local dest_mode = f.dest_mode  -- "alt" | "original"
  local confirm = f.confirm

  if dest_mode == "original" and confirm ~= snap_id then
    return { status = 400,
      body = "type-to-confirm failed: confirm field must equal snapshot id" }
  end
  if dest_mode == "alt" then
    if not dest or dest == "" then
      dest = "/var/lib/knowhere/restore/" .. os.date("!%Y-%m-%dT%H-%M-%S")
    end
  end

  local res = backups.start_restore({
    actor = actor_from(req),
    snapshot_id = snap_id,
    dest = dest,
  })
  if not res.ok then
    return { status = 400, body = "restore failed: " .. (res.error or "?") }
  end
  return { status = 303,
    headers = { ["Location"] = "/backups/job/" .. res.job_id } }
end

return M
