local M = {}

-- Counts host systemd .service units in three buckets:
--   running = ActiveState == "active"
--   failed  = ActiveState == "failed"
--   total   = LoadState  == "loaded" (every unit systemd has actually parsed)
-- One D-Bus round-trip via the systemd builtin's list_units() — no subprocess.
-- Every probe is pcall-wrapped; missing builtin, D-Bus errors, or unexpected
-- shapes degrade silently to zero so the dashboard never 500s here.

local ZERO = { running = 0, failed = 0, total = 0 }

local function tally(units)
  local running, failed, total = 0, 0, 0
  for _, u in ipairs(units) do
    if u.load == "loaded" then total = total + 1 end
    if u.active == "active" then
      running = running + 1
    elseif u.active == "failed" then
      failed = failed + 1
    end
  end
  return { running = running, failed = failed, total = total }
end

function M.summary()
  if type(systemd) ~= "table" or type(systemd.list_units) ~= "function" then
    return ZERO
  end
  local ok, units = pcall(systemd.list_units, "*.service")
  if not ok or type(units) ~= "table" then return ZERO end
  local ok_t, out = pcall(tally, units)
  if not ok_t or type(out) ~= "table" then return ZERO end
  return out
end

return M
