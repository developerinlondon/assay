local M = {}

-- Counts host-level scheduled work in two buckets:
--   timers       = systemd timer units (systemctl list-timers)
--   crontab_jobs = /etc/crontab + /etc/cron.d/* + /etc/cron.{hourly,daily,
--                  weekly,monthly}/* + per-user crontabs (root-readable)
-- Every probe is pcall-wrapped; missing builtins, missing files, or
-- permission errors degrade silently to 0 so the caller can render a
-- partial dashboard without surfacing a stack trace.

local function count_timers()
  if type(systemd) ~= "table" or type(systemd.list_timers) ~= "function" then
    return 0
  end
  local ok, timers = pcall(systemd.list_timers)
  if not ok or type(timers) ~= "table" then return 0 end
  return #timers
end

local function count_crontab_jobs()
  local ok, cron = pcall(require, "assay.cron")
  if not ok or type(cron) ~= "table" or type(cron.all) ~= "function" then
    return 0
  end
  local ok_all, list = pcall(cron.all)
  if not ok_all or type(list) ~= "table" then return 0 end
  -- cron.all() folds systemd timers into the unified list; strip them so
  -- the two buckets stay disjoint and `total` doesn't double-count.
  local n = 0
  for _, row in ipairs(list) do
    if row.kind ~= "timer" then n = n + 1 end
  end
  return n
end

function M.summary()
  local timers = count_timers()
  local crontab_jobs = count_crontab_jobs()
  return {
    timers = timers,
    crontab_jobs = crontab_jobs,
    total = timers + crontab_jobs,
  }
end

return M
