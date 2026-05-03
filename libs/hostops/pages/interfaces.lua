local render = require("pages.render")
local ctx = require("hostops.ctx")
local M = {}

local SKIP_PREFIXES = { "docker", "cni", "veth", "flannel", "kube-", "nodelocal" }

local function sh(cmd)
  local ok, r = pcall(shell.exec, cmd)
  if ok and r and r.status == 0 then return r.stdout or "" end
  return nil
end

local function json_parse_safe(s)
  if not s or s == "" then return nil end
  local ok, t = pcall(json.parse, s)
  if ok then return t end
  return nil
end

-- json.parse stores `local` (a Lua reserved word) literally; rawget bypasses parser quirks.
local function addr_local(a)
  if not a then return nil end
  local v = rawget(a, "local")
  if v then return v end
  for k, val in pairs(a) do
    if k == "local" then return val end
  end
  return nil
end

local function is_skipped(name)
  if name == "lo" then return true end
  for _, p in ipairs(SKIP_PREFIXES) do
    if name:sub(1, #p) == p and not name:match("^ve%-") then return true end
  end
  return false
end

local function addrs_for(name)
  local raw = sh("ip -j addr show dev " .. name)
  local data = raw and json_parse_safe(raw) or {}
  local v4, v6 = {}, {}
  if data[1] and data[1].addr_info then
    for _, a in ipairs(data[1].addr_info) do
      local loc = addr_local(a)
      if loc then
        if a.family == "inet" and not loc:match("^169%.254%.") then
          v4[#v4 + 1] = loc .. "/" .. (a.prefixlen or "?")
        elseif a.family == "inet6" and not loc:match("^fe80") then
          v6[#v6 + 1] = loc .. "/" .. (a.prefixlen or "?")
        end
      end
    end
  end
  return table.concat(v4, ", "), table.concat(v6, ", ")
end

local function parse_links()
  local raw = sh("ip -j link show")
  local list = raw and json_parse_safe(raw) or {}

  local host_ifaces = {}
  local machine_ifaces = {}

  for _, iface in ipairs(list) do
    local name = iface.ifname
    if name and not iface.master and not is_skipped(name) then
      local v4, v6 = addrs_for(name)
      local entry = {
        name  = name,
        mac   = iface.address,
        mtu   = iface.mtu,
        state = iface.operstate,
        ipv4  = v4,
        ipv6  = v6,
      }

      if name:sub(1, 3) == "ve-" then
        -- nspawn host-side veth: ifname is e.g. "ve-apex" (no "@ifN" suffix here);
        -- machine name = ifname minus "ve-"
        entry.machine = name:sub(4):gsub("[A-Za-z0-9]+$", function(s)
          -- machinectl truncates long names + adds 4-char suffix; keep best-effort full name
          return s
        end)
        machine_ifaces[#machine_ifaces + 1] = entry
      else
        host_ifaces[#host_ifaces + 1] = entry
      end
    end
  end
  return host_ifaces, machine_ifaces
end

function M.page(req)
  local snap = ctx.state.snapshot()

  local ok, host_ifaces, machine_ifaces = pcall(parse_links)
  if not ok then host_ifaces, machine_ifaces = {}, {} end

  -- Snapshot has the canonical machine names (e.g. "github-runner") even when m.ip is empty.
  -- Veth names are truncated by machinectl (e.g. "ve-github-rCia9"), so resolve by prefix.
  local known_names = {}
  for _, m in ipairs(snap.machines or {}) do
    known_names[#known_names + 1] = m.name
  end

  for _, row in ipairs(machine_ifaces) do
    local mname = row.machine or ""
    for _, known in ipairs(known_names) do
      -- The truncated veth keeps the first 8 chars of the machine name + 4-char hash suffix
      if mname == known or known:sub(1, 8) == mname:sub(1, 8) then
        row.machine = known
        break
      end
    end

    -- Container-side IPv4 from host's ARP cache for this veth
    local neigh = sh("ip -4 neigh show dev " .. row.name) or ""
    local ip = neigh:match("(%d+%.%d+%.%d+%.%d+)")
    if ip then row.container_ip = ip end
  end

  return render.render("interfaces", {
    nav_active     = "interfaces",
    host           = snap.host,
    machines       = snap.machines,
    host_ifaces    = host_ifaces,
    machine_ifaces = machine_ifaces,
  }, req)
end

return M
