-- services/host/backup_schedule.lua
--
-- Manage the systemd timer that runs `sysops backup-run host` daily.
-- Reads/writes /etc/systemd/system/sysops-backup-host.{service,timer}
-- via shell.exec (sudo when not root). Idempotent.

local M = {}

local SERVICE_PATH = "/etc/systemd/system/sysops-backup-host.service"
local TIMER_PATH   = "/etc/systemd/system/sysops-backup-host.timer"
local TIMER_UNIT   = "sysops-backup-host.timer"
local SERVICE_UNIT = "sysops-backup-host.service"

local function service_body(binary_path, profile)
  return string.format([[
[Unit]
Description=sysops daily backup (profile %s)
After=network-online.target
Wants=network-online.target

[Service]
Type=oneshot
ExecStart=%s backup-run %s
StandardOutput=journal
StandardError=journal
Nice=10
IOSchedulingClass=best-effort
IOSchedulingPriority=7
]], profile, binary_path, profile)
end

local function timer_body(hour, jitter_s, persistent)
  local minute = "00"
  return string.format([[
[Unit]
Description=sysops daily backup timer (profile host)

[Timer]
OnCalendar=*-*-* %02d:%s:00
RandomizedDelaySec=%d
Persistent=%s
Unit=%s

[Install]
WantedBy=timers.target
]], hour, minute, jitter_s, persistent and "true" or "false", SERVICE_UNIT)
end

local function detect_binary()
  local r = shell.exec("readlink -f /proc/self/exe")
  if r and r.status == 0 and r.stdout then
    local p = (r.stdout):gsub("[\r\n]+$", "")
    if p ~= "" then return p end
  end
  return "/usr/local/bin/sysops"
end

local function write_unit_file(path, body)
  local sentinel = "EOF_KW_" .. tostring(os.time()) .. "_" .. tostring(math.random(1, 1e9))
  local script = string.format("cat > %s << '%s'\n%s%s\n",
    path, sentinel, body, sentinel)
  local r = shell.exec("sudo -n bash -c " .. string.format("%q", script))
  if not r or r.status ~= 0 then
    return nil, "write " .. path .. " failed: " .. ((r and r.stderr) or "?")
  end
  return true
end

local function daemon_reload()
  local r = shell.exec("sudo -n systemctl daemon-reload")
  return r and r.status == 0
end

--- Write both unit files for the given profile (default "host") and
--- schedule. `args` = { profile = "host", hour = 2, jitter_s = 1800,
--- persistent = true, binary_path = nil-->autodetect }.
function M.write_timer(args)
  args = args or {}
  local profile     = args.profile or "host"
  local hour        = tonumber(args.hour) or 2
  local jitter_s    = tonumber(args.jitter_s) or 1800
  local persistent  = args.persistent
  if persistent == nil then persistent = true end
  local binary_path = args.binary_path or detect_binary()

  if hour < 0 or hour > 23 then return nil, "hour out of range: " .. hour end
  if jitter_s < 0 or jitter_s > 86400 then return nil, "jitter out of range" end

  local ok, err = write_unit_file(SERVICE_PATH, service_body(binary_path, profile))
  if not ok then return nil, err end
  ok, err = write_unit_file(TIMER_PATH, timer_body(hour, jitter_s, persistent))
  if not ok then return nil, err end
  if not daemon_reload() then return nil, "systemctl daemon-reload failed" end
  return true
end

--- Enable + start the timer.
function M.enable()
  local r = shell.exec("sudo -n systemctl enable --now " .. TIMER_UNIT)
  if not r or r.status ~= 0 then
    return nil, "enable failed: " .. ((r and r.stderr) or "?")
  end
  return true
end

--- Stop + disable the timer.
function M.disable()
  local r = shell.exec("sudo -n systemctl disable --now " .. TIMER_UNIT)
  if not r or r.status ~= 0 then
    return nil, "disable failed: " .. ((r and r.stderr) or "?")
  end
  return true
end

--- Remove the unit files.
function M.remove()
  shell.exec("sudo -n systemctl disable --now " .. TIMER_UNIT)
  shell.exec("sudo -n rm -f " .. SERVICE_PATH .. " " .. TIMER_PATH)
  daemon_reload()
  return true
end

--- Status: enabled? active? next/last fire times. Read from
--- `systemctl list-timers` + `is-active` + `is-enabled`.
function M.status()
  local out = { enabled = false, active = false, next_fire = nil, last_fire = nil }

  local r1 = shell.exec("systemctl is-enabled " .. TIMER_UNIT .. " 2>/dev/null")
  out.enabled = r1 and r1.status == 0 and (r1.stdout or ""):find("enabled", 1, true) ~= nil

  local r2 = shell.exec("systemctl is-active " .. TIMER_UNIT .. " 2>/dev/null")
  out.active = r2 and r2.status == 0

  -- Parse list-timers for next/last
  local r3 = shell.exec("systemctl list-timers --no-legend --no-pager " .. TIMER_UNIT .. " 2>/dev/null")
  if r3 and r3.status == 0 and r3.stdout then
    -- Output columns: NEXT LEFT LAST PASSED UNIT ACTIVATES
    -- Take the first non-empty line.
    for line in r3.stdout:gmatch("[^\n]+") do
      local stripped = line:gsub("^%s+", ""):gsub("%s+$", "")
      if stripped ~= "" then
        out.list_timers_line = stripped
        break
      end
    end
  end

  -- Last fire from the marker file, if present.
  local marker = require("services.host.backup_marker")
  local m = marker.read("host")
  if m then
    out.last_fire_ts = m.ts
    out.last_exit    = m.exit
    out.last_duration_s = m.duration_s
    out.last_snap_id    = m.snap_id
    out.last_fs_consistency = m.fs_consistency
  end

  return out
end

--- Read the current hour the timer is configured for. Returns nil if
--- the timer file isn't installed or can't be parsed.
function M.read_hour()
  local f = io.open(TIMER_PATH, "r")
  if not f then return nil end
  local body = f:read("*a") or ""
  f:close()
  local hh = body:match("OnCalendar=%*%-%*%-%* (%d%d):")
  if hh then return tonumber(hh) end
  return nil
end

return M
