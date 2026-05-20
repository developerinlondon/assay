--! sysops.middleware.require_session tests.
--!
--! Run via:
--!   LUA_PATH='libs/?.lua;libs/?/init.lua;libs/sysops/?.lua;libs/sysops/tests-lua/?.lua;;' \
--!     assay libs/sysops/tests-lua/require_session.test.lua

local ctx              = require("sysops.ctx")
local session          = require("sysops.session")
local require_session  = require("sysops.middleware.require_session")
local authz            = require("sysops.authz")

print("[sysops.middleware.require_session]")

local KEY = "0123456789abcdef0123456789abcdef"

local function setup()
  ctx.session_signer = session.new({
    signing_key = KEY,
    ttl_seconds = 3600,
    cookie_name = "gondor_session",
  })
  -- Authz allow-all so the middleware focus stays on session semantics.
  -- authz.test.lua exercises the per-resource paths directly.
  ctx.engine = {
    get  = function(_)    return { status = 200, body = "{}" } end,
    post = function(_, _) return {
      status = 200, body = json.encode({ allowed = true }),
    } end,
  }
  authz.invalidate()
end

local function teardown()
  ctx.session_signer = nil
  ctx.engine = nil
  authz.invalidate()
end

local function counting_inner()
  local called_with
  local fn = function(req)
    called_with = req
    return { status = 200, body = "inner-ran" }
  end
  return fn, function() return called_with end
end

-- ---------------------------------------------------------------------
-- 1. No cookie → 302 /auth/login?return_to=<encoded path>.
-- ---------------------------------------------------------------------

do
  setup()
  local inner, last = counting_inner()
  local wrapped = require_session.wrap(inner)
  local r = wrapped({ path = "/auth/users" })
  assert.eq(r.status, 302, "no cookie → 302")
  assert.eq(r.headers.Location, "/auth/login?return_to=%2Fauth%2Fusers", "return_to encoded")
  assert.eq(last(), nil, "inner NOT called when unauthenticated")
  teardown()
  print("  ok unauthenticated → 302 /auth/login")
end

-- ---------------------------------------------------------------------
-- 2. Valid cookie → inner runs + receives session_claims.
-- ---------------------------------------------------------------------

do
  setup()
  local cookie = ctx.session_signer:issue({ sub = "alice@example", email = "alice@example" })
  local inner, last = counting_inner()
  local wrapped = require_session.wrap(inner)
  local r = wrapped({
    path    = "/auth/users",
    headers = { cookie = "gondor_session=" .. cookie },
  })
  assert.eq(r.status, 200, "inner response passed through")
  assert.eq(r.body, "inner-ran", "inner ran")
  local passed = last()
  assert.eq(passed.session_claims.sub, "alice@example", "claims attached to req")
  assert.eq(passed.session_claims.email, "alice@example", "email in claims")
  teardown()
  print("  ok valid cookie → inner runs with claims attached")
end

-- ---------------------------------------------------------------------
-- 3. Expired cookie → 302 /auth/login.
-- ---------------------------------------------------------------------

do
  setup()
  local cookie = ctx.session_signer:issue({ sub = "alice@example", exp = os.time() - 1 })
  local inner, last = counting_inner()
  local wrapped = require_session.wrap(inner)
  local r = wrapped({
    path    = "/auth/users",
    headers = { cookie = "gondor_session=" .. cookie },
  })
  assert.eq(r.status, 302, "expired → 302")
  assert.eq(last(), nil, "inner NOT called")
  teardown()
  print("  ok expired cookie → 302 /auth/login")
end

-- ---------------------------------------------------------------------
-- 4. Bad-signature cookie → 302 /auth/login.
-- ---------------------------------------------------------------------

do
  setup()
  local other = session.new({ signing_key = string.rep("z", 32), cookie_name = "gondor_session" })
  local cookie = other:issue({ sub = "mallory" })
  local inner, last = counting_inner()
  local wrapped = require_session.wrap(inner)
  local r = wrapped({
    path    = "/auth/users",
    headers = { cookie = "gondor_session=" .. cookie },
  })
  assert.eq(r.status, 302, "bad sig → 302")
  assert.eq(last(), nil, "inner NOT called")
  teardown()
  print("  ok bad-signature cookie → 302 /auth/login")
end

-- ---------------------------------------------------------------------
-- 5. Pass-through when auth gateway not configured (backward compat).
-- ---------------------------------------------------------------------

do
  ctx.session_signer = nil
  local inner, last = counting_inner()
  local wrapped = require_session.wrap(inner)
  local r = wrapped({ path = "/auth/users" })
  assert.eq(r.status, 200, "pass-through when no signer")
  assert.not_nil(last(), "inner WAS called")
  print("  ok no signer → pass-through (existing sysops UX unchanged)")
end

print("[sysops.middleware.require_session] ok")
