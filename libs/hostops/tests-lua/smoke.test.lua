--! hostops smoke test — boots the lib in-process with stub services on
--! a non-default port, curls a representative set of routes, asserts
--! shape + content. No shell, no external HTTP.
--!
--! Run via:
--!   LUA_PATH='libs/?.lua;libs/?/init.lua;libs/hostops/?.lua;libs/hostops/tests-lua/?.lua;;' \
--!     assay libs/hostops/tests-lua/smoke.test.lua

local hostops = require("hostops.mount")
local stubs   = require("stubs")

local function fail(msg)  error("smoke fail: " .. tostring(msg), 2) end
local function ok(label) print("  ✓ " .. label) end

print("[hostops.smoke]")

-- Pick a high port that's unlikely to be claimed by anything else on the
-- host. 18786 collides with the predecessor knowhere daemon.
local PORT = 47917
local opts = stubs.opts({
  extra_sidebar_links = {
    { href = "/skip-trace", label = "Skip trace", nav_active = "skip_trace" },
  },
})

-- Build the routes table; mount() registers every host-ops route on it.
local routes = { GET = {}, POST = {} }
hostops.mount(routes, opts)

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
  if not body or not body:find(needle, 1, true) then
    fail(("%s: missing %q in body (got %s bytes)"):format(label, needle, body and #body or 0))
  end
end

-- ── harness sanity probe ──────────────────────────────────────────────
do
  local r = get("/__smoke_alive")
  if r.status ~= 200 or r.body ~= "ok" then
    fail(("alive: status=%s body=%q"):format(tostring(r.status), tostring(r.body)))
  end
  ok("/__smoke_alive returns 200 ok")
end

-- ── / dashboard: layout sidebar + brand-name footer ───────────────────
do
  local r = get("/")
  if r.status ~= 200 then fail("GET / → " .. r.status) end
  assert_contains(r.body, "<aside",                     "dashboard sidebar")
  assert_contains(r.body, "Test Brand",                 "dashboard brand name")
  assert_contains(r.body, "test-host",                  "dashboard host name")
  ok("/ renders dashboard with sidebar + brand")
end

-- ── /static/styles.css: serves CSS from libs/hostops/static/ ──────────
do
  local r = get("/static/styles.css")
  if r.status ~= 200 then fail("static styles.css: " .. tostring(r.status)) end
  if not (r.headers and r.headers["content-type"]) then
    fail("static missing content-type header")
  end
  if not r.headers["content-type"]:find("text/css", 1, true) then
    fail("static wrong content-type: " .. r.headers["content-type"])
  end
  ok("/static/styles.css returns text/css")
end

-- ── /machines: lists fixture machines from stub state ─────────────────
do
  local r = get("/machines")
  if r.status ~= 200 then fail("GET /machines → " .. r.status) end
  assert_contains(r.body, "agentx",     "machines list (agentx)")
  assert_contains(r.body, "k3s-server", "machines list (k3s-server)")
  ok("/machines lists stub machines")
end

-- ── /services: page renders with sidebar (no real systemd needed) ─────
do
  local r = get("/services")
  if r.status ~= 200 then fail("GET /services → " .. r.status) end
  assert_contains(r.body, "<aside", "services sidebar")
  ok("/services renders")
end

-- ── /cron, /logs, /tunnels, /tailscale, /interfaces: smoke each ───────
for _, p in ipairs({ "/cron", "/logs", "/tunnels", "/tailscale", "/interfaces" }) do
  local r = get(p)
  if r.status ~= 200 then fail("GET " .. p .. " → " .. r.status) end
  assert_contains(r.body, "<aside", p .. " sidebar")
  ok(p .. " renders")
end

-- ── /audit: viewer renders, brand visible, no dead admin tabs ─────────
do
  local r = get("/audit")
  if r.status ~= 200 then fail("GET /audit → " .. r.status) end
  assert_contains(r.body, "Test Brand", "audit brand")
  -- Tabs to /inventory, /packages, /settings were removed in 0.1.2 —
  -- those routes don't exist. Catch any future regression that puts
  -- them back without registering handlers.
  if r.body:find('href="/inventory"', 1, true)
     or r.body:find('href="/packages"', 1, true)
     or r.body:find('href="/settings"', 1, true) then
    fail("audit page contains dead /inventory|/packages|/settings link")
  end
  ok("/audit renders, no dead admin tabs")
end

-- ── extra_sidebar_links: layout renders consumer-app links ────────────
do
  local r = get("/")
  if r.status ~= 200 then fail("GET / for extra-sidebar-links → " .. r.status) end
  assert_contains(r.body, 'href="/skip-trace"', "extra sidebar link href")
  assert_contains(r.body, "Skip trace",         "extra sidebar link label")
  ok("/ renders extra_sidebar_links")
end

-- ── /backups: page renders even with no profile (state = "B") ─────────
do
  local r = get("/backups")
  if r.status ~= 200 then fail("GET /backups → " .. r.status) end
  assert_contains(r.body, "<aside", "backups sidebar")
  ok("/backups renders (no-profile state)")
end

print("[hostops.smoke] all passed")
