--- @module assay.cron
--- @description Scheduled job inspector. Parses /etc/crontab, /etc/cron.d/*, /etc/cron.{hourly,daily,weekly,monthly}/*, /var/spool/cron/crontabs/*, plus a passthrough to the systemd builtin's timer list.
--- @keywords cron, crontab, schedule, scheduled, jobs, systemd, timer, timers, anacron
--- @quickref M.system_crontab() -> [{source, line, schedule, user, command}, ...] | Parse /etc/crontab + /etc/cron.d/*
--- @quickref M.user_crontabs() -> {user = parsed_crontab, ...} | Parse /var/spool/cron/crontabs/* (root-readable)
--- @quickref M.daily_dropins() -> {hourly={}, daily={}, weekly={}, monthly={}} | Listings of /etc/cron.* dirs
--- @quickref M.timers() -> [...] | Thin wrapper around systemd.list_timers()
--- @quickref M.all() -> [{source, schedule, command, user, next_fire?, last_fire?}, ...] | Unified view across all sources

local M = {}

-- ---------------------------------------------------------------------------
-- Helpers
-- ---------------------------------------------------------------------------

local SHORTHAND = {
  ["@reboot"]   = "@reboot",
  ["@yearly"]   = "0 0 1 1 *",
  ["@annually"] = "0 0 1 1 *",
  ["@monthly"]  = "0 0 1 * *",
  ["@weekly"]   = "0 0 * * 0",
  ["@daily"]    = "0 0 * * *",
  ["@midnight"] = "0 0 * * *",
  ["@hourly"]   = "0 * * * *",
}

local function trim(s)
  return (s or ""):gsub("^%s+", ""):gsub("%s+$", "")
end

local function split_words(s, max)
  local out = {}
  local rest = s
  while rest ~= "" and #out < (max - 1) do
    local i = rest:find("%S")
    if not i then break end
    rest = rest:sub(i)
    local j = rest:find("%s") or (#rest + 1)
    table.insert(out, rest:sub(1, j - 1))
    rest = rest:sub(j)
    rest = trim(rest)
  end
  if rest ~= "" then table.insert(out, rest) end
  return out
end

local function read_file(path)
  local body = fs.read(path)
  if not body then return nil end
  return body
end

local function list_dir(path)
  local ok, entries = pcall(fs.list, path)
  if not ok or not entries then return {} end
  local names = {}
  for _, e in ipairs(entries) do
    if type(e) == "table" then
      table.insert(names, e.name or e.path)
    elseif type(e) == "string" then
      table.insert(names, e)
    end
  end
  return names
end

-- Parse a single crontab line into a record, or nil if it's a comment / blank /
-- environment assignment. `with_user = true` for system-wide crontabs
-- (/etc/crontab, /etc/cron.d/*) where field 6 is the user; false for user
-- crontabs where field 6 is the command.
local function parse_line(line, with_user)
  line = trim(line)
  if line == "" or line:sub(1, 1) == "#" then return nil end

  -- VAR=value style env assignment — skip but don't error
  if line:match("^[A-Za-z_][A-Za-z0-9_]*%s*=") then
    return { kind = "env", line = line }
  end

  -- Shorthand
  local first_word = line:match("^(@%S+)")
  if first_word and SHORTHAND[first_word] then
    local rest = trim(line:sub(#first_word + 1))
    local schedule = SHORTHAND[first_word]
    if with_user then
      local user, cmd = rest:match("^(%S+)%s+(.+)$")
      if not user then return nil end
      return { kind = "entry", schedule = schedule, user = user, command = cmd, raw = line }
    else
      return { kind = "entry", schedule = schedule, user = nil, command = rest, raw = line }
    end
  end

  local fields = with_user and 7 or 6
  local parts = split_words(line, fields)
  if #parts < fields then return nil end
  local schedule = table.concat(parts, " ", 1, 5)
  if with_user then
    return {
      kind = "entry",
      schedule = schedule,
      user = parts[6],
      command = parts[7],
      raw = line,
    }
  end
  return {
    kind = "entry",
    schedule = schedule,
    user = nil,
    command = parts[6],
    raw = line,
  }
end

local function parse_crontab(body, source, with_user)
  local out = {}
  for line in body:gmatch("[^\r\n]+") do
    local rec = parse_line(line, with_user)
    if rec and rec.kind == "entry" then
      rec.source = source
      table.insert(out, rec)
    end
  end
  return out
end

-- ---------------------------------------------------------------------------
-- Public API
-- ---------------------------------------------------------------------------

--- Parse /etc/crontab + every file in /etc/cron.d/*.
--- @return [{source, schedule, user, command, raw}, ...]
function M.system_crontab()
  local entries = {}
  local main = read_file("/etc/crontab")
  if main then
    for _, e in ipairs(parse_crontab(main, "/etc/crontab", true)) do
      table.insert(entries, e)
    end
  end
  for _, name in ipairs(list_dir("/etc/cron.d")) do
    if not name:match("^%.") then
      local path = "/etc/cron.d/" .. name
      local body = read_file(path)
      if body then
        for _, e in ipairs(parse_crontab(body, path, true)) do
          table.insert(entries, e)
        end
      end
    end
  end
  return entries
end

--- Parse user crontabs from /var/spool/cron/crontabs/* (root-readable).
--- @return {user = [{source, schedule, command, raw}, ...], ...}
function M.user_crontabs()
  local out = {}
  for _, name in ipairs(list_dir("/var/spool/cron/crontabs")) do
    if not name:match("^%.") then
      local path = "/var/spool/cron/crontabs/" .. name
      local body = read_file(path)
      if body then
        local entries = parse_crontab(body, path, false)
        for _, e in ipairs(entries) do
          e.user = name
        end
        out[name] = entries
      end
    end
  end
  return out
end

--- List the contents of /etc/cron.{hourly,daily,weekly,monthly}/.
--- @return {hourly={path=...}, daily=..., weekly=..., monthly=...}
function M.daily_dropins()
  local out = {}
  for _, freq in ipairs({ "hourly", "daily", "weekly", "monthly" }) do
    local dir = "/etc/cron." .. freq
    local items = {}
    for _, name in ipairs(list_dir(dir)) do
      if not name:match("^%.") then
        table.insert(items, { name = name, path = dir .. "/" .. name })
      end
    end
    out[freq] = items
  end
  return out
end

--- Thin passthrough to the systemd builtin's timer list. Errors with a
--- helpful hint if the systemd builtin isn't loaded (e.g. running on macOS).
--- @return [{unit, next_elapse_realtime, last_trigger_realtime, passed, activates}, ...]
function M.timers()
  if type(systemd) ~= "table" or type(systemd.list_timers) ~= "function" then
    error("assay.cron.timers: systemd builtin not available (Linux only)")
  end
  return systemd.list_timers()
end

--- Unified view: every crontab entry + every drop-in + every systemd timer,
--- merged into a single list. Each row has a stable shape across sources;
--- next_fire and last_fire are populated only for systemd timers (cron jobs
--- would need a full cron-spec evaluator to compute these reliably).
--- @return [{kind, source, schedule, command, user?, next_fire?, last_fire?, raw?}, ...]
function M.all()
  local rows = {}

  for _, e in ipairs(M.system_crontab()) do
    table.insert(rows, {
      kind = "crontab",
      source = e.source,
      schedule = e.schedule,
      command = e.command,
      user = e.user,
      raw = e.raw,
    })
  end

  for user, entries in pairs(M.user_crontabs()) do
    for _, e in ipairs(entries) do
      table.insert(rows, {
        kind = "user_crontab",
        source = e.source,
        schedule = e.schedule,
        command = e.command,
        user = user,
        raw = e.raw,
      })
    end
  end

  local dropins = M.daily_dropins()
  for freq, items in pairs(dropins) do
    for _, item in ipairs(items) do
      table.insert(rows, {
        kind = "dropin",
        source = item.path,
        schedule = "@" .. freq,
        command = item.path,
        user = "root",
      })
    end
  end

  if type(systemd) == "table" and type(systemd.list_timers) == "function" then
    local ok, timers = pcall(systemd.list_timers)
    if ok and type(timers) == "table" then
      for _, t in ipairs(timers) do
        table.insert(rows, {
          kind = "timer",
          source = t.unit,
          schedule = t.schedule or t.activates or "",
          command = t.activates or "",
          user = nil,
          next_fire = t.next_elapse_realtime or t.next,
          last_fire = t.last_trigger_realtime or t.last,
        })
      end
    end
  end

  return rows
end

return M
