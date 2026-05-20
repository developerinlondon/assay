--! sysops.mount() — auth-gateway opt-in wiring.
--!
--! Run via:
--!   LUA_PATH='libs/?.lua;libs/?/init.lua;libs/sysops/?.lua;libs/sysops/tests-lua/?.lua;;' \
--!     assay libs/sysops/tests-lua/mount_auth_gateway.test.lua

local sysops = require("sysops.mount")
local stubs  = require("stubs")
local ctx    = require("sysops.ctx")

print("[sysops.mount.auth_gateway]")

-- ---------------------------------------------------------------------
-- Helper — full opts table including the four auth-gateway blocks.
-- ---------------------------------------------------------------------

local function full_opts(overrides)
  overrides = overrides or {}
  local base = stubs.opts()
  base.oidc = overrides.oidc or {
    issuer       = "https://idp.test",
    client_id    = "sysops",
    redirect_uri = "https://gondor.fcar.ai/auth/callback",
  }
  base.session = overrides.session or {
    signing_key = "0123456789abcdef0123456789abcdef",
    ttl_seconds = 86400,
    cookie_name = "gondor_session",
  }
  base.gateway = overrides.gateway or {
    engine_upstream = "http://127.0.0.1:8080",
    admin_bearer   = "TEST-ADMIN-BEARER",
  }
  base.authz = overrides.authz
  return base
end

-- Reset ctx so tests don't bleed.
local function reset_ctx()
  ctx.oidc_client                = nil
  ctx.session_signer             = nil
  ctx.session_store              = nil
  ctx.engine_base_url            = nil
  ctx.engine_upstream_url        = nil
  ctx.gateway_admin_bearer       = nil
  ctx.authz_require_admin        = false
  ctx.authz_bootstrap_first_admin = true
  ctx.zanzibar_check             = nil
end

-- Stub http so the OIDC client doesn't actually do discovery during
-- mount (mount itself doesn't call discover, but defensively stub).
local original_http = http
http = {
  get = function(_) return { status = 200, body = '{}' } end,
}

-- ---------------------------------------------------------------------
-- 1. opt-in via opts.oidc registers all gateway routes.
-- ---------------------------------------------------------------------

do
  reset_ctx()
  local routes = { GET = {}, POST = {} }
  sysops.mount(routes, full_opts())

  -- OIDC dance.
  assert.not_nil(routes.GET["/auth/login"],    "/auth/login registered")
  assert.not_nil(routes.GET["/auth/callback"], "/auth/callback registered")
  assert.not_nil(routes.GET["/auth/logout"],   "/auth/logout registered")

  -- /whoami intercept (specific path).
  assert.not_nil(routes.GET["/api/v1/engine/auth/whoami"], "/whoami intercept registered")

  -- API proxy on every verb — two roots (engine + vault).
  for _, method in ipairs({ "GET", "POST", "PUT", "PATCH", "DELETE" }) do
    assert.not_nil(routes[method]["/api/v1/engine/*"],
                   method .. " /api/v1/engine/* registered")
    assert.not_nil(routes[method]["/api/v1/vault/*"],
                   method .. " /api/v1/vault/* registered")
  end

  -- Dashboard SPA + cross-console asset wildcards.
  assert.not_nil(routes.GET["/auth/*"],     "/auth/* registered (proxy fallback)")
  assert.not_nil(routes.GET["/vault/*"],    "/vault/* registered")
  assert.not_nil(routes.GET["/workflow"],   "/workflow registered")
  assert.not_nil(routes.GET["/workflow/*"], "/workflow/* registered")
  assert.not_nil(routes.GET["/engine/*"],   "/engine/* registered")
  assert.not_nil(routes.GET["/shared/*"],   "/shared/* registered")

  -- ctx fields populated.
  assert.not_nil(ctx.oidc_client,          "oidc_client built")
  assert.not_nil(ctx.session_signer,       "session_signer built")
  assert.not_nil(ctx.session_store,        "session_store built")
  assert.eq(ctx.engine_upstream_url, "http://127.0.0.1:8080", "engine_upstream_url set")
  assert.eq(ctx.gateway_admin_bearer, "TEST-ADMIN-BEARER", "admin_bearer set")
  assert.eq(ctx.session_signer.cookie_name, "gondor_session", "cookie_name propagated")

  reset_ctx()
  print("  ok opts.oidc opt-in wires every gateway route + ctx field")
end

-- ---------------------------------------------------------------------
-- 2. backward-compat: no opts.oidc → no gateway routes, no ctx fields.
-- ---------------------------------------------------------------------

do
  reset_ctx()
  local routes = { GET = {}, POST = {} }
  sysops.mount(routes, stubs.opts())   -- the legacy 0.1.x opts shape

  -- Auth-gateway routes NOT registered.
  assert.eq(routes.GET["/auth/login"], nil, "no /auth/login when oidc absent")
  assert.eq(routes.GET["/auth/callback"], nil, "no /auth/callback when oidc absent")
  assert.eq(routes.GET["/api/v1/engine/auth/whoami"], nil,
            "no /whoami when oidc absent")
  assert.eq(routes.GET["/api/v1/engine/*"], nil,
            "no proxy when oidc absent")

  -- ctx auth-gateway fields stay nil.
  assert.eq(ctx.oidc_client, nil, "no oidc_client when opted out")
  assert.eq(ctx.session_signer, nil, "no session_signer when opted out")
  assert.eq(ctx.gateway_admin_bearer, nil, "no admin_bearer when opted out")

  reset_ctx()
  print("  ok no opts.oidc → existing 0.1.x behaviour, no auth-gateway routes")
end

-- ---------------------------------------------------------------------
-- 3. opts.authz overrides propagate to ctx.
-- ---------------------------------------------------------------------

do
  reset_ctx()
  local routes = { GET = {}, POST = {} }
  sysops.mount(routes, full_opts({
    authz = {
      require_zanzibar_admin = true,
      bootstrap_first_admin  = false,
    },
  }))
  assert.eq(ctx.authz_require_admin, true, "require_zanzibar_admin propagated")
  assert.eq(ctx.authz_bootstrap_first_admin, false, "bootstrap_first_admin propagated")
  reset_ctx()
  print("  ok opts.authz overrides flow into ctx")
end

-- ---------------------------------------------------------------------
-- 4. missing opts.oidc.issuer is rejected with a clear error.
-- ---------------------------------------------------------------------

do
  reset_ctx()
  local routes = { GET = {}, POST = {} }
  local ok, err = pcall(sysops.mount, routes, full_opts({
    oidc = { client_id = "sysops" }, -- no issuer
  }))
  assert.eq(ok, false, "mount errored")
  assert.not_nil(tostring(err):find("opts.oidc.issuer required", 1, true),
                 "error message names the missing field")
  reset_ctx()
  print("  ok missing opts.oidc.issuer is rejected with a clear error")
end

-- ---------------------------------------------------------------------
-- 5. missing opts.gateway.admin_bearer is rejected.
-- ---------------------------------------------------------------------

do
  reset_ctx()
  local routes = { GET = {}, POST = {} }
  local ok, err = pcall(sysops.mount, routes, full_opts({
    gateway = { engine_upstream = "http://127.0.0.1:8080" }, -- no bearer
  }))
  assert.eq(ok, false, "mount errored")
  assert.not_nil(tostring(err):find("opts.gateway.admin_bearer required", 1, true),
                 "error message names the missing field")
  reset_ctx()
  print("  ok missing opts.gateway.admin_bearer is rejected")
end

-- ---------------------------------------------------------------------
-- 6. mount with prefix prepends to gateway routes.
-- ---------------------------------------------------------------------

do
  reset_ctx()
  local routes = { GET = {}, POST = {} }
  sysops.mount(routes, (function()
    local o = full_opts()
    o.prefix = "/host"
    return o
  end)())
  assert.not_nil(routes.GET["/host/auth/login"],
                 "prefix prepended to /auth/login")
  assert.not_nil(routes.GET["/host/api/v1/engine/auth/whoami"],
                 "prefix prepended to /whoami")
  assert.not_nil(routes.GET["/host/api/v1/engine/*"],
                 "prefix prepended to proxy wildcard")
  reset_ctx()
  print("  ok mount prefix prepended to gateway routes")
end

-- ---------------------------------------------------------------------
-- 7. Existing sysops pages get gated; public paths stay open.
-- ---------------------------------------------------------------------

do
  reset_ctx()
  -- Install a discovery-capable http stub BEFORE sysops.mount runs so
  -- the OIDC client's lazy-discover succeeds when /auth/login fires.
  http = { get = function(_)
    return {
      status = 200,
      body   = json.encode({
        authorization_endpoint = "https://idp.test/auth/authorize",
        token_endpoint         = "https://idp.test/auth/token",
        jwks_uri               = "https://idp.test/auth/jwks",
      }),
    }
  end }

  local routes = { GET = {}, POST = {} }
  -- Pre-register some routes BEFORE mount runs so we can verify the
  -- wrap-existing-routes pass picks them up.
  routes.GET["/"]                  = function() return { status = 200, body = "dashboard" } end
  routes.GET["/static/styles.css"] = function() return { status = 200, body = "css" } end
  routes.GET["/healthz"]           = function() return { status = 200, body = "ok" } end

  sysops.mount(routes, full_opts())

  -- Dashboard `/` should be wrapped → 302 without cookie.
  local r1 = routes.GET["/"]({ path = "/" })
  assert.eq(r1.status, 302, "/ wrapped: 302 to /auth/login when unauthenticated")
  assert.not_nil(r1.headers.Location:find("/auth/login", 1, true),
                 "Location is /auth/login?return_to=...")

  -- Static asset stays open.
  local r2 = routes.GET["/static/styles.css"]({ path = "/static/styles.css" })
  assert.eq(r2.status, 200, "/static/* stays public")

  -- /healthz stays open.
  local r3 = routes.GET["/healthz"]({ path = "/healthz" })
  assert.eq(r3.status, 200, "/healthz stays public")

  -- Gateway's own /auth/login is NOT wrapped — it 302s to the IdP, not
  -- back to itself (a wrapped login would loop).
  local r4 = routes.GET["/auth/login"]({ params = { return_to = "/" } })
  assert.eq(r4.status, 302, "/auth/login returns 302 to IdP (not wrapped)")
  assert.not_nil(r4.headers.Location:find("idp.test/auth/authorize", 1, true),
                 "/auth/login redirects to IdP, not back to /auth/login")
  http = original_http

  reset_ctx()
  print("  ok existing routes wrapped; public paths stay open")
end

-- Restore real http for any later tests.
http = original_http

print("[sysops.mount.auth_gateway] ok")
