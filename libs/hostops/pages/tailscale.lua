local render = require("pages.render")
local state  = require("services.state")

local M = {}

local function sh_json(cmd)
  local ok, r = pcall(shell.exec, cmd)
  if not ok or not r or r.status ~= 0 then return nil end
  local ok2, parsed = pcall(json.parse, r.stdout or "")
  if ok2 then return parsed end
  return nil
end

local function parse_ts(raw)
  if not raw then return nil end

  local self_node = raw.Self or {}
  local ts_self = {
    hostname    = self_node.HostName,
    dns_name    = self_node.DNSName,
    os          = self_node.OS,
    tailnet_ips = self_node.TailscaleIPs or {},
    relay       = self_node.Relay,
    online      = self_node.Online,
    tags        = self_node.Tags or {},
  }

  local peers = {}
  local peer_map = raw.Peer or {}
  for _, p in pairs(peer_map) do
    peers[#peers + 1] = {
      hostname    = p.HostName,
      dns_name    = p.DNSName,
      os          = p.OS,
      tailnet_ips = p.TailscaleIPs or {},
      relay       = p.Relay,
      online      = p.Online,
      last_seen   = p.LastSeen,
      tags        = p.Tags or {},
    }
  end

  table.sort(peers, function(a, b)
    local ao = a.online and 1 or 0
    local bo = b.online and 1 or 0
    if ao ~= bo then return ao > bo end
    return (a.hostname or "") < (b.hostname or "")
  end)

  local tag_set = {}
  for _, tag in ipairs(ts_self.tags) do tag_set[tag] = true end
  for _, p in ipairs(peers) do
    for _, tag in ipairs(p.tags or {}) do tag_set[tag] = true end
  end
  local tags = {}
  for t in pairs(tag_set) do tags[#tags + 1] = t end
  table.sort(tags)

  return {
    self          = ts_self,
    peers         = peers,
    magic_dns     = raw.MagicDNSSuffix,
    backend_state = raw.BackendState,
    version       = raw.Version,
    tags          = tags,
  }
end

function M.page(req)
  local snap = state.snapshot()

  local raw = sh_json("tailscale status --json")
  local ts  = parse_ts(raw)

  return render.render("tailscale", {
    nav_active = "tailscale",
    host       = snap.host,
    machines   = snap.machines,
    ts         = ts,
  }, req)
end

return M
