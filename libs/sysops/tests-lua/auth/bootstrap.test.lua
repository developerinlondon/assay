--! First-admin bootstrap tests.
--!
--! Run via:
--!   LUA_PATH='libs/?.lua;libs/?/init.lua;libs/sysops/?.lua;libs/sysops/tests-lua/?.lua;;' \
--!     assay libs/sysops/tests-lua/auth/bootstrap.test.lua

local ctx       = require("sysops.ctx")
local bootstrap = require("pages.auth.bootstrap")

print("[sysops.auth.bootstrap]")

-- ---------------------------------------------------------------------
-- Stub engine HTTP client (the sysops.auth zanzibar SDK calls
-- engine.get(path) / engine.post(path, body)). Record calls + script
-- responses keyed on method+path.
-- ---------------------------------------------------------------------

local function stub_engine(scripted)
  local calls = {}
  local function dispatch(method, path, body)
    table.insert(calls, { method = method, path = path, body = body })
    local handler = (scripted or {})[method .. " " .. path:gsub("%?.*$", "")]
    if handler then return handler(path, body) end
    return { status = 200, body = "{}" }
  end
  return {
    calls = calls,
    get    = function(p)    return dispatch("GET",    p) end,
    post   = function(p, b) return dispatch("POST",   p, b) end,
    put    = function(p, b) return dispatch("PUT",    p, b) end,
    delete = function(p)    return dispatch("DELETE", p) end,
  }
end

local TUPLES_PATH = "/api/v1/engine/auth/admin/zanzibar/tuples"

local function teardown()
  ctx.engine = nil
  ctx.authz_bootstrap_first_admin = true
  ctx.audit = nil
end

-- ---------------------------------------------------------------------
-- 1. No admins → bootstrap grants tuple.
-- ---------------------------------------------------------------------

do
  ctx.engine = stub_engine({
    ["GET "  .. TUPLES_PATH] = function() return { status = 200, body = json.encode({}) } end,
    ["POST " .. TUPLES_PATH] = function() return { status = 200, body = "{}" } end,
  })
  local r = bootstrap.maybe_grant_first_admin({ sub = "alice@example" })
  assert.eq(r, "granted", "returns 'granted' when it writes the tuple")

  -- Confirm POST happened with the right body.
  local saw_post
  for _, c in ipairs(ctx.engine.calls) do
    if c.method == "POST" and c.path:find(TUPLES_PATH, 1, true) then saw_post = c end
  end
  assert.not_nil(saw_post, "POST /tuples was called")
  assert.eq(saw_post.body.subject, "user:alice@example", "subject set")
  assert.eq(saw_post.body.relation, "admin", "relation set")
  assert.eq(saw_post.body.object_type, "engine", "object_type set")
  assert.eq(saw_post.body.object_id, "core", "object_id set")
  teardown()
  print("  ok grants admin tuple when none exist")
end

-- ---------------------------------------------------------------------
-- 2. Admins already exist → no-op.
-- ---------------------------------------------------------------------

do
  ctx.engine = stub_engine({
    ["GET " .. TUPLES_PATH] = function()
      return { status = 200, body = json.encode({
        { subject = "user:existing@example", relation = "admin",
          object_type = "engine", object_id = "core" },
      }) }
    end,
  })
  local r = bootstrap.maybe_grant_first_admin({ sub = "alice@example" })
  assert.eq(r, nil, "returns nil when admins already exist")
  -- Confirm no POST happened.
  for _, c in ipairs(ctx.engine.calls) do
    if c.method == "POST" then
      error("unexpected POST when admins exist: " .. c.path)
    end
  end
  teardown()
  print("  ok no-op when admins already exist")
end

-- ---------------------------------------------------------------------
-- 3. opt-out via authz_bootstrap_first_admin=false.
-- ---------------------------------------------------------------------

do
  ctx.engine = stub_engine({
    ["GET " .. TUPLES_PATH] = function() return { status = 200, body = json.encode({}) } end,
  })
  ctx.authz_bootstrap_first_admin = false
  local r = bootstrap.maybe_grant_first_admin({ sub = "alice@example" })
  assert.eq(r, nil, "no-op when opted out")
  assert.eq(#ctx.engine.calls, 0, "no engine traffic at all")
  teardown()
  print("  ok skipped when authz_bootstrap_first_admin=false")
end

-- ---------------------------------------------------------------------
-- 4. No engine wired → no-op (safe).
-- ---------------------------------------------------------------------

do
  ctx.engine = nil
  local r = bootstrap.maybe_grant_first_admin({ sub = "alice@example" })
  assert.eq(r, nil, "no-op when engine unwired")
  teardown()
  print("  ok no-op when engine unwired")
end

-- ---------------------------------------------------------------------
-- 5. Engine tuple-list errors → fail closed (treat as admins exist).
-- ---------------------------------------------------------------------

do
  ctx.engine = stub_engine({
    ["GET " .. TUPLES_PATH] = function() return { status = 405, body = "method not allowed" } end,
  })
  local r = bootstrap.maybe_grant_first_admin({ sub = "alice@example" })
  assert.eq(r, nil, "fail closed when listing errors")
  -- Confirm no POST attempted.
  for _, c in ipairs(ctx.engine.calls) do
    if c.method == "POST" then
      error("unexpected POST when list errored: " .. c.path)
    end
  end
  teardown()
  print("  ok fails closed when engine doesn't expose tuple listing")
end

-- ---------------------------------------------------------------------
-- 6. Audit logger is invoked on grant.
-- ---------------------------------------------------------------------

do
  ctx.engine = stub_engine({
    ["GET "  .. TUPLES_PATH] = function() return { status = 200, body = json.encode({}) } end,
    ["POST " .. TUPLES_PATH] = function() return { status = 200, body = "{}" } end,
  })
  local audited
  ctx.audit = {
    log = function(action, data) audited = { action = action, data = data } end,
  }
  bootstrap.maybe_grant_first_admin({ sub = "alice@example" })
  assert.eq(audited.action, "auth.bootstrap_first_admin", "audit action logged")
  assert.eq(audited.data.sub, "alice@example", "sub in audit data")
  teardown()
  print("  ok audit log written on grant")
end

print("[sysops.auth.bootstrap] ok")
