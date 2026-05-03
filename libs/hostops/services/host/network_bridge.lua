-- services/host/network_bridge.lua
--
-- Idempotent ensurer for the nspawn-container bridge `nsbr0`. Containers
-- attach to this bridge via [template.nspawn] bridge="nsbr0" so each gets
-- its own DHCP'd IP on a dedicated /24 with NAT'd outbound — bypassing
-- per-veth FORWARD policies that aggressive firewall managers (kube-router,
-- Docker) install on the default veth interfaces.
--
-- Design contract: every M.ensure() call is a no-op when the bridge is
-- already configured the way we want. Only diffs trigger writes; only
-- writes trigger `networkctl reload`. Iptables rules are gated by
-- `iptables -C` checks before `-I`. Safe to call from boot and per-provision.

local audit = require("services.audit")

local M = {}

local NETWORK_DIR = "/etc/systemd/network"

-- Default config for `nsbr0`. Address/subnet are deliberately conservative —
-- 10.99.0.0/24 isn't part of any of the common k8s pod CIDRs (10.42.x.x for
-- flannel, 10.244.x.x for canal, 10.43.x.x for k3s services). Operators
-- with collisions can pass opts to override.
local DEFAULTS = {
  name      = "nsbr0",
  address   = "10.99.0.1/24",
  dns       = { "1.1.1.1", "8.8.8.8" },
}

-- Build the config file contents. Pure function; same input → same bytes.
local function netdev_content(name)
  return ("[NetDev]\nName=%s\nKind=bridge\n"):format(name)
end

local function network_content(opts)
  return string.format([[
[Match]
Name=%s

[Network]
Address=%s
DHCPServer=yes
IPMasquerade=both
LinkLocalAddressing=no
LLDP=yes
ConfigureWithoutCarrier=yes

[DHCPServer]
%s
EmitTimezone=yes
]], opts.name, opts.address,
    table.concat(
      (function() local lines = {}
        for _, ns in ipairs(opts.dns) do lines[#lines+1] = "DNS=" .. ns end
        return lines end)(),
      "\n"))
end

-- We deliberately do NOT write a .network file matching the bridge-mode
-- veths (named vb-<machine>). systemd-nspawn enslaves them to the
-- configured bridge itself when it brings the container up. Networkd
-- doesn't need to know — and a too-broad Match pattern would risk
-- accidentally enslaving the ve-* veths of containers running in
-- VirtualEthernet=yes mode (that's how this rev got it wrong: it
-- accidentally pulled apex/agentx into nsbr0 because their ve-* veths
-- matched). Keeping the 41-* file out entirely sidesteps the issue.

-- Read an absolute filesystem path. fs.read goes through knowhere's
-- LayeredFs which only knows about the embedded VFS + brand/plugins
-- overlays — it doesn't resolve /etc/. Use io.open for the real-disk
-- read against absolute paths.
local function read_disk(path)
  local f = io.open(path, "r")
  if not f then return nil end
  local body = f:read("*a")
  f:close()
  return body
end

-- Atomic file write via sudo install(1). Returns true if file content
-- differed and was replaced, false if already up to date.
local function write_if_changed(path, content)
  local current = read_disk(path)
  if current == content then return false end
  -- Stage in a tmp the user owns, then sudo install into /etc.
  local tmp = "/tmp/knowhere-network-" .. os.time() .. "-" .. math.random(0, 0xffff) .. ".tmp"
  fs.write(tmp, content)
  local r = shell.exec(("sudo -n install -D -m 0644 -o root -g root %q %q"):format(tmp, path), {})
  fs.remove(tmp)
  if not r or r.status ~= 0 then
    error("ensure_bridge: install " .. path .. " failed: " .. ((r and r.stderr) or "unknown"))
  end
  return true
end

-- Check iptables for an exact rule match; insert if missing. Returns true
-- if the rule was newly inserted.
local function ensure_iptables_rule(chain, direction, bridge)
  -- direction: "-i" (incoming on bridge) or "-o" (outgoing on bridge)
  -- chain: "FORWARD" (routed through bridge) or "INPUT" (terminating on host
  -- via the bridge, e.g. DHCP server replies)
  local check = ("sudo -n iptables -C %s %s %s -j ACCEPT 2>/dev/null"):format(chain, direction, bridge)
  local r = shell.exec(check, {})
  if r and r.status == 0 then return false end  -- already present

  local insert = ("sudo -n iptables -I %s 1 %s %s -j ACCEPT"):format(chain, direction, bridge)
  local ir = shell.exec(insert, {})
  if not ir or ir.status ~= 0 then
    error("ensure_bridge: iptables -I " .. chain .. " failed: " ..
          ((ir and ir.stderr) or "unknown"))
  end
  return true
end

--- Ensure bridge `name` is configured, up, and forwarded through iptables.
--- Safe to call repeatedly; only acts when state diverges from desired.
---
--- opts:
---   address  default "10.99.0.1/24"
---   dns      default { "1.1.1.1", "8.8.8.8" }
---   name     default "nsbr0" (bridge name)
---
--- Returns { ok=true, changed=bool, actions=[...]} describing what was done.
function M.ensure(opts)
  opts = opts or {}
  for k, v in pairs(DEFAULTS) do
    if opts[k] == nil then opts[k] = v end
  end

  local actions = {}

  local netdev  = NETWORK_DIR .. "/40-" .. opts.name .. ".netdev"
  local network = NETWORK_DIR .. "/40-" .. opts.name .. ".network"
  local stale_veth = NETWORK_DIR .. "/41-" .. opts.name .. "-veth.network"

  local netdev_changed  = write_if_changed(netdev,  netdev_content(opts.name))
  if netdev_changed  then actions[#actions+1] = "wrote " .. netdev end
  local network_changed = write_if_changed(network, network_content(opts))
  if network_changed then actions[#actions+1] = "wrote " .. network end

  -- Clean up the stale-from-an-earlier-rev veth match file if present.
  -- It accidentally enslaved unrelated ve-* veths to this bridge.
  local removed_stale = false
  if fs.exists(stale_veth) then
    local rm = shell.exec(("sudo -n rm -f %q"):format(stale_veth), {})
    if rm and rm.status == 0 then
      actions[#actions+1] = "removed stale " .. stale_veth
      removed_stale = true
    end
  end

  if netdev_changed or network_changed or removed_stale then
    local r = shell.exec("sudo -n networkctl reload", {})
    if not r or r.status ~= 0 then
      error("ensure_bridge: networkctl reload failed: " .. ((r and r.stderr) or "unknown"))
    end
    actions[#actions+1] = "networkctl reload"
  end

  -- Bring the bridge up if not already (no-op if already up).
  local up_check = shell.exec(("ip -br link show %s 2>/dev/null"):format(opts.name), {})
  local already_up = up_check and up_check.status == 0
                     and up_check.stdout and up_check.stdout:match(" UP ") ~= nil
  if not already_up then
    local up = shell.exec(("sudo -n networkctl up %s 2>&1 || true"):format(opts.name), {})
    actions[#actions+1] = "networkctl up " .. opts.name
    -- networkctl up returns 0 even when the bridge is just about to come up;
    -- we don't gate on its exit status. The .network file's ConfigureWithoutCarrier=yes
    -- ensures the bridge boots without an attached cable.
    local _ = up
  end

  -- Disable bridge-netfilter for THIS bridge only. The global sysctl
  -- net.bridge.bridge-nf-call-iptables=1 is required by k8s/kube-router
  -- (their bridges need iptables traversal for service mesh). But that
  -- forces nsbr0's traffic through INPUT/FORWARD too — where kube-router
  -- aggressively reinstalls drop rules at position 1 and evicts any
  -- ACCEPT we add. Per-bridge knobs in /sys/class/net/<br>/bridge/* are
  -- the right escape hatch: keep iptables on for k8s bridges, turn it
  -- off for nsbr0. Idempotent: writes "0" to the same files repeatedly
  -- is a no-op.
  for _, knob in ipairs({ "nf_call_iptables", "nf_call_ip6tables", "nf_call_arptables" }) do
    local path = "/sys/class/net/" .. opts.name .. "/bridge/" .. knob
    if fs.exists(path) then
      local cur = (read_disk(path) or ""):gsub("%s+$", "")
      if cur ~= "0" then
        local r = shell.exec(("sudo -n sh -c %q"):format(
          "echo 0 > " .. path), {})
        if not r or r.status ~= 0 then
          error("ensure_bridge: writing " .. path .. " failed: " ..
                ((r and r.stderr) or "unknown"))
        end
        actions[#actions+1] = path .. "=0"
      end
    end
  end

  -- FORWARD ACCEPT in both directions. These cover routed traffic
  -- (container ↔ external). br_netfilter being off for nsbr0 means
  -- bridge-internal DHCP no longer goes through iptables, but routed
  -- container traffic still does — and kube-router's FORWARD chain
  -- defaults to drop, so we need explicit ACCEPT. Order matters less
  -- here than in INPUT because FORWARD rule re-installation by other
  -- managers is rarer; we still insert at position 1 defensively.
  if ensure_iptables_rule("FORWARD", "-i", opts.name) then
    actions[#actions+1] = "iptables -I FORWARD -i " .. opts.name .. " -j ACCEPT"
  end
  if ensure_iptables_rule("FORWARD", "-o", opts.name) then
    actions[#actions+1] = "iptables -I FORWARD -o " .. opts.name .. " -j ACCEPT"
  end

  if #actions > 0 then
    audit.append({
      actor  = "system",
      action = "host.network_bridge.ensure",
      target = opts.name,
      meta   = { actions = actions },
    })
    log.info("network_bridge: " .. opts.name .. ": " .. table.concat(actions, "; "))
  end

  return { ok = true, changed = #actions > 0, actions = actions }
end

-- ── Static IP allocation for nspawn containers ─────────────────────────────
--
-- Why static-IP rather than DHCP: systemd-networkd's built-in DHCPServer
-- on `nsbr0` doesn't reliably issue leases on this host class — DHCP
-- DISCOVERs arrive at the host bridge but the server never replies (the
-- DHCPv4 server appears not to start at all on bridge interfaces in some
-- configurations). dnsmasq as a dedicated DHCP service has the same
-- broadcast-reception failure. Static IP per container, written into the
-- container's rootfs at provision time, sidesteps the entire DHCP path.
--
-- Allocations live in <data_dir>/network/leases.toml as a stable mapping
-- from container name → IP, so the same name always gets the same IP
-- across re-provisions (idempotent). The pool is 10.99.0.50–10.99.0.200.

local LEASES_FILE = (function()
  -- assay sandbox doesn't expose os.getenv at module-load; use env.get
  -- which is the assay-provided builtin.
  local data_dir = (env and env.get and env.get("KNOWHERE_DATA_DIR")) or "/var/lib/knowhere"
  return data_dir .. "/network/leases.toml"
end)()

local POOL_START = 50
local POOL_END   = 200

local function read_leases_disk(path)
  -- io.open instead of fs.read because LayeredFs doesn't resolve absolute paths.
  local f = io.open(path, "r")
  if not f then return { leases = {} } end
  local body = f:read("*a") or ""
  f:close()
  local ok, parsed = pcall(toml.parse, body)
  if not ok or type(parsed) ~= "table" then return { leases = {} } end
  return { leases = parsed.leases or {} }
end

local function write_leases_disk(state)
  local body = "# Managed by knowhere; static-IP allocations on nsbr0.\n"
  -- Stable key order for deterministic file content.
  local names = {}
  for n, _ in pairs(state.leases or {}) do names[#names+1] = n end
  table.sort(names)
  for _, name in ipairs(names) do
    body = body .. ("\n[leases.%q]\nip = %q\n"):format(name, state.leases[name].ip)
  end
  local dir = LEASES_FILE:match("^(.*)/[^/]+$")
  if dir then
    fs.mkdir(dir)
  end
  local tmp = LEASES_FILE .. ".tmp." .. tostring(os.time())
  fs.write(tmp, body)
  fs.rename(tmp, LEASES_FILE)
end

--- Allocate (or return existing) a static IP for `name`. Idempotent —
--- same name always returns same IP. Returns nil and an error string
--- on pool exhaustion.
function M.allocate_ip(name)
  local state = read_leases_disk(LEASES_FILE)
  if state.leases[name] then
    return state.leases[name].ip
  end
  local taken = {}
  for _, lease in pairs(state.leases) do
    local last = tonumber((lease.ip:match("(%d+)$")))
    if last then taken[last] = true end
  end
  for octet = POOL_START, POOL_END do
    if not taken[octet] then
      local ip = "10.99.0." .. tostring(octet)
      state.leases[name] = { ip = ip }
      write_leases_disk(state)
      return ip
    end
  end
  return nil, "nsbr0 IP pool 10.99.0." .. POOL_START .. "-" .. POOL_END .. " exhausted"
end

--- Release the lease for `name`. Called from the destroy flow.
function M.release_ip(name)
  local state = read_leases_disk(LEASES_FILE)
  if state.leases[name] then
    state.leases[name] = nil
    write_leases_disk(state)
    return true
  end
  return false
end

--- Write the per-container static-IP networkd config into a freshly-
--- bootstrapped rootfs. Path is
--- /var/lib/machines/<name>/etc/systemd/network/10-host0-static.network.
--- Uses sudo install since /var/lib/machines is root-only.
---
--- Returns the IP allocated.
function M.write_container_static_config(name)
  local ip, err = M.allocate_ip(name)
  if not ip then return nil, err end

  local body = ([[# Static IP for nspawn host0 — allocated by knowhere
# (see services/host/network_bridge.lua). nsbr0's built-in DHCP server
# doesn't reliably issue leases on this host; static avoids the dependency.
[Match]
Name=host0

[Network]
Address=%s/24
Gateway=10.99.0.1
DNS=1.1.1.1
DNS=8.8.8.8

[Link]
RequiredForOnline=routable
]]):format(ip)

  local rootfs_dir = "/var/lib/machines/" .. name .. "/etc/systemd/network"
  local dst = rootfs_dir .. "/10-host0-static.network"
  local tmp = "/tmp/knowhere-host0-" .. name .. "." .. tostring(os.time())
  fs.write(tmp, body)
  local cmd = ("sudo -n install -D -m 0644 -o root -g root %q %q"):format(tmp, dst)
  local r = shell.exec(cmd, {})
  fs.remove(tmp)
  if not r or r.status ~= 0 then
    return nil, "install host0-static.network failed: " .. ((r and r.stderr) or "unknown")
  end
  return ip
end

return M
