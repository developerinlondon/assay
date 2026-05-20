--! sysops.gateway.whoami tests.
--!
--! Run via:
--!   LUA_PATH='libs/?.lua;libs/?/init.lua;libs/sysops/?.lua;libs/sysops/tests-lua/?.lua;;' \
--!     assay libs/sysops/tests-lua/gateway_whoami.test.lua

local ctx     = require("sysops.ctx")
local session = require("sysops.session")
local gateway = require("sysops.gateway")

print("[sysops.gateway.whoami]")

local KEY = "0123456789abcdef0123456789abcdef"

local function setup(cookie_name)
  ctx.session_signer = session.new({
    signing_key = KEY,
    ttl_seconds = 3600,
    cookie_name = cookie_name or "sysops_session",
  })
end

local function teardown()
  ctx.session_signer = nil
end

-- ---------------------------------------------------------------------
-- 1. Valid session → 200 with identity JSON.
-- ---------------------------------------------------------------------

do
  setup()
  local cookie = ctx.session_signer:issue({ sub = "alice@example", email = "alice@example" })
  local r = gateway.whoami({
    headers = { cookie = ctx.session_signer.cookie_name .. "=" .. cookie },
  })
  assert.eq(r.status, 200, "valid session → 200")
  assert.eq(r.headers["Content-Type"], "application/json", "content-type set")
  local body = json.parse(r.body)
  assert.eq(body.sub, "alice@example", "sub in body")
  assert.eq(body.user_id, "alice@example", "user_id mirrors sub")
  assert.eq(body.email, "alice@example", "email in body")
  teardown()
  print("  ok valid session returns 200 + identity")
end

-- ---------------------------------------------------------------------
-- 2. No cookie → 401.
-- ---------------------------------------------------------------------

do
  setup()
  local r = gateway.whoami({})
  assert.eq(r.status, 401, "no cookie → 401")
  assert.not_nil(r.body:find("no session", 1, true), "error message")
  teardown()
  print("  ok no cookie → 401")
end

-- ---------------------------------------------------------------------
-- 3. Bad signature → 401.
-- ---------------------------------------------------------------------

do
  setup()
  local s1 = session.new({ signing_key = KEY })
  local s2 = session.new({ signing_key = string.rep("z", 32) })
  local cookie = s2:issue({ sub = "mallory" })
  local r = gateway.whoami({
    headers = { cookie = ctx.session_signer.cookie_name .. "=" .. cookie },
  })
  assert.eq(r.status, 401, "wrong-key cookie → 401")
  assert.not_nil(r.body:find("bad signature", 1, true), "bad signature surfaced")
  teardown()
  print("  ok bad-signature cookie rejected")
end

-- ---------------------------------------------------------------------
-- 4. Expired cookie → 401.
-- ---------------------------------------------------------------------

do
  setup()
  local cookie = ctx.session_signer:issue({ sub = "alice@example", exp = os.time() - 1 })
  local r = gateway.whoami({
    headers = { cookie = ctx.session_signer.cookie_name .. "=" .. cookie },
  })
  assert.eq(r.status, 401, "expired cookie → 401")
  assert.not_nil(r.body:find("expired", 1, true), "expired surfaced")
  teardown()
  print("  ok expired cookie rejected")
end

-- ---------------------------------------------------------------------
-- 5. Custom cookie name honoured.
-- ---------------------------------------------------------------------

do
  setup("gondor_session")
  local cookie = ctx.session_signer:issue({ sub = "alice@example" })
  -- Send under the right name → 200
  local r1 = gateway.whoami({ headers = { cookie = "gondor_session=" .. cookie } })
  assert.eq(r1.status, 200, "custom name accepted")
  -- Send under the WRONG name → 401
  local r2 = gateway.whoami({ headers = { cookie = "sysops_session=" .. cookie } })
  assert.eq(r2.status, 401, "default-name cookie ignored when gateway expects custom name")
  teardown()
  print("  ok custom cookie_name honoured")
end

-- ---------------------------------------------------------------------
-- 6. No signer configured → 503.
-- ---------------------------------------------------------------------

do
  ctx.session_signer = nil
  local r = gateway.whoami({ headers = { cookie = "sysops_session=anything" } })
  assert.eq(r.status, 503, "no signer → 503")
  print("  ok 503 when auth gateway not configured")
end

print("[sysops.gateway.whoami] ok")
