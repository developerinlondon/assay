-- pages/backups.lua
--
-- Plan 15 — host Backups tab. State-driven page: show wizard, dashboard,
-- or repair UI based on what's actually configured on disk + vault state.

local render  = require("pages.render")
local backups = require("services.host.backups")
local schedule = require("services.host.backup_schedule")
local marker   = require("services.host.backup_marker")
local fs_snap  = require("assay.fs_snapshot")
local sysops_ctx = require("sysops.ctx")
local M = {}

local function recent_audit_for(prefix)
  local out = {}
  local recent = sysops_ctx.audit.recent and sysops_ctx.audit.recent(50) or {}
  for _, e in ipairs(recent) do
    if type(e.action) == "string" and e.action:sub(1, #prefix) == prefix then
      table.insert(out, e)
      if #out >= 5 then break end
    end
  end
  return out
end

local function detect_for_sources(sources)
  -- Best effort: detect FS backend for the first source path. Most
  -- v1 hosts will only have one source root they care about
  -- (`/var/lib/machines`); the wizard / sources editor surfaces this.
  if not sources or #sources == 0 then return { backend = "none" } end
  local target = "/var/lib/machines"
  for _, s in ipairs(sources) do
    if s == target then target = s; break end
  end
  local ok, det = pcall(fs_snap.detect, target)
  if not ok or type(det) ~= "table" then return { backend = "none" } end
  return det
end

function M.page(req)
  local snap = sysops_ctx.state.snapshot()
  local s = backups.state()

  local ctx = {
    nav_active = "backups",
    host       = snap.host,
    machines   = snap.machines,
    page_state = s,
  }

  if s == "B" or s == "B-blk" then
    -- Setup wizard.
    local va_ok, va = pcall(require, "services.vault_admin")
    local vault_loaded = false
    local vault_sealed = false
    if va_ok and va and type(va.status) == "function" then
      local ok2, st = pcall(va.status)
      if ok2 and type(st) == "table" then
        vault_loaded = st.loaded == true
        vault_sealed = st.sealed == true
      end
    end
    ctx.vault_loaded = vault_loaded
    ctx.vault_sealed = vault_sealed
    ctx.default_sources = {
      "/etc",
      "/root",
      "/etc/systemd/nspawn",
      "/var/lib/machines",
    }
    return render.render("backups/index", ctx, req)
  end

  -- Configured state: load profile + last-run + snapshots + active jobs.
  local profile = backups.read_profile()
  ctx.profile = profile
  ctx.repository_url = profile and profile.repository and profile.repository.url
  ctx.region = profile and profile.repository and profile.repository.region
  ctx.sources = (profile and profile.backup and profile.backup.sources) or {}
  ctx.fs_backend = detect_for_sources(ctx.sources)
  ctx.schedule_status = schedule.status()
  ctx.schedule_hour = schedule.read_hour() or 2
  ctx.last_run = marker.read("host")
  if ctx.last_run and ctx.last_run.snap_id then
    ctx.last_run.snap_id_short = ctx.last_run.snap_id:sub(1, 8)
  end
  ctx.audit_recent = recent_audit_for("backups.")

  if s == "C" then
    local snaps_or_err, err = backups.list_snapshots()
    if snaps_or_err and not err then
      ctx.snapshots = snaps_or_err
      ctx.snap_count = #snaps_or_err
      -- limit to last 10 for the dashboard table; add id_short
      local trimmed = {}
      for i = 1, math.min(10, #snaps_or_err) do
        local s = snaps_or_err[i]
        if type(s) == "table" and s.id then
          s.id_short = tostring(s.id):sub(1, 10)
        end
        trimmed[i] = s
      end
      ctx.snapshots_recent = trimmed
    else
      ctx.snapshots_error = err or "rustic.snapshots returned nil"
      ctx.snapshots = {}
      ctx.snap_count = 0
      ctx.snapshots_recent = {}
    end
  else
    ctx.snapshots = {}
    ctx.snap_count = 0
    ctx.snapshots_recent = {}
  end

  -- Active jobs (run-now or restore in flight)
  local active = sysops_ctx.jobs.active({ kind = "backups.run_now" })
  for _, j in ipairs(sysops_ctx.jobs.active({ kind = "backups.restore" })) do
    table.insert(active, j)
  end
  ctx.active_jobs = active
  if #active > 0 then ctx.page_state = "C-run" end

  return render.render("backups/index", ctx, req)
end

return M
