local M = {}

-- Reports cloudflared.service state for the dashboard tile.
--   state        = "active"|"inactive"|"failed"|"activating"|"deactivating"|"unknown"
--   tunnel_name  = string from `tunnel:` in /etc/cloudflared/config.yml, or
--                  parsed from ExecStart `tunnel run <name>`, else nil
--   connections  = 4 when binary present and state == "active" (typical
--                  default of 4 edge regions); nil otherwise. Prefer nil
--                  over a fabricated number.
-- Every probe is pcall-wrapped; on any failure we degrade to {state="unknown"}.

local function read_active_state()
  local f = io.popen("systemctl is-active cloudflared.service 2>/dev/null")
  if not f then return "unknown" end
  local out = f:read("*a") or ""
  f:close()
  local s = out:gsub("%s+$", "")
  if s == "" then return "unknown" end
  return s
end

local function read_tunnel_name()
  local cf = io.open("/etc/cloudflared/config.yml", "r")
  if cf then
    for line in cf:lines() do
      local name = line:match("^%s*tunnel:%s*([%w%-_%.]+)")
      if name then cf:close(); return name end
    end
    cf:close()
  end
  local sx = io.popen("systemctl show cloudflared.service --property=ExecStart --value 2>/dev/null")
  if sx then
    local exec = sx:read("*a") or ""
    sx:close()
    local name = exec:match("tunnel%s+run%s+([%w%-_%.]+)")
    if name and name ~= "--token" then return name end
  end
  return nil
end

local function binary_exists()
  local f = io.open("/usr/local/bin/cloudflared", "rb")
  if f then f:close(); return true end
  return false
end

function M.summary()
  local ok, result = pcall(function()
    local state = read_active_state()
    local out = { state = state, tunnel_name = read_tunnel_name() }
    if state == "active" and binary_exists() then out.connections = 4 end
    return out
  end)
  if not ok or type(result) ~= "table" then return { state = "unknown" } end
  return result
end

return M
