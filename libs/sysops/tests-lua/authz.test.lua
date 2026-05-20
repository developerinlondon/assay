--! sysops.authz tests — per-resource permission lookup + cache.
--!
--! Run via:
--!   LUA_PATH='libs/?.lua;libs/?/init.lua;libs/sysops/?.lua;libs/sysops/tests-lua/?.lua;;' \
--!     assay libs/sysops/tests-lua/authz.test.lua

local ctx   = require("sysops.ctx")
local authz = require("sysops.authz")

print("[sysops.authz]")

-- ---------------------------------------------------------------------
-- Path-rule mapping (pure lookup, no engine).
-- ---------------------------------------------------------------------

do
  local r
  r = authz.rule_for_path("/auth/login")
  assert.eq(r.bypass, true, "/auth/login bypasses authz")
  r = authz.rule_for_path("/auth/callback")
  assert.eq(r.bypass, true, "/auth/callback bypasses authz")
  r = authz.rule_for_path("/api/v1/engine/auth/whoami")
  assert.eq(r.bypass, true, "/whoami bypasses authz")
  r = authz.rule_for_path("/static/styles.css")
  assert.eq(r.bypass, true, "/static/* bypasses authz")
  r = authz.rule_for_path("/healthz")
  assert.eq(r.bypass, true, "/healthz bypasses authz")
  print("  ok bypass paths recognised")
end

do
  local r
  r = authz.rule_for_path("/api/v1/engine/auth/admin/users")
  assert.eq(r.object_type, "auth", "/api/v1/engine/auth/* → auth")
  assert.eq(r.relation, "admin", "auth requires admin")

  r = authz.rule_for_path("/api/v1/engine/core/info")
  assert.eq(r.object_type, "engine", "/api/v1/engine/core/* → engine")

  r = authz.rule_for_path("/api/v1/engine/workflow/runs")
  assert.eq(r.object_type, "workflow", "/api/v1/engine/workflow/* → workflow")
  assert.eq(r.relation, "access", "workflow requires access")

  r = authz.rule_for_path("/api/v1/vault/sys/seal-status")
  assert.eq(r.object_type, "vault", "/api/v1/vault/* → vault")

  print("  ok API paths map to canonical resources")
end

do
  local r
  r = authz.rule_for_path("/engine/console")
  assert.eq(r.object_type, "engine", "engine console pane → engine")
  r = authz.rule_for_path("/vault/console")
  assert.eq(r.object_type, "vault", "vault console pane → vault")
  r = authz.rule_for_path("/workflow")
  assert.eq(r.object_type, "workflow", "workflow pane → workflow")
  r = authz.rule_for_path("/auth/users")
  assert.eq(r.object_type, "auth", "sysops /auth/users → auth")
  r = authz.rule_for_path("/zanzibar")
  assert.eq(r.object_type, "auth", "/zanzibar* → auth")
  -- Shared assets stay public-after-auth.
  r = authz.rule_for_path("/auth/style.css")
  assert.eq(r.bypass, true, "/auth/style.css is shared asset, bypass")
  r = authz.rule_for_path("/engine/app.js")
  assert.eq(r.bypass, true, "/engine/app.js is asset, bypass")
  print("  ok SPA shells + sysops pages map correctly; assets stay public")
end

-- ---------------------------------------------------------------------
-- is_allowed: per-tuple Zanzibar checks via stubbed engine.
-- ---------------------------------------------------------------------

local function stub_engine(allowed_tuples)
  -- allowed_tuples = { ["sub|object_type:object_id#relation"] = true, … }
  local calls = {}
  local function dispatch(method, path, body)
    table.insert(calls, { method = method, path = path, body = body })
    if path:match("/zanzibar/check") and body then
      -- New engine request shape: split subject + resource + permission.
      local key = "user:" .. body.subject_id ..
                  "|" .. body.resource_type .. ":" .. body.resource_id ..
                  "#" .. body.permission
      return { status = 200, body = json.encode({ allowed = allowed_tuples[key] == true }) }
    end
    return { status = 200, body = "{}" }
  end
  return {
    calls  = calls,
    get    = function(p)    return dispatch("GET", p) end,
    post   = function(p, b) return dispatch("POST", p, b) end,
    put    = function(p, b) return dispatch("PUT", p, b) end,
    delete = function(p)    return dispatch("DELETE", p) end,
  }
end

local function reset()
  ctx.engine = nil
  authz.invalidate()
end

do
  reset()
  ctx.engine = stub_engine({})
  local ok, reason = authz.is_allowed("alice@example", "/api/v1/engine/auth/whoami")
  assert.eq(ok, true, "whoami is bypassed for any signed-in user")
  assert.eq(reason, "bypass", "reason=bypass")
  -- No engine call for bypass paths.
  assert.eq(#ctx.engine.calls, 0, "no engine call for bypass")
  reset()
  print("  ok bypass paths don't hit the engine")
end

do
  reset()
  ctx.engine = stub_engine({
    ["user:alice@example|workflow:main#access"] = true,
  })
  local ok, reason = authz.is_allowed("alice@example", "/api/v1/engine/workflow/runs")
  assert.eq(ok, true, "alice has workflow access")
  assert.eq(reason, "granted", "reason=granted")
  reset()
  print("  ok granted tuple allows the request")
end

do
  reset()
  ctx.engine = stub_engine({}) -- nobody allowed
  local ok, reason = authz.is_allowed("bob@example", "/api/v1/engine/workflow/runs")
  assert.eq(ok, false, "bob has no workflow access → denied")
  assert.eq(reason, "missing-tuple", "reason=missing-tuple")
  reset()
  print("  ok missing tuple denies the request")
end

do
  reset()
  local stub = stub_engine({
    ["user:alice@example|engine:core#admin"] = true,
  })
  ctx.engine = stub
  -- Two calls, second should be cache hit.
  authz.is_allowed("alice@example", "/api/v1/engine/core/info")
  authz.is_allowed("alice@example", "/api/v1/engine/core/modules")
  local check_calls = 0
  for _, c in ipairs(stub.calls) do
    if c.path:match("/zanzibar/check") then check_calls = check_calls + 1 end
  end
  assert.eq(check_calls, 1, "second engine:core call is cached, one zanzibar check total")
  reset()
  print("  ok cache reuses the per-tuple decision across paths in the same resource")
end

do
  reset()
  local stub = stub_engine({
    ["user:alice@example|engine:core#admin"]   = true,
    ["user:alice@example|vault:main#access"]   = false,
  })
  ctx.engine = stub
  assert.eq(authz.is_allowed("alice@example", "/engine/console"), true, "engine allowed")
  assert.eq(authz.is_allowed("alice@example", "/vault/console"), false, "vault denied")
  reset()
  print("  ok partial permissions: engine-yes, vault-no")
end

do
  reset()
  -- No engine wired → fail closed.
  local ok, reason = authz.is_allowed("alice@example", "/api/v1/engine/workflow/runs")
  assert.eq(ok, false, "no engine → denied")
  assert.eq(reason, "no-engine", "reason=no-engine")
  reset()
  print("  ok fails closed when engine not configured")
end

do
  reset()
  ctx.engine = stub_engine({})
  -- Empty sub → denied.
  local ok, reason = authz.is_allowed("", "/api/v1/engine/workflow/runs")
  assert.eq(ok, false, "empty sub → denied")
  assert.eq(reason, "no-sub", "reason=no-sub")
  reset()
  print("  ok empty sub denied")
end

-- ---------------------------------------------------------------------
-- invalidate() clears cache so a freshly-granted tuple takes effect.
-- ---------------------------------------------------------------------

do
  reset()
  local current_allowed = false
  local stub = {
    calls = {},
    get = function() return { status = 200, body = "{}" } end,
    post = function(p, b)
      if p:match("/zanzibar/check") then
        return { status = 200, body = json.encode({ allowed = current_allowed }) }
      end
      return { status = 200, body = "{}" }
    end,
  }
  ctx.engine = stub
  -- First check: not allowed. Cached.
  assert.eq(authz.is_allowed("alice", "/vault/console"), false, "initial: denied")
  -- Now grant in the stub but cache still has 'denied'.
  current_allowed = true
  assert.eq(authz.is_allowed("alice", "/vault/console"), false,
            "still denied — cache hit on previous result")
  -- Invalidate; should now allow.
  authz.invalidate()
  assert.eq(authz.is_allowed("alice", "/vault/console"), true,
            "after invalidate: allowed")
  reset()
  print("  ok invalidate() clears cache so newly-granted tuples take effect")
end

-- ---------------------------------------------------------------------
-- Strict-prefix fail-closed: unmapped /api/v1/* etc. MUST be denied.
-- ---------------------------------------------------------------------

do
  reset()
  ctx.engine = stub_engine({})
  -- A path under /api/v1/ that no rule matches. Must NOT be no-rule allow.
  local ok, reason = authz.is_allowed("alice@example", "/api/v1/engine/some-future-endpoint/x")
  assert.eq(ok, false, "unmapped /api/v1/* → denied")
  assert.eq(reason, "no-rule-strict", "reason flags strict fail-closed")
  -- A path that legitimately has no rule (consumer-added) stays open.
  local ok2 = authz.is_allowed("alice@example", "/skip-trace")
  assert.eq(ok2, true, "unmapped non-sensitive path stays no-rule open")
  reset()
  print("  ok unmapped /api/v1/* fails closed; unmapped /skip-trace stays open")
end

-- ---------------------------------------------------------------------
-- Mount-prefix awareness: /host/auth/users matches the /auth/users rule
-- when ctx.prefix is /host.
-- ---------------------------------------------------------------------

do
  reset()
  ctx.engine = stub_engine({
    ["user:alice@example|auth:system#admin"] = true,
  })
  ctx.prefix = "/host"
  assert.eq(authz.is_allowed("alice@example", "/host/auth/users"), true,
            "/host/auth/users routes to /auth/users rule when prefix=/host")
  assert.eq(authz.is_allowed("alice@example", "/host/api/v1/engine/auth/admin/users"),
            true,
            "prefix-stripped /api/v1/* path also resolves")
  ctx.prefix = nil
  reset()
  print("  ok mount prefix is stripped before rule matching")
end

print("[sysops.authz] ok")
