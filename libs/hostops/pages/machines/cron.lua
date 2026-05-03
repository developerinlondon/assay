-- /machines/<name>/cron — timers inside the nspawn container.
-- Source: shell-out to `systemctl --machine=<name> list-timers --output=json`.
-- Tracked upstream at developerinlondon/assay#112; swap to direct binding once
-- list_timers gains a `machine` opt.
-- v1 covers timers only. /etc/cron.d, drop-ins, user crontabs deferred.

local render = require("pages.render")
local priv   = require("services.host.privilege")

local ctx = require("hostops.ctx")
local M = {}

local function find_machine(snap, name)
  for _, m in ipairs(snap.machines) do
    if m.name == name then return m end
  end
  return nil
end

local function fmt_relative_iso(s)
  -- systemctl list-timers --output=json gives ISO strings or "n/a".
  if not s or s == "" or s == "n/a" then return "—" end
  return s
end

local function valid_machine_name(s)
  return type(s) == "string" and #s > 0 and #s <= 64 and s:match("^[%w_%-%.]+$") ~= nil
end

-- The --machine transport requires host-root. priv.elevated_prefix is
-- "" when knowhere runs as root and "sudo -n " otherwise (see
-- services/host/privilege.lua and deploy/knowhere-machinectl.sudoers.example).
local function fetch_timers(name)
  if not valid_machine_name(name) then return {}, "invalid machine name" end
  local cmd = priv.elevated_prefix .. "systemctl --machine=" .. name
    .. " list-timers --all --output=json --no-pager"
  local ok, r = pcall(shell.exec, cmd)
  if not ok or not r or r.status ~= 0 then
    return {}, (r and r.stderr) or "shell.exec failed"
  end
  local ok2, parsed = pcall(json.parse, r.stdout or "[]")
  if not ok2 or type(parsed) ~= "table" then return {}, "json parse failed" end
  return parsed, nil
end

function M.page(req)
  local name = (req.path or ""):match("^/machines/([^/]+)/cron$")
  if not name then return { status = 404, body = "not found" } end

  local snap = ctx.state.snapshot()
  local machine = find_machine(snap, name)
  if not machine then return { status = 404, body = "machine not found: " .. name } end

  local raw, err = fetch_timers(name)
  local timers = {}
  for _, t in ipairs(raw) do
    timers[#timers + 1] = {
      unit             = t.unit,
      activates        = t.activates,
      next_fire_pretty = fmt_relative_iso(t.next),
      last_fire_pretty = fmt_relative_iso(t.left or t.last),
    }
  end

  return render.render("machines/cron", {
    nav_active  = "machine:" .. name,
    page_title  = name .. " — cron",
    machine_tab = "cron",
    host        = snap.host,
    machines    = snap.machines,
    machine     = machine,
    timers      = timers,
    counts      = { timers = #timers },
    fetch_error = err,
  }, req)
end

return M
