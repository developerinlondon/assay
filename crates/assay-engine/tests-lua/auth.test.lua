--! Lua test: assay.engine.auth surface.
--!
--! Exercises user CRUD + Zanzibar + biscuit + JWKS + OIDC discovery.
--! Assumes init.lua has seeded the namespaces so the gate's checks
--! resolve.

local engine = require("assay.engine")

local function fail(msg) error("test failure: " .. msg) end
local function ok(label) print("  ✓ " .. label) end

print("[engine.auth]")

local e = engine.connect({
  engine_url = env.get("ASSAY_ENGINE_URL"),
  api_key = env.get("ASSAY_ADMIN_KEY"),
})

-- ── Users CRUD ─────────────────────────────────────────────────────────

local test_email = "lua-test-" .. tostring(os.time()) .. "@example.com"
local user = e.auth.users:create({
  email = test_email,
  display_name = "Lua Test User",
  email_verified = true,
  password = "lua-test-pw",
})
if not user.id then fail("users.create returned no id") end
ok(string.format("users.create → %s", user.id))

local fetched = e.auth.users:get(user.id)
if not fetched.user or fetched.user.id ~= user.id then
  fail("users.get round-trip failed")
end
ok("users.get → round-trips")

local list = e.auth.users:list({ search = test_email, limit = 5 })
local found = false
for _, u in ipairs(list.items or {}) do
  if u.id == user.id then found = true; break end
end
if not found then fail("users.list didn't return our test user") end
ok("users.list → finds test user")

e.auth.users:update(user.id, { display_name = "Renamed" })
local updated = e.auth.users:get(user.id)
if updated.user.display_name ~= "Renamed" then fail("users.update didn't apply") end
ok("users.update → display_name persisted")

e.auth.users:reset_password(user.id, "new-pw")
ok("users.reset_password → ok")

e.auth.users:delete(user.id)
ok("users.delete → ok")

-- ── Zanzibar ───────────────────────────────────────────────────────────

local namespaces = e.auth.zanzibar:list_namespaces()
if not namespaces or #namespaces == 0 then
  fail("zanzibar.list_namespaces empty — did init.lua run?")
end
ok(string.format("zanzibar.list_namespaces → %d", #namespaces))

local engine_ns = e.auth.zanzibar:get_namespace("engine")
if not engine_ns or engine_ns.name ~= "engine" then
  fail("zanzibar.get_namespace('engine') failed")
end
ok("zanzibar.get_namespace → engine schema present")

-- Write + check + delete a tuple round-trip.
local tuple_user = "lua-test-zanzibar-" .. tostring(os.time())
local t = {
  object_type = "circle", object_id = "test-" .. os.time(),
  relation = "member",
  subject_type = "user", subject_id = tuple_user,
}
e.auth.zanzibar:write(t)
ok("zanzibar.write → wrote test tuple")

local allowed = e.auth.zanzibar:check(
  t.object_type, t.object_id, t.relation, t.subject_type, t.subject_id)
if not allowed then fail("zanzibar.check returned false for own tuple") end
ok("zanzibar.check → allowed")

e.auth.zanzibar:delete(t)
ok("zanzibar.delete → removed tuple")

local allowed_after = e.auth.zanzibar:check(
  t.object_type, t.object_id, t.relation, t.subject_type, t.subject_id)
if allowed_after then fail("zanzibar.check still allowed after delete") end
ok("zanzibar.check → denied after delete")

-- ── Biscuit ────────────────────────────────────────────────────────────

local pem = e.auth.biscuit:public_pem()
if not pem or pem == "" then fail("biscuit.public_pem empty") end
if not pem:find("BEGIN PUBLIC KEY") then
  fail("biscuit.public_pem doesn't look like PEM")
end
ok("biscuit.public_pem → looks like PEM")

local kid = e.auth.biscuit:active_kid()
if not kid or kid == "" then fail("biscuit.active_kid empty") end
ok(string.format("biscuit.active_kid → %s", kid))

-- ── JWKS ───────────────────────────────────────────────────────────────

local jwks = e.auth.jwks:get()
if not jwks or jwks.keys == nil then fail("jwks.get missing keys") end
ok(string.format("jwks.get → %d key(s)", #jwks.keys))

-- ── OIDC provider discovery (public) ───────────────────────────────────

local disc = e.auth.oidc_provider:discovery()
if not disc.issuer then fail("discovery missing issuer") end
if not disc.token_endpoint then fail("discovery missing token_endpoint") end
ok(string.format("oidc_provider.discovery → issuer=%s", disc.issuer))

local public_jwks = e.auth.oidc_provider:jwks()
if not public_jwks or public_jwks.keys == nil then
  fail("public jwks missing keys")
end
ok(string.format("oidc_provider.jwks → %d public key(s)", #public_jwks.keys))

print("OK — engine.auth")
