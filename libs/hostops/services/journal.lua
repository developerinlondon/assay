local M = {}

-- Strip ANSI escape sequences and other control characters.
local function strip_ctrl(s)
  if type(s) ~= "string" then return "" end
  s = s:gsub("\27%[[%d;]*[A-Za-z]", "")
  s = s:gsub("[%z\1-\8\11\12\14-\31\127]", "")
  return s
end

-- Parse one `--output=short` line:
--   "Mon DD HH:MM:SS hostname unit[pid]: message"
-- Returns nil if the line does not conform.
local function parse_line(line)
  line = strip_ctrl(line)
  if #line < 16 then return nil end
  local ts = line:sub(1, 15)
  -- Validate timestamp shape: 3 letters, space, ?? ??:??:??
  if not ts:match("^%a%a%a [ %d]%d %d%d:%d%d:%d%d$") then return nil end
  local rest = line:sub(17)
  -- Skip hostname token.
  local sp = rest:find(" ", 1, true)
  if not sp then return nil end
  local after_host = rest:sub(sp + 1)
  -- unit name is up to the first '[' or ':'.
  local unit_end = after_host:find("[%[:]")
  if not unit_end then return nil end
  local unit = after_host:sub(1, unit_end - 1)
  local tail = after_host:sub(unit_end)
  -- Drop optional `[pid]`.
  tail = tail:gsub("^%b[]", "", 1)
  -- Expect ": message".
  local msg = tail:match("^:%s*(.*)$") or ""
  return { timestamp = ts, unit = unit, message = msg }
end

local function read_journal(n)
  local cmd = "journalctl -n " .. tostring(n)
              .. " --no-pager --output=short --reverse 2>/dev/null"
  local h = io.popen(cmd)
  if not h then return {} end
  local out = {}
  for line in h:lines() do
    local entry = parse_line(line)
    if entry then out[#out + 1] = entry end
  end
  h:close()
  return out
end

function M.recent(n)
  n = tonumber(n) or 20
  if n < 1 then n = 1 end
  if n > 100 then n = 100 end
  local ok, result = pcall(read_journal, n)
  if not ok or type(result) ~= "table" then return {} end
  return result
end

return M
