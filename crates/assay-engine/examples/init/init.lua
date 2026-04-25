--! First-time engine setup — single-shot bootstrap script.
--!
--! Operator runs this once after deploying a fresh assay-engine, with
--! `auth.admin_api_keys = ["temp-bootstrap-key"]` set in engine.toml as
--! a break-glass. The script:
--!
--!   1. Defines the default Zanzibar namespaces (engine, auth, workflow)
--!   2. Creates the operator's admin user
--!   3. Grants the user admin tuples in all three namespaces
--!
--! After running, the operator can:
--!   - log in via /api/v1/engine/auth/login with email + password
--!   - remove `admin_api_keys` from engine.toml (or rotate it)
--!
--! Idempotent: re-running with the same email updates the password and
--! re-asserts the tuples; namespace definitions upsert.
--!
--! Usage:
--!
--!   ASSAY_ENGINE_URL=http://localhost:8420 \
--!   ASSAY_ADMIN_KEY=temp-bootstrap-key \
--!   assay run examples/init/init.lua \
--!     --email admin@example.com --password 'change-me'

local engine = require("assay.engine")

-- ── CLI args ───────────────────────────────────────────────────────────

local function parse_args(argv)
  local out = {}
  local i = 1
  while i <= #argv do
    local a = argv[i]
    if a == "--email" then out.email = argv[i + 1]; i = i + 2
    elseif a == "--password" then out.password = argv[i + 1]; i = i + 2
    elseif a == "--workflow-namespace" then out.ns = argv[i + 1]; i = i + 2
    elseif a == "--engine-url" then out.engine_url = argv[i + 1]; i = i + 2
    elseif a == "--admin-key" then out.admin_key = argv[i + 1]; i = i + 2
    else error("init.lua: unknown arg " .. tostring(a)) end
  end
  if not out.email or not out.password then
    error("init.lua: --email and --password are required")
  end
  return out
end

local args = parse_args(arg or {})

-- ── Connect (admin api-key break-glass) ────────────────────────────────

local engine_url = args.engine_url or env.get("ASSAY_ENGINE_URL")
local admin_key = args.admin_key or env.get("ASSAY_ADMIN_KEY")
if not engine_url or engine_url == "" then
  error("init.lua: ASSAY_ENGINE_URL or --engine-url is required")
end
if not admin_key or admin_key == "" then
  error("init.lua: ASSAY_ADMIN_KEY or --admin-key is required (break-glass)")
end

local e = engine.connect({ engine_url = engine_url, api_key = admin_key })

-- ── Step 1: Zanzibar namespaces ────────────────────────────────────────

print("seeding zanzibar namespaces...")

-- engine: operator can manage every engine-core admin endpoint.
e.auth.zanzibar:define_namespace({
  name = "engine",
  definitions = {
    admin = { name = "admin", kind = { kind = "direct", value = {{ object_type = "user" }} } },
    viewer = { name = "viewer", kind = { kind = "direct", value = {{ object_type = "user" }} } },
  },
})
print("  engine: admin, viewer")

-- auth: cross-cutting admin (users / sessions / OIDC clients / Zanzibar).
e.auth.zanzibar:define_namespace({
  name = "auth",
  definitions = {
    admin = { name = "admin", kind = { kind = "direct", value = {{ object_type = "user" }} } },
  },
})
print("  auth: admin")

-- workflow: per-namespace (admin / user / viewer) with `access` =
-- admin ∪ user (the gate's coarse check).
e.auth.zanzibar:define_namespace({
  name = "workflow",
  definitions = {
    admin = { name = "admin", kind = { kind = "direct", value = {{ object_type = "user" }} } },
    user = { name = "user", kind = { kind = "direct", value = {{ object_type = "user" }} } },
    viewer = { name = "viewer", kind = { kind = "direct", value = {{ object_type = "user" }} } },
    access = {
      name = "access",
      kind = {
        kind = "permission",
        value = {
          op = "union",
          left = { op = "direct", relation = "admin" },
          right = { op = "direct", relation = "user" },
        },
      },
    },
  },
})
print("  workflow: admin, user, viewer + access permission")

-- ── Step 2: admin user ─────────────────────────────────────────────────

print("creating admin user " .. args.email .. "...")

local existing = nil
do
  local list = e.auth.users:list({ search = args.email, limit = 50 })
  if list and list.items then
    for _, u in ipairs(list.items) do
      if u.email == args.email then existing = u; break end
    end
  end
end

local user_id
if existing then
  print("  user already exists (" .. existing.id .. "); resetting password")
  e.auth.users:reset_password(existing.id, args.password)
  user_id = existing.id
else
  local user = e.auth.users:create({
    email = args.email,
    email_verified = true,
    password = args.password,
  })
  user_id = user.id
  print("  created " .. user_id)
end

-- ── Step 3: operator-grant tuples ──────────────────────────────────────

print("granting operator tuples...")

local ns_workflow = args.ns or "main"
local tuples = {
  { object_type = "engine", object_id = "core", relation = "admin",
    subject_type = "user", subject_id = user_id },
  { object_type = "auth", object_id = "system", relation = "admin",
    subject_type = "user", subject_id = user_id },
  { object_type = "workflow", object_id = ns_workflow, relation = "admin",
    subject_type = "user", subject_id = user_id },
}
for _, t in ipairs(tuples) do
  e.auth.zanzibar:write(t)
  print("  " .. t.object_type .. ":" .. t.object_id .. "#" .. t.relation
    .. " @ user:" .. user_id)
end

-- ── Done ───────────────────────────────────────────────────────────────

print("")
print("✓ engine ready.")
print("  log in via POST /api/v1/engine/auth/login")
print("    { \"email\": \"" .. args.email .. "\", \"password\": \"...\" }")
print("  then remove auth.admin_api_keys from engine.toml.")
