--! sysops smoke test — boots the lib in-process with stub services on
--! a non-default port, curls a representative set of routes, asserts
--! shape + content. No shell, no external HTTP.
--!
--! Run via:
--!   LUA_PATH='libs/?.lua;libs/?/init.lua;libs/sysops/?.lua;libs/sysops/tests-lua/?.lua;;' \
--!     assay libs/sysops/tests-lua/smoke.test.lua

local sysops = require("sysops.mount")
local stubs   = require("stubs")

local function ok(label) print("  ✓ " .. label) end

print("[sysops.smoke]")

-- Pick a high port that's unlikely to be claimed by anything else on the
-- host. 18786 collides with the predecessor daemon.
local PORT = 47917
local opts = stubs.opts({
  extra_sidebar_links = {
    { href = "/skip-trace", label = "Skip trace", nav_active = "skip_trace" },
    {
      label = "Workflows",
      children = {
        { href = "/example-flow", label = "Example flow", nav_active = "example_flow" },
      },
    },
  },
})

local original_systemd = systemd
local cpu_status_calls = {}

systemd = {
  list_units = function(filter)
    local units = {
    {
      name = "demo.service",
      description = "Demo service",
      load = "loaded",
      active = "active",
      sub = "running",
    },
    {
      name = "heavy.service",
      description = "Heavy service",
      load = "loaded",
      active = "active",
      sub = "running",
    },
    {
      name = "demo.timer",
      description = "Demo timer",
      load = "loaded",
      active = "active",
        sub = "waiting",
      },
    }
    if filter == "*.service" then return { units[1], units[2] } end
    if filter == "*.timer" then return { units[3] } end
    return units
  end,
  unit_status = function(name)
    cpu_status_calls[name] = (cpu_status_calls[name] or 0) + 1
    if name == "demo.service" then
      return {
        name = "demo.service",
        load = "loaded",
        active = "active",
        sub = "running",
        description = "Demo service",
        memory_current = 67108864,
        tasks_current = 22,
        cpu_usage_nsec = 1000000000 + (cpu_status_calls[name] * 2000000000),
        n_restarts = 2,
        unit_file_state = "enabled",
        fragment_path = "/etc/systemd/system/demo.service",
        main_pid = 4242,
        exec_start = "/usr/bin/demo --flag",
        restart = "on-failure",
      }
    end
    if name == "heavy.service" then
      return {
        name = "heavy.service",
        load = "loaded",
        active = "active",
        sub = "running",
        description = "Heavy service",
        memory_current = 134217728,
        tasks_current = 11,
        cpu_usage_nsec = 1000000000 + (cpu_status_calls[name] * 100000000),
        n_restarts = 0,
        unit_file_state = "enabled",
        fragment_path = "/etc/systemd/system/heavy.service",
      }
    end
    return {
      name = name,
      load = "loaded",
      active = "active",
      sub = "running",
      description = name,
    }
  end,
  list_timers = function() return {} end,
  journal = function() return {} end,
  unit_action = function()
    return { status = 0, stdout = "", stderr = "" }
  end,
}

-- Build the routes table; mount() registers every host-ops route on it.
local routes = { GET = {}, POST = {} }
sysops.mount(routes, opts)

-- Add a unique liveness probe. /healthz collides with the assay
-- binary's embedded engine, so use a smoke-test-only route name.
routes.GET["/__smoke_alive"] = function() return { status = 200, body = "ok" } end

print(("  routes registered: GET=%d, POST=%d, alive=%s"):format(
  (function() local n = 0; for _ in pairs(routes.GET) do n = n + 1 end; return n end)(),
  (function() local n = 0; for _ in pairs(routes.POST) do n = n + 1 end; return n end)(),
  type(routes.GET["/__smoke_alive"])))

assert.eq(type(routes.POST["/api/services/restart"]), "function", "service restart route")
assert.eq(type(routes.POST["/api/services/start"]), "function", "service start route")
assert.eq(type(routes.POST["/api/services/stop"]), "function", "service stop route")

-- Boot the server in a worker; the test thread issues HTTP requests
-- against it.
async.spawn(function()
  http.serve(PORT, routes)
end)

-- Give the listener a moment to bind.
sleep(0.5)

local function get(path)
  return http.get("http://127.0.0.1:" .. PORT .. path)
end

-- ── harness sanity probe ──────────────────────────────────────────────
do
  local r = get("/__smoke_alive")
  assert.eq(r.status, 200, "alive status")
  assert.eq(r.body, "ok", "alive body")
  ok("/__smoke_alive returns 200 ok")
end

-- ── / dashboard: layout sidebar + brand-name footer ───────────────────
do
  local r = get("/")
  assert.eq(r.status, 200, "GET /")
  assert.not_nil(r.body, "dashboard body")
  assert.contains(r.body, "<aside", "dashboard sidebar")
  assert.contains(r.body, "Test Brand", "dashboard brand name")
  assert.contains(r.body, "test-host", "dashboard host name")
  ok("/ renders dashboard with sidebar + brand")
end

-- ── /static/styles.css: serves CSS from libs/sysops/static/ ──────────
do
  local r = get("/static/styles.css")
  assert.eq(r.status, 200, "static styles.css")
  assert.not_nil(r.headers and r.headers["content-type"], "static content-type header")
  assert.contains(r.headers["content-type"], "text/css", "static content-type")
  ok("/static/styles.css returns text/css")
end

-- ── /machines: lists fixture machines from stub state ─────────────────
do
  local r = get("/machines")
  assert.eq(r.status, 200, "GET /machines")
  assert.not_nil(r.body, "machines body")
  assert.contains(r.body, "agentx", "machines list (agentx)")
  assert.contains(r.body, "k3s-server", "machines list (k3s-server)")
  ok("/machines lists stub machines")
end

-- ── /services: page renders with sidebar (no real systemd needed) ─────
do
  local r = get("/services")
  assert.eq(r.status, 200, "GET /services")
  assert.not_nil(r.body, "services body")
  assert.contains(r.body, "<aside", "services sidebar")
  assert.contains(r.body, "Memory", "services memory column")
  assert.contains(r.body, "Tasks", "services tasks column")
  assert.contains(r.body, "CPU", "services CPU usage column")
  assert.eq(r.body:find("<th>Load</th>", 1, true), nil, "services omits load column")
  assert.eq(r.body:find("<td>loaded</td>", 1, true), nil, "services omits load cells")
  assert.eq(r.body:find("Load state", 1, true), nil, "services omits load detail")
  assert.eq(r.body:find("<th>Sub</th>", 1, true), nil, "services omits sub column")
  assert.eq(r.body:find("<th>Restarts</th>", 1, true), nil, "services omits restarts column")
  assert.eq(r.body:find(">Description</th>", 1, true), nil, "services omits description column")
  assert.eq(r.body:find("CPU Time", 1, true), nil, "services omits CPU time column")
  assert.contains(r.body, "sort=memory", "services memory sort link")
  assert.contains(r.body, "sort=cpu", "services CPU sort link")
  assert.contains(r.body, "64 M", "services memory value")
  assert.contains(r.body, "%", "services CPU usage value")
  assert.contains(r.body, "service-toggle", "services row has expandable unit control")
  assert.contains(r.body, "service-detail-row", "services renders expanded-detail row shell")
  assert.contains(r.body, "Unit file", "services detail includes unit file label")
  assert.contains(r.body, "enabled", "services detail includes unit file value")
  assert.contains(r.body, "Main PID", "services detail includes main pid label")
  assert.contains(r.body, "4242", "services detail includes main pid value")
  assert.contains(r.body, "Exec start", "services detail includes exec label")
  assert.contains(r.body, "demo --flag", "services detail includes exec value")
  assert.contains(r.body, 'action="/api/services/start"', "services start form")
  assert.contains(r.body, 'action="/api/services/stop"', "services stop form")
  assert.contains(r.body, 'action="/api/services/restart"', "services restart form")
  ok("/services renders expandable service details + lifecycle actions")
end

do
  local r = get("/services?sort=memory&dir=desc")
  assert.eq(r.status, 200, "GET /services memory sort")
  assert.not_nil(r.body, "services memory-sort body")
  assert.contains(r.body, "Memory <span>↓</span>", "services memory desc arrow")
  assert.eq(r.body:find("Memory <span>desc</span>", 1, true), nil, "services memory sort hides desc text")
  local heavy_pos = r.body:find("heavy.service", 1, true)
  local demo_pos = r.body:find("demo.service", 1, true)
  assert.not_nil(heavy_pos, "services memory sort includes heavy service")
  assert.not_nil(demo_pos, "services memory sort includes demo service")
  assert.eq(heavy_pos < demo_pos, true, "services sorts by memory descending")

  r = get("/services?sort=memory&dir=asc")
  assert.eq(r.status, 200, "GET /services memory sort asc")
  assert.not_nil(r.body, "services memory-sort asc body")
  assert.contains(r.body, "Memory <span>↑</span>", "services memory asc arrow")
  assert.eq(r.body:find("Memory <span>asc</span>", 1, true), nil, "services memory sort hides asc text")

  r = get("/services?sort=cpu&dir=desc")
  assert.eq(r.status, 200, "GET /services CPU sort")
  assert.not_nil(r.body, "services CPU-sort body")
  assert.contains(r.body, "CPU <span>↓</span>", "services CPU desc arrow")
  assert.eq(r.body:find("CPU <span>desc</span>", 1, true), nil, "services CPU sort hides desc text")
  heavy_pos = r.body:find("heavy.service", 1, true)
  demo_pos = r.body:find("demo.service", 1, true)
  assert.not_nil(heavy_pos, "services CPU sort includes heavy service")
  assert.not_nil(demo_pos, "services CPU sort includes demo service")
  assert.eq(demo_pos < heavy_pos, true, "services sorts by CPU descending")
  ok("/services sorts by memory and CPU")
end

-- ── /cron, /logs, /tunnels, /tailscale, /interfaces: smoke each ───────
for _, p in ipairs({ "/cron", "/logs", "/tunnels", "/tailscale", "/interfaces" }) do
  local r = get(p)
  assert.eq(r.status, 200, "GET " .. p)
  assert.not_nil(r.body, p .. " body")
  assert.contains(r.body, "<aside", p .. " sidebar")
  ok(p .. " renders")
end

-- ── /audit: viewer renders, brand visible, no dead admin tabs ─────────
do
  local r = get("/audit")
  assert.eq(r.status, 200, "GET /audit")
  assert.not_nil(r.body, "audit body")
  assert.contains(r.body, "Test Brand", "audit brand")
  -- Tabs to /inventory, /packages, /settings were removed in 0.1.2 —
  -- those routes don't exist. Catch any future regression that puts
  -- them back without registering handlers.
  assert.eq(r.body:find('href="/inventory"', 1, true), nil, "audit no /inventory link")
  assert.eq(r.body:find('href="/packages"', 1, true), nil, "audit no /packages link")
  assert.eq(r.body:find('href="/settings"', 1, true), nil, "audit no /settings link")
  ok("/audit renders, no dead admin tabs")
end

-- ── extra_sidebar_links: layout renders consumer-app links ────────────
do
  local r = get("/")
  assert.eq(r.status, 200, "GET / for extra-sidebar-links")
  assert.not_nil(r.body, "extra-sidebar body")
  assert.contains(r.body, 'href="/skip-trace"', "extra sidebar link href")
  assert.contains(r.body, "Skip trace", "extra sidebar link label")
  ok("/ renders extra_sidebar_links")
end

-- ── grouped extra_sidebar_links: header label + indented children ─────
do
  local r = get("/")
  assert.eq(r.status, 200, "GET / for grouped sidebar")
  assert.not_nil(r.body, "grouped-sidebar body")
  assert.contains(r.body, "nav-group", "grouped sidebar nav class")
  assert.contains(r.body, 'data-section="Workflows"', "grouped sidebar details data-section")
  assert.contains(r.body, "<summary>Workflows", "grouped sidebar summary label")
  assert.contains(r.body, 'href="/example-flow"', "grouped sidebar child href")
  assert.contains(r.body, ">Example flow<", "grouped sidebar child label")
  ok("/ renders grouped extra_sidebar_links")
end

-- ── /backups: page renders even with no profile (state = "B") ─────────
do
  local r = get("/backups")
  assert.eq(r.status, 200, "GET /backups")
  assert.not_nil(r.body, "backups body")
  assert.contains(r.body, "<aside", "backups sidebar")
  ok("/backups renders (no-profile state)")
end

print("[sysops.smoke] all passed")

systemd = original_systemd
