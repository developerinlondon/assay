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
--!     engine = engine_http_client,            -- HTTP wrapper, see plan
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

local ctx = require("hostops.ctx")

local M = {}

--- Build a prefix-safe URL helper for routes mounted at `prefix`.
--- Exposed as `hostops.url(p)` after mount, and on `ctx.url`.
local function build_url(prefix)
  prefix = prefix or "/"
  -- Normalise: strip trailing slash unless it IS "/".
  if prefix ~= "/" and prefix:sub(-1) == "/" then
    prefix = prefix:sub(1, -2)
  end
  -- "/" prefix means "no prepending": `url("/foo")` → "/foo".
  if prefix == "/" then prefix = "" end

  return function(path)
    path = path or "/"
    if path:sub(1, 1) ~= "/" then path = "/" .. path end
    if prefix == "" then return path end
    -- url("/") at a non-root prefix should resolve to the prefix itself.
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

--- Validate opts and populate the shared ctx module.
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

  -- Phase 3.3 wires the route attachment: pages.lua + api.lua will
  -- expose `attach(routes, url)` once their top-level service requires
  -- have been replaced with ctx reads. For phase 3.2 the contract is
  -- established; the route registration plumbing follows.
  -- TODO(plan-21 phase 3.3): attach pages + api routes here.
  return ctx
end

--- Build-only helper exposed for testing the URL prefix logic in
--- isolation. Not part of the public mount() contract.
M._build_url = build_url

return M
