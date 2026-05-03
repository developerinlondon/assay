-- Detect whether knowhere is running as root, and expose a command
-- prefix for shell-outs that need host-root (e.g. `systemctl --machine`).
--
-- Two supported deploys:
--   - root:           prefix = ""           (direct call)
--   - low-priv user:  prefix = "sudo -n "   (NOPASSWD allowlist required;
--                                            see deploy/knowhere-machinectl.sudoers.example)
--
-- Detection runs once at module load via /proc/self/status.

local M = {}

local function detect_euid()
  local f = io.open("/proc/self/status", "r")
  if not f then return nil end
  local euid
  for line in f:lines() do
    -- Uid: <real> <effective> <saved> <fs>
    local _, eff = line:match("^Uid:%s+(%d+)%s+(%d+)")
    if eff then euid = tonumber(eff); break end
  end
  f:close()
  return euid
end

local euid = detect_euid()
M.euid = euid
M.is_root = euid == 0
M.elevated_prefix = M.is_root and "" or "sudo -n "

return M
