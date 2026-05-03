local render = require("pages.render")
local state  = require("services.state")
local cron   = require("assay.cron")

local M = {}

local function pcall_or_empty(fn, ...)
  local ok, result = pcall(fn, ...)
  if ok and type(result) == "table" then return result end
  return {}
end

local function count_table_keys(t)
  if type(t) ~= "table" then return 0 end
  local n = 0
  for _ in pairs(t) do n = n + 1 end
  return n
end

local function fmt_relative(us)
  if not us or us == 0 then return "—" end
  local now_us = os.time() * 1000000
  local diff_s = math.floor((us - now_us) / 1000000)
  local abs_s  = math.abs(diff_s)

  local function parts(secs)
    if secs < 60    then return secs .. "s" end
    if secs < 3600  then return math.floor(secs / 60) .. "m " .. (secs % 60) .. "s" end
    local h = math.floor(secs / 3600)
    local m = math.floor((secs % 3600) / 60)
    return h .. "h " .. m .. "m"
  end

  if diff_s > 0 then
    return "in " .. parts(abs_s)
  else
    return parts(abs_s) .. " ago"
  end
end

function M.page(req)
  local snap = state.snapshot()
  local q    = req.params or {}
  local tab  = q.tab or "timers"

  local timers        = pcall_or_empty(systemd.list_timers)
  local sys_cron      = pcall_or_empty(cron.system_crontab)
  local user_crontabs = pcall_or_empty(cron.user_crontabs)
  local dropins       = pcall_or_empty(cron.daily_dropins)

  -- Annotate timers with human-readable times
  for _, t in ipairs(timers) do
    t.next_fire_pretty = fmt_relative(t.next_elapse_realtime)
    t.last_fire_pretty = fmt_relative(t.last_trigger_realtime)
  end

  local dropin_count = 0
  for _, freq in ipairs({"hourly", "daily", "weekly", "monthly"}) do
    local lst = dropins[freq]
    if type(lst) == "table" then dropin_count = dropin_count + #lst end
  end

  return render.render("cron", {
    nav_active    = "cron",
    host          = snap.host,
    machines      = snap.machines,
    tab           = tab,
    timers        = timers,
    sys_cron      = sys_cron,
    user_crontabs = user_crontabs,
    dropins       = dropins,
    counts = {
      timers   = #timers,
      crond    = #sys_cron,
      dropins  = dropin_count,
      usercron = count_table_keys(user_crontabs),
    },
  }, req)
end

return M
