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
-- host. 18786 collides with the predecessor knowhere daemon.
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

local function assert_contains(body, needle, label)
  assert.not_nil(body, label .. " body")
  assert.contains(body, needle, label)
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
  assert_contains(r.body, "<aside",                     "dashboard sidebar")
  assert_contains(r.body, "Test Brand",                 "dashboard brand name")
  assert_contains(r.body, "test-host",                  "dashboard host name")
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
  assert_contains(r.body, "agentx",     "machines list (agentx)")
  assert_contains(r.body, "k3s-server", "machines list (k3s-server)")
  ok("/machines lists stub machines")
end

-- ── /services: page renders with sidebar (no real systemd needed) ─────
do
  local r = get("/services")
  assert.eq(r.status, 200, "GET /services")
  assert_contains(r.body, "<aside", "services sidebar")
  ok("/services renders")
end

-- ── /cron, /logs, /tunnels, /tailscale, /interfaces: smoke each ───────
for _, p in ipairs({ "/cron", "/logs", "/tunnels", "/tailscale", "/interfaces" }) do
  local r = get(p)
  assert.eq(r.status, 200, "GET " .. p)
  assert_contains(r.body, "<aside", p .. " sidebar")
  ok(p .. " renders")
end

-- ── /audit: viewer renders, brand visible, no dead admin tabs ─────────
do
  local r = get("/audit")
  assert.eq(r.status, 200, "GET /audit")
  assert_contains(r.body, "Test Brand", "audit brand")
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
  assert_contains(r.body, 'href="/skip-trace"', "extra sidebar link href")
  assert_contains(r.body, "Skip trace",         "extra sidebar link label")
  ok("/ renders extra_sidebar_links")
end

-- ── grouped extra_sidebar_links: header label + indented children ─────
do
  local r = get("/")
  assert.eq(r.status, 200, "GET / for grouped sidebar")
  assert_contains(r.body, "nav-group",                "grouped sidebar nav class")
  assert_contains(r.body, 'data-section="Workflows"', "grouped sidebar details data-section")
  assert_contains(r.body, "<summary>Workflows",       "grouped sidebar summary label")
  assert_contains(r.body, 'href="/example-flow"',     "grouped sidebar child href")
  assert_contains(r.body, ">Example flow<",           "grouped sidebar child label")
  ok("/ renders grouped extra_sidebar_links")
end

-- ── /backups: page renders even with no profile (state = "B") ─────────
do
  local r = get("/backups")
  assert.eq(r.status, 200, "GET /backups")
  assert_contains(r.body, "<aside", "backups sidebar")
  ok("/backups renders (no-profile state)")
end

print("[sysops.smoke] all passed")
