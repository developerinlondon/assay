local render = require("pages.render")
local ctx = require("hostops.ctx")
local M = {}

local STUBS = {
  services     = { title = "Services",               eyebrow = "System · systemd units",   phase = "Phase 6",  nav = "services",
    coming = { "All systemd units with status, CPU & memory", "Filter by type: service / timer / socket / mount", "Inline restart / stop actions", "Export unit list as JSON" } },
  cron         = { title = "Cron & Timers",           eyebrow = "System · cron & timers",   phase = "Phase 6",  nav = "cron",
    coming = { "systemd timer list with next/last run times", "/etc/cron.d and user crontab preview", "Overdue / disabled alerts", "Reload daemon button" } },
  logs         = { title = "Logs",                    eyebrow = "System · journalctl",       phase = "Phase 5",  nav = "logs",
    coming = { "Live journal stream via systemd.journal_follow", "Filter by machine / unit / priority", "Pause + export ndjson", "Per-machine journal drill-down" } },
  tunnels      = { title = "Tunnels",                 eyebrow = "Edge · cloudflared",        phase = "Phase 7",  nav = "tunnels",
    coming = { "cloudflared.service status and tunnel ID", "Ingress routes table with latency probes", "Reload config / add hostname actions", "Health-check history per route" } },
  interfaces   = { title = "Interfaces",              eyebrow = "Networks · host & nspawn",  phase = "Phase 7",  nav = "interfaces",
    coming = { "Host NIC stats (rx/tx, state, MTU)", "Per-machine veth + container IP", "NAT / nftables masquerade rule summary", "UFW rule preview" } },
  tailscale    = { title = "Tailscale",               eyebrow = "Networks · tailnet",        phase = "Phase 7",  nav = "tailscale",
    coming = { "tailscale status: tailnet name, DERP latency, MagicDNS", "Device list with IPs, OS, tags, key expiry", "Subnet advertisement and exit-node status", "ACL preview from tailscale acl get" } },
  inventory    = { title = "Inventory",               eyebrow = '<a href="/inventory">Admin</a> &middot; hosts.yml',         phase = "Phase 9",  nav = "inventory",
    coming = { "hosts.yml parsed view with live-state comparison", "Drift detection: declared vs running", "Inline editor with git diff preview", "Ansible plan / apply trigger" } },
  audit        = { title = "Audit log",               eyebrow = '<a href="/inventory">Admin</a> &middot; mutations',          phase = "Phase 8",  nav = "audit",
    coming = { "In-memory ring buffer of all write actions (24h)", "Attribution from Cloudflare Access JWT", "Filter by user / action type / time range", "Export as ndjson" } },
  settings     = { title = "Settings",                eyebrow = '<a href="/inventory">Admin</a> &middot; preferences',        phase = "Phase 11", nav = "settings",
    coming = { "Poll intervals and data retention tuning", "Notification destinations (webhook / email)", "API token management for automation access", "Theme and display preferences" } },
  provision_new = { title = "Provision new machine",  eyebrow = '<a href="/machines">Containers</a> &middot; provision',     phase = "Phase 3",  nav = "provision_new",
    coming = { "Declare new container in hosts.yml via form", "Distro + memory + CPU quota picker", "Ansible apply to bootstrap and start the machine", "Progress stream while debootstrap runs" } },
}

local function make_handler(key)
  local def = STUBS[key]
  return function(req)
    local snap = ctx.state.snapshot()
    return render.render("stub", {
      nav_active   = def.nav,
      page_title   = def.title,
      page_eyebrow = def.eyebrow,
      coming       = def.coming,
      host         = snap.host,
      machines     = snap.machines,
    }, req)
  end
end

for key, _ in pairs(STUBS) do
  M[key] = make_handler(key)
end

return M
