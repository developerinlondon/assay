-- services/host/backup_marker.lua
--
-- Read/write `/var/lib/sysops/backups/<profile>.last` — a simple
-- key=value file the timer-launched `sysops backup-run` writes after
-- every run. Lets the dashboard surface "last run" status without
-- having to grep journalctl or query rustic.
--
-- File shape (see src/backup_run/mod.rs):
--   ts=1730500800
--   exit=0
--   duration_s=492
--   snap_id=9f3a02b1...
--   kind=timer | manual
--   fs_consistency=btrfs | zfs | live

local M = {}

local STATE_DIR = "/var/lib/sysops/backups"

local function path_for(profile)
  return STATE_DIR .. "/" .. profile .. ".last"
end

--- Read marker for `profile`. Returns a table or nil if no run yet.
function M.read(profile)
  local f = io.open(path_for(profile), "r")
  if not f then return nil end
  local body = f:read("*a") or ""
  f:close()
  local out = {}
  for k, v in body:gmatch("([%w_]+)=([^\n]*)") do
    out[k] = v
  end
  if next(out) == nil then return nil end
  -- Coerce numeric fields.
  out.ts = tonumber(out.ts)
  out.exit = tonumber(out.exit)
  out.duration_s = tonumber(out.duration_s)
  return out
end

--- Write marker for a manual run. (The Rust subcommand writes its own
--- for timer-launched runs; this is the Lua-side counterpart used by
--- the run-now job handler.)
function M.write(profile, fields)
  shell.exec("install -d -m 0700 -o root -g root " .. STATE_DIR)
  fields = fields or {}
  fields.ts = fields.ts or os.time()
  fields.kind = fields.kind or "manual"
  fields.fs_consistency = fields.fs_consistency or "live"
  local lines = {}
  for _, k in ipairs({ "ts", "exit", "duration_s", "snap_id", "kind", "fs_consistency" }) do
    if fields[k] ~= nil then
      table.insert(lines, string.format("%s=%s", k, tostring(fields[k])))
    end
  end
  local body = table.concat(lines, "\n") .. "\n"
  -- Atomic-ish: write to tmp then rename.
  local tmp = path_for(profile) .. ".tmp"
  local out = io.open(tmp, "w")
  if not out then return nil, "open tmp failed" end
  out:write(body)
  out:close()
  os.rename(tmp, path_for(profile))
  return true
end

return M
