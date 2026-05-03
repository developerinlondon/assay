--! hostops library entry point.
--!
--! Usage from a consumer app:
--!
--!   local hostops = require("hostops.mount")
--!   local routes  = { GET = {}, POST = {} }
--!   hostops.mount(routes, {
--!     prefix = "/host",                       -- optional, default "/"
--!     state  = require("app.services.state"),
--!     audit  = require("app.services.audit"),
--!     jobs   = require("app.services.jobs"),
--!     secret = require("app.services.secret"),
--!     brand  = require("app.brand"),
--!     engine = engine_http_client,            -- HTTP wrapper to engine
--!     lib_root = "/opt/assay/libs/hostops",   -- optional, default "."
--!     -- Optional package-management config (used by /machines/new and
--!     -- container-provisioning flow; defaults below):
--!     catalog_paths      = { "/etc/myapp/catalogs" },
--!     template_paths     = { "/etc/myapp/templates" },
--!     desired_state_path = "/var/lib/myapp/pkg/desired_state.json",
--!   })
--!   http.serve(8080, routes)
--!
--! mount() registers every host-ops route on the caller's `routes`
--! table, prefixed by `opts.prefix`. Pages access injected services
--! through `hostops.ctx` rather than top-level requires.
--!
--! Three contract properties:
--!
--! 1. **Prefix-safe templates.** Internal links go through `ctx.url(p)`
--!    so `/machines/web` becomes `/host/machines/web` when mounted at
--!    `/host`. Templates use `{{ url("/machines") }}` style refs.
--! 2. **Injectable services.** `state`, `audit`, `jobs`, `secret`,
--!    `brand`, `engine` arrive via opts. The library never `require`s
--!    them at top level.
--! 3. **Engine over HTTP.** `engine` wraps `http.request` against
--!    `ENGINE_URL`, replacing the in-process `knowhere.engine.api_call`
--!    used by the predecessor monolith.

local ctx   = require("hostops.ctx")
local pages = require("hostops.pages")

local M = {}

--- Build a prefix-safe URL helper for routes mounted at `prefix`.
local function build_url(prefix)
  prefix = prefix or "/"
  if prefix ~= "/" and prefix:sub(-1) == "/" then
    prefix = prefix:sub(1, -2)
  end
  if prefix == "/" then prefix = "" end

  return function(path)
    path = path or "/"
    if path:sub(1, 1) ~= "/" then path = "/" .. path end
    if prefix == "" then return path end
    if path == "/" then return prefix end
    return prefix .. path
  end
end

local function require_table(opts, key)
  local v = opts[key]
  if type(v) ~= "table" then
    error(("hostops.mount: opts.%s must be a table (got %s)"):format(key, type(v)), 3)
  end
  return v
end

----------------------------------------------------------------------
-- URL → handler-slug map. Every value here is a key into pages.lua's
-- handler table. Wildcards inside the URL become http.serve glob
-- patterns; mount() prepends opts.prefix at attach time.
----------------------------------------------------------------------

local GET_ROUTES = {
  -- Top-level dashboard + SSE + dashboard fragments.
  ["/"]                              = "dashboard",
  ["/api/events"]                    = "events",
  ["/api/overview/host-strip"]       = "host_strip",
  ["/api/overview/machines-grid"]    = "machines_grid",
  ["/api/overview/status-strip"]     = "status_strip",
  ["/api/overview/recent-activity"]  = "recent_activity",

  -- Async machine-provision job state. Match BEFORE the wildcard so
  -- /api/machines/jobs/<id> doesn't fall through to the wildcard.
  ["/api/machines/jobs/*"]           = "machine_job_status",

  -- Logs SSE stream.
  ["/api/logs/stream"]               = "logs_stream",

  -- Host-level read-only pages.
  ["/services"]                      = "services",
  ["/cron"]                          = "cron",
  ["/logs"]                          = "logs",

  -- Host shell (xterm.js + WS PTY bridge).
  ["/host/shell"]                    = "shell_host",
  ["/api/host/shell"]                = "shell_host_ws",

  -- Networks.
  ["/tunnels"]                       = "tunnels",
  ["/interfaces"]                    = "interfaces",
  ["/tailscale"]                     = "tailscale",

  -- Audit log + export.
  ["/audit"]                         = "audit",
  ["/api/audit/export"]              = "audit_export",

  -- Backups (read-only views + editor pages).
  ["/backups"]                       = "backups",
  ["/backups/sources"]               = "backups_sources_editor",
  ["/backups/schedule"]              = "backups_schedule_editor",

  -- Backup snapshot detail + job status (wildcard paths).
  ["/backups/snapshot/*"]            = "backups_snapshot_detail",
  ["/backups/job/*"]                 = "backups_job_detail",
  ["/api/backups/jobs/*"]            = "backups_job_status",

  -- nspawn containers list.
  ["/machines"]                      = "machines_index",
}

local POST_ROUTES = {
  -- Provision a new container (POST /api/machines).
  ["/api/machines"]                  = "machine_provision",

  -- Lifecycle actions: /api/machines/<name>/<action>.
  -- Note: this also catches POST /api/machines/<name>/destroy etc.
  ["/api/machines/*"]                = "machine_action",

  -- Backups setup + run actions.
  ["/api/backups/setup/test"]        = "backups_setup_test",
  ["/api/backups/setup/init"]        = "backups_setup_init",
  ["/api/backups/reconfigure"]       = "backups_reconfigure",
  ["/api/backups/sources"]           = "backups_sources_update",
  ["/api/backups/schedule"]          = "backups_schedule_update",
  ["/api/backups/run"]               = "backups_run_now",
  ["/api/backups/restore"]           = "backups_restore_action",
}

----------------------------------------------------------------------
-- Wildcard dispatchers. Path patterns http.serve doesn't natively
-- pattern-match — we read req.path inline.
----------------------------------------------------------------------

local function machines_get_wildcard(req)
  local rest = (req.path or ""):match("^/machines/(.+)$")
  if not rest then return { status = 404, body = "not found" } end
  if rest == "new" then return pages.provision_new(req) end
  if rest:match("^[^/]+/shell$")    then return pages.shell_machine(req)   end
  if rest:match("^[^/]+/services$") then return pages.machine_services(req) end
  if rest:match("^[^/]+/cron$")     then return pages.machine_cron(req)     end
  if rest:match("^[^/]+/logs$")     then return pages.machine_logs(req)     end
  return pages.machine_detail(req)
end

local function api_machines_get_wildcard(req)
  local suffix = (req.path or ""):match("^/api/machines/[^/]+/(.+)$")
  if suffix == "utilization" then return pages.machine_utilization(req) end
  if suffix == "processes"   then return pages.machine_processes(req)   end
  if suffix == "journal"     then return pages.machine_journal(req)     end
  if suffix == "shell"       then return pages.shell_machine_ws(req)    end
  return { status = 404, body = "not found" }
end

----------------------------------------------------------------------
-- Static file handler.
--
-- Reads from `<lib_root>/static/<path>` so the lib can be installed at
-- any filesystem location. Defaults to "." for in-repo development;
-- consumer apps installed via `assay install` pass the resolved path.
----------------------------------------------------------------------

local function content_type(path)
  if path:match("%.js$")   then return "application/javascript" end
  if path:match("%.css$")  then return "text/css" end
  if path:match("%.svg$")  then return "image/svg+xml" end
  if path:match("%.png$")  then return "image/png" end
  if path:match("%.html$") then return "text/html; charset=utf-8" end
  if path:match("%.json$") then return "application/json" end
  return "application/octet-stream"
end

local function build_static_handler(lib_root)
  return function(req)
    local rel = (req.path or ""):match("^.-/static/(.+)$")
    if not rel or rel:find("%.%.") then
      return { status = 400, body = "bad path" }
    end
    local body = fs.read(lib_root .. "/static/" .. rel)
    if not body then return { status = 404, body = "not found" } end
    return {
      status  = 200,
      body    = body,
      headers = {
        ["Content-Type"]  = content_type(rel),
        ["Cache-Control"] = "no-cache, must-revalidate",
      },
    }
  end
end

----------------------------------------------------------------------
-- mount()
----------------------------------------------------------------------

function M.mount(routes, opts)
  if type(routes) ~= "table" then
    error("hostops.mount(routes, opts): routes must be a table", 2)
  end
  if type(opts) ~= "table" then
    error("hostops.mount(routes, opts): opts must be a table", 2)
  end

  ctx.prefix = opts.prefix or "/"
  ctx.url    = build_url(ctx.prefix)
  ctx.state  = require_table(opts, "state")
  ctx.audit  = require_table(opts, "audit")
  ctx.jobs   = require_table(opts, "jobs")
  ctx.secret = require_table(opts, "secret")
  ctx.brand  = require_table(opts, "brand")
  ctx.engine = require_table(opts, "engine")

  -- Optional pkg-management paths. nil-safe — `services/pkg_view.lua`
  -- treats absent paths as empty catalogs / no persistence.
  ctx.catalog_paths      = opts.catalog_paths
  ctx.template_paths     = opts.template_paths
  ctx.desired_state_path = opts.desired_state_path
  -- Backup profile directory (default /etc/rustic). Tests override.
  ctx.backup_profile_dir = opts.backup_profile_dir

  -- Base URL of the consumer's engine sidecar (e.g.
  -- "https://knowhere2-engine.agenteda.com"). When set, the sidebar
  -- exposes links to the engine's whitelabeled SPA at /auth/console,
  -- /vault/console, /engine/console, /workflow/. Nil = links hidden.
  ctx.engine_base_url = opts.engine_base_url

  ctx.lib_root = opts.lib_root or "."
  local lib_root = ctx.lib_root

  routes.GET  = routes.GET  or {}
  routes.POST = routes.POST or {}

  -- Static file route — must register before the wildcard dispatchers.
  routes.GET[ctx.url("/static/*")] = build_static_handler(lib_root)

  -- Concrete + glob routes from the URL maps.
  for pattern, slug in pairs(GET_ROUTES) do
    routes.GET[ctx.url(pattern)] = pages[slug]
  end
  for pattern, slug in pairs(POST_ROUTES) do
    routes.POST[ctx.url(pattern)] = pages[slug]
  end

  -- Wildcard dispatchers (page sub-paths http.serve can't pattern-match).
  routes.GET[ctx.url("/machines/*")]      = machines_get_wildcard
  routes.GET[ctx.url("/api/machines/*")]  = api_machines_get_wildcard

  return ctx
end

--- Internal helpers exposed for testing.
M._build_url = build_url

return M
