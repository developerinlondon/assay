--! sysops auth gateway — routes + ctx wiring.
--!
--! Called from mount.lua iff the consumer provides `opts.oidc`. Mounts:
--!
--!   GET  /auth/login         → kick off OIDC
--!   GET  /auth/callback      → finish OIDC + set cookie
--!   GET  /auth/logout        → clear cookie + revoke refresh
--!
--!   GET  /api/v1/engine/auth/whoami  → INTERCEPT (answers locally)
--!   ANY  /api/v1/engine/*    → PROXY to engine + bearer injection
--!
--!   GET  /workflow, /workflow/*       → PROXY (dashboard SPA assets)
--!   GET  /engine/console, /engine/console/*
--!   GET  /shared/*
--!
--! Backward-compat: mount.lua never calls this when opts.oidc is
--! absent, so consumers that don't opt in see exactly the 0.1.x
--! behaviour (admin bearer required at the engine layer).

local oidc    = require("sysops.oidc")
local session = require("sysops.session")
local gateway = require("sysops.gateway")
local ctx     = require("sysops.ctx")

local login_pg    = require("pages.auth.login")
local callback_pg = require("pages.auth.callback")
local logout_pg   = require("pages.auth.logout")

local M = {}

--- Validate opts.oidc / opts.session / opts.gateway and build the
--- client + signer + store, attaching them to ctx.
local function build_ctx(opts)
  -- OIDC client
  local o = opts.oidc
  if type(o) ~= "table" then
    error("sysops.mount: opts.oidc must be a table", 3)
  end
  if type(o.issuer) ~= "string" then
    error("sysops.mount: opts.oidc.issuer required", 3)
  end
  if type(o.client_id) ~= "string" then
    error("sysops.mount: opts.oidc.client_id required", 3)
  end
  ctx.oidc_client = oidc.new(o)

  -- Session signer
  local s = opts.session or {}
  ctx.session_signer = session.new({
    signing_key = s.signing_key,
    ttl_seconds = s.ttl_seconds,
    cookie_name = s.cookie_name,
  })
  ctx.session_store = session.store_new()

  -- Gateway config
  local g = opts.gateway or {}
  if type(g.engine_upstream) ~= "string" then
    error("sysops.mount: opts.gateway.engine_upstream required", 3)
  end
  if type(g.admin_bearer) ~= "string" then
    error("sysops.mount: opts.gateway.admin_bearer required", 3)
  end
  ctx.engine_base_url      = g.engine_upstream
  ctx.gateway_admin_bearer = g.admin_bearer

  -- Authz
  local a = opts.authz or {}
  if a.require_zanzibar_admin ~= nil then
    ctx.authz_require_admin = a.require_zanzibar_admin
  end
  if a.bootstrap_first_admin ~= nil then
    ctx.authz_bootstrap_first_admin = a.bootstrap_first_admin
  end
  -- ctx.zanzibar_check is left for the consumer or mount-time wiring
  -- to fill in if they want per-request role enforcement.
end

--- Register all the auth-gateway routes on the consumer's `routes`
--- table. `url` is the prefix-safe URL builder mount.lua exposes via
--- ctx.url so sysops mounted at /host/ also gets /host/auth/login etc.
local function register_routes(routes, url)
  routes.GET  = routes.GET  or {}
  routes.POST = routes.POST or {}
  routes.PUT  = routes.PUT  or {}
  routes.PATCH = routes.PATCH or {}
  routes.DELETE = routes.DELETE or {}

  -- OIDC dance.
  routes.GET[url("/auth/login")]    = login_pg.page
  routes.GET[url("/auth/callback")] = callback_pg.page
  routes.GET[url("/auth/logout")]   = logout_pg.page

  -- /whoami intercept (specific path; beats /api/v1/engine/* wildcard).
  routes.GET[url("/api/v1/engine/auth/whoami")] = gateway.whoami

  -- API proxy on every verb.
  for _, method in ipairs({ "GET", "POST", "PUT", "PATCH", "DELETE" }) do
    routes[method][url("/api/v1/engine/*")] = gateway.proxy
  end

  -- Dashboard SPA assets — engine serves the HTML/JS; we proxy through.
  routes.GET[url("/workflow")]            = gateway.proxy
  routes.GET[url("/workflow/*")]          = gateway.proxy
  routes.GET[url("/engine/console")]      = gateway.proxy
  routes.GET[url("/engine/console/*")]    = gateway.proxy
  routes.GET[url("/shared/*")]            = gateway.proxy
end

--- Routes that must stay unwrapped even after the gateway opt-in:
--- public assets and healthchecks. Everything else gets gated.
local function is_public_path(path)
  if path:match("^/static/") then return true end
  if path:match("^/brand/") then return true end
  if path == "/healthz" then return true end
  if path == "/favicon.ico" then return true end
  return false
end

--- Wrap every route already registered (sysops dashboard, machines,
--- services, /auth/users, /vault/kv, /zanzibar/*, etc.) with
--- require_session so the browser must sign in before reaching any of
--- it. Called BEFORE register_routes adds the gateway's own routes —
--- so /auth/login, /auth/callback, /whoami, the proxy paths etc.
--- stay unwrapped.
local function wrap_existing_routes(routes)
  local require_session = require("sysops.middleware.require_session")
  for _, method in ipairs({ "GET", "POST", "PUT", "PATCH", "DELETE" }) do
    local tbl = routes[method]
    if type(tbl) == "table" then
      for path, handler in pairs(tbl) do
        if not is_public_path(path) and type(handler) == "function" then
          tbl[path] = require_session.wrap(handler)
        end
      end
    end
  end
end

--- Public entry — mount.lua calls this iff opts.oidc is set.
function M.register(routes, url, opts)
  build_ctx(opts)
  wrap_existing_routes(routes)
  register_routes(routes, url)
end

-- Internal helpers exposed for tests.
M._build_ctx            = build_ctx
M._register_routes      = register_routes
M._wrap_existing_routes = wrap_existing_routes
M._is_public_path       = is_public_path

return M
