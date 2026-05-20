--! sysops.gateway.proxy tests — dual-mode reverse proxy.
--!
--! Run via:
--!   LUA_PATH='libs/?.lua;libs/?/init.lua;libs/sysops/?.lua;libs/sysops/tests-lua/?.lua;;' \
--!     assay libs/sysops/tests-lua/gateway_proxy.test.lua

local ctx     = require("sysops.ctx")
local session = require("sysops.session")
local gateway = require("sysops.gateway")

print("[sysops.gateway.proxy]")

local KEY = "0123456789abcdef0123456789abcdef"
local original_http = http

-- ---------------------------------------------------------------------
-- Override the `http` global with a recorder + scripted response.
-- ---------------------------------------------------------------------

local function install_http(scripted)
  local calls = {}
  local function dispatch(method, url_str, body_or_opts, maybe_opts)
    local opts, body
    -- get/delete: (url, opts).  post/put/patch: (url, body, opts).
    if method == "get" or method == "delete" then
      opts = body_or_opts
    else
      body = body_or_opts
      opts = maybe_opts
    end
    table.insert(calls, {
      method  = method,
      url     = url_str,
      headers = (opts and opts.headers) or {},
      body    = body,
    })
    return scripted(method, url_str, opts) or { status = 200, body = "{}" }
  end
  http = {
    get    = function(u, o)     return dispatch("get",    u, o) end,
    post   = function(u, b, o)  return dispatch("post",   u, b, o) end,
    put    = function(u, b, o)  return dispatch("put",    u, b, o) end,
    patch  = function(u, b, o)  return dispatch("patch",  u, b, o) end,
    delete = function(u, o)     return dispatch("delete", u, o) end,
  }
  return calls
end

local function restore_http()
  http = original_http
end

local function setup(opts)
  opts = opts or {}
  ctx.session_signer = session.new({
    signing_key = KEY,
    ttl_seconds = 3600,
    cookie_name = "gondor_session",
  })
  ctx.session_store        = session.store_new()
  ctx.engine_base_url      = opts.engine_base_url or "http://127.0.0.1:8080"
  ctx.gateway_admin_bearer = opts.admin_bearer or "ADMIN-BEARER-TOKEN"
  ctx.authz_require_admin  = opts.authz_require_admin or false
  ctx.zanzibar_check       = opts.zanzibar_check
end

local function teardown()
  ctx.session_signer       = nil
  ctx.session_store        = nil
  ctx.engine_base_url      = nil
  ctx.gateway_admin_bearer = nil
  ctx.authz_require_admin  = false
  ctx.zanzibar_check       = nil
  restore_http()
end

-- ---------------------------------------------------------------------
-- 1. Session-injection: cookie + no Authorization → inject admin bearer.
-- ---------------------------------------------------------------------

do
  setup()
  local calls = install_http(function(method, url_str, opts)
    return { status = 200, body = '{"runs":[]}' }
  end)

  local cookie = ctx.session_signer:issue({ sub = "alice@example", email = "alice@example" })
  local r = gateway.proxy({
    method = "GET",
    path   = "/api/v1/engine/workflow/runs",
    headers = { cookie = "gondor_session=" .. cookie },
  })

  assert.eq(r.status, 200, "forwarded successfully")
  assert.eq(#calls, 1, "one upstream call")
  assert.eq(calls[1].method, "get", "method preserved")
  assert.eq(calls[1].url, "http://127.0.0.1:8080/api/v1/engine/workflow/runs",
            "URL preserved")
  assert.eq(calls[1].headers.authorization, "Bearer ADMIN-BEARER-TOKEN",
            "admin bearer injected")
  assert.eq(calls[1].headers["X-User-Id"], "alice@example", "X-User-Id set")
  assert.eq(calls[1].headers.cookie, nil, "Cookie header stripped (never leak to engine)")
  teardown()
  print("  ok session-injection mode: admin bearer + X-User-Id")
end

-- ---------------------------------------------------------------------
-- 2. Pass-through: caller has Authorization → forward unchanged.
-- ---------------------------------------------------------------------

do
  setup()
  local calls = install_http(function(method, url_str, opts)
    return { status = 200, body = '{"ok":true}' }
  end)

  local r = gateway.proxy({
    method = "GET",
    path   = "/api/v1/engine/workflow/runs",
    headers = { authorization = "Bearer CALLER-OWN-TOKEN" },
  })

  assert.eq(r.status, 200, "forwarded successfully")
  assert.eq(calls[1].headers.authorization, "Bearer CALLER-OWN-TOKEN",
            "caller's bearer preserved — NOT replaced by admin bearer")
  teardown()
  print("  ok pass-through mode: caller's bearer preserved")
end

-- ---------------------------------------------------------------------
-- 3. No session, no bearer → 401 (no upstream call).
-- ---------------------------------------------------------------------

do
  setup()
  local calls = install_http(function() return { status = 999, body = "should not be called" } end)
  local r = gateway.proxy({
    method = "GET",
    path   = "/api/v1/engine/workflow/runs",
  })
  assert.eq(r.status, 401, "no creds → 401")
  assert.eq(#calls, 0, "no upstream call when unauthenticated")
  teardown()
  print("  ok unauthenticated → 401 without upstream call")
end

-- ---------------------------------------------------------------------
-- 4. POST body + query string propagate to upstream.
-- ---------------------------------------------------------------------

do
  setup()
  local calls = install_http(function() return { status = 201, body = "{}" } end)

  local cookie = ctx.session_signer:issue({ sub = "alice@example" })
  local r = gateway.proxy({
    method   = "POST",
    path      = "/api/v1/engine/auth/admin/users",
    raw_query = "limit=10",
    body      = '{"email":"bob@example"}',
    headers   = { cookie = "gondor_session=" .. cookie },
  })
  assert.eq(r.status, 201, "passes through 201")
  assert.eq(calls[1].method, "post", "post dispatched")
  assert.eq(calls[1].url,
            "http://127.0.0.1:8080/api/v1/engine/auth/admin/users?limit=10",
            "URL has query string")
  assert.eq(calls[1].body, '{"email":"bob@example"}', "body propagated")
  teardown()
  print("  ok POST body + query string propagate")
end

-- ---------------------------------------------------------------------
-- 5. Zanzibar gate: configured + check returns false → 403.
-- ---------------------------------------------------------------------

do
  local seen_sub
  setup({
    authz_require_admin = true,
    zanzibar_check = function(sub) seen_sub = sub; return false end,
  })
  install_http(function() return { status = 200, body = "should not reach engine" } end)

  local cookie = ctx.session_signer:issue({ sub = "bob@example" })
  local r = gateway.proxy({
    method = "GET",
    path   = "/api/v1/engine/auth/admin/users",
    headers = { cookie = "gondor_session=" .. cookie },
  })
  assert.eq(r.status, 403, "non-admin → 403")
  assert.eq(seen_sub, "bob@example", "zanzibar_check called with the sub")
  teardown()
  print("  ok zanzibar denies non-admin")
end

-- ---------------------------------------------------------------------
-- 6. Zanzibar gate: check returns true → request proxied.
-- ---------------------------------------------------------------------

do
  setup({
    authz_require_admin = true,
    zanzibar_check = function(_) return true end,
  })
  local calls = install_http(function() return { status = 200, body = "{}" } end)

  local cookie = ctx.session_signer:issue({ sub = "alice@example" })
  local r = gateway.proxy({
    method = "GET",
    path   = "/api/v1/engine/auth/admin/users",
    headers = { cookie = "gondor_session=" .. cookie },
  })
  assert.eq(r.status, 200, "admin succeeds")
  assert.eq(#calls, 1, "request reached engine")
  teardown()
  print("  ok zanzibar admins are allowed through")
end

-- ---------------------------------------------------------------------
-- 7. Zanzibar gate misconfigured (require_admin=true, no check fn) → 503.
-- ---------------------------------------------------------------------

do
  setup({ authz_require_admin = true })
  install_http(function() return { status = 200, body = "{}" } end)

  local cookie = ctx.session_signer:issue({ sub = "alice@example" })
  local r = gateway.proxy({
    method = "GET",
    path   = "/api/v1/engine/auth/admin/users",
    headers = { cookie = "gondor_session=" .. cookie },
  })
  assert.eq(r.status, 503, "misconfig → 503")
  assert.not_nil(r.body:find("zanzibar_check", 1, true), "error message names the missing wiring")
  teardown()
  print("  ok misconfigured authz fails closed (503)")
end

-- ---------------------------------------------------------------------
-- 8. Hop-by-hop headers stripped both ways.
-- ---------------------------------------------------------------------

do
  setup()
  local calls = install_http(function()
    return {
      status = 200,
      body = "{}",
      headers = { ["Transfer-Encoding"] = "chunked", ["Content-Type"] = "application/json" },
    }
  end)

  local cookie = ctx.session_signer:issue({ sub = "alice@example" })
  local r = gateway.proxy({
    method = "GET",
    path   = "/api/v1/engine/workflow/runs",
    headers = {
      cookie     = "gondor_session=" .. cookie,
      Connection = "keep-alive",
    },
  })
  assert.eq(calls[1].headers.Connection, nil, "Connection stripped in upstream req")
  assert.eq(r.headers["Transfer-Encoding"], nil, "Transfer-Encoding stripped in downstream resp")
  assert.eq(r.headers["Content-Type"], "application/json", "Content-Type preserved")
  teardown()
  print("  ok hop-by-hop headers stripped (req + resp)")
end

-- ---------------------------------------------------------------------
-- 9b. Session present + stale bearer in Authorization → cookie wins.
--     Regression for the dashboard SPA case where localStorage holds
--     an `assay-admin-token` that long predates the OIDC flow.
-- ---------------------------------------------------------------------

do
  setup()
  local calls = install_http(function() return { status = 200, body = '{}' } end)
  local cookie = ctx.session_signer:issue({ sub = "alice@example", email = "alice@example" })
  local r = gateway.proxy({
    method   = "GET",
    path     = "/api/v1/engine/workflow/runs",
    headers  = {
      cookie        = "gondor_session=" .. cookie,
      authorization = "Bearer STALE-LOCALSTORAGE-TOKEN",
    },
  })
  assert.eq(r.status, 200, "valid session beats stale bearer")
  assert.eq(calls[1].headers.authorization, "Bearer ADMIN-BEARER-TOKEN",
            "session-injected admin bearer used; stale token discarded")
  teardown()
  print("  ok valid session overrides a stale Authorization header")
end

-- ---------------------------------------------------------------------
-- 9. Not configured → 503.
-- ---------------------------------------------------------------------

do
  teardown() -- ensure ctx is empty
  local r = gateway.proxy({ method = "GET", path = "/api/v1/engine/x" })
  assert.eq(r.status, 503, "no signer → 503")
  print("  ok 503 when not configured")
end

print("[sysops.gateway.proxy] ok")
