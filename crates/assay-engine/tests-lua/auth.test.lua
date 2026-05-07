--! Lua test: assay.engine.auth surface.
--!
--! Exercises user CRUD + Zanzibar + biscuit + JWKS + OIDC discovery.
--! Assumes init.lua has seeded the namespaces so the gate's checks
--! resolve.

local engine = require("assay.engine")

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
assert.not_nil(user.id, "users.create returned no id")
ok(string.format("users.create → %s", user.id))

local fetched = e.auth.users:get(user.id)
assert.not_nil(fetched.user, "users.get missing user")
assert.eq(fetched.user.id, user.id, "users.get round-trip failed")
ok("users.get → round-trips")

local list = e.auth.users:list({ search = test_email, limit = 5 })
local found = false
for _, u in ipairs(list.items or {}) do
  if u.id == user.id then found = true; break end
end
assert.eq(found, true, "users.list didn't return our test user")
ok("users.list → finds test user")

e.auth.users:update(user.id, { display_name = "Renamed" })
local updated = e.auth.users:get(user.id)
assert.eq(updated.user.display_name, "Renamed", "users.update didn't apply")
ok("users.update → display_name persisted")

e.auth.users:reset_password(user.id, "new-pw")
ok("users.reset_password → ok")

e.auth.users:delete(user.id)
ok("users.delete → ok")

-- ── Zanzibar ───────────────────────────────────────────────────────────

local namespaces = e.auth.zanzibar:list_namespaces()
assert.not_nil(namespaces, "zanzibar.list_namespaces returned nil")
assert.gt(#namespaces, 0, "zanzibar.list_namespaces empty — did init.lua run?")
ok(string.format("zanzibar.list_namespaces → %d", #namespaces))

local engine_ns = e.auth.zanzibar:get_namespace("engine")
assert.not_nil(engine_ns, "zanzibar.get_namespace('engine') returned nil")
assert.eq(engine_ns.name, "engine", "zanzibar.get_namespace('engine') failed")
ok("zanzibar.get_namespace → engine schema present")

-- Define an isolated namespace for this round-trip. init.lua only seeds
-- engine/auth/workflow operator schemas; this keeps the test independent
-- from demo seed data.
local z_resource_type = "lua_test_circle"
e.auth.zanzibar:define_namespace({
  name = z_resource_type,
  definitions = {
    member = {
      name = "member",
      kind = { kind = "direct", value = {{ object_type = "user" }} },
    },
  },
})
ok("zanzibar.define_namespace → lua test schema present")

-- Write + check + delete a tuple round-trip.
local tuple_user = "lua-test-zanzibar-" .. tostring(os.time())
local t = {
  object_type = z_resource_type, object_id = "test-" .. os.time(),
  relation = "member",
  subject_type = "user", subject_id = tuple_user,
}
e.auth.zanzibar:write(t)
ok("zanzibar.write → wrote test tuple")

local allowed = e.auth.zanzibar:check(
  t.object_type, t.object_id, t.relation, t.subject_type, t.subject_id)
assert.eq(allowed, true, "zanzibar.check returned false for own tuple")
ok("zanzibar.check → allowed")

e.auth.zanzibar:delete(t)
ok("zanzibar.delete → removed tuple")

local allowed_after = e.auth.zanzibar:check(
  t.object_type, t.object_id, t.relation, t.subject_type, t.subject_id)
assert.eq(allowed_after, false, "zanzibar.check still allowed after delete")
ok("zanzibar.check → denied after delete")

-- ── Biscuit ────────────────────────────────────────────────────────────

local pem = e.auth.biscuit:public_pem()
assert.not_nil(pem, "biscuit.public_pem nil")
assert.ne(pem, "", "biscuit.public_pem empty")
assert.contains(pem, "BEGIN PUBLIC KEY", "biscuit.public_pem doesn't look like PEM")
ok("biscuit.public_pem → looks like PEM")

local kid = e.auth.biscuit:active_kid()
assert.not_nil(kid, "biscuit.active_kid nil")
assert.ne(kid, "", "biscuit.active_kid empty")
ok(string.format("biscuit.active_kid → %s", kid))

-- ── JWKS ───────────────────────────────────────────────────────────────

local jwks = e.auth.jwks:get()
assert.not_nil(jwks, "jwks.get returned nil")
assert.not_nil(jwks.keys, "jwks.get missing keys")
ok(string.format("jwks.get → %d key(s)", #jwks.keys))

-- ── OIDC provider discovery (public) ───────────────────────────────────

local disc = e.auth.oidc_provider:discovery()
assert.not_nil(disc.issuer, "discovery missing issuer")
assert.not_nil(disc.token_endpoint, "discovery missing token_endpoint")
ok(string.format("oidc_provider.discovery → issuer=%s", disc.issuer))

local public_jwks = e.auth.oidc_provider:jwks()
assert.not_nil(public_jwks, "public jwks returned nil")
assert.not_nil(public_jwks.keys, "public jwks missing keys")
ok(string.format("oidc_provider.jwks → %d public key(s)", #public_jwks.keys))

print("OK — engine.auth")
