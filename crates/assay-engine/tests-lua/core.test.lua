--! Lua test: assay.engine.core surface.
--!
--! Drives a live assay-engine via the Lua client and asserts the
--! engine-core admin endpoints behave as documented. Assumes the engine
--! has been booted and `examples/init/init.lua` has run (so admin_api_keys
--! is set + the namespaces exist).

local engine = require("assay.engine")

local function ok(label) print("  ✓ " .. label) end

print("[engine.core]")

local e = engine.connect({
  engine_url = env.get("ASSAY_ENGINE_URL"),
  api_key = env.get("ASSAY_ADMIN_KEY"),
})

-- info — public, no auth required
local info = e.core:info()
assert.not_nil(info, "info returned nil")
assert.not_nil(info.version, "info missing version")
assert.not_nil(info.instance_id, "info missing instance_id")
assert.not_nil(info.modules, "info missing modules")
assert.gt(#info.modules, 0, "info.modules is empty")
ok(string.format("info → v%s, instance %s, %d module(s)",
  info.version, info.instance_id, #info.modules))

-- health — public
local health = e.core:health()
assert.eq(health.status, "ok", "health.status != ok")
ok("health → ok")

-- active_modules — public, drives dashboard cross-nav
local active = e.core:active_modules()
assert.not_nil(active.modules, "active_modules missing modules")
local found_workflow, found_auth = false, false
for _, m in ipairs(active.modules) do
  if m == "workflow" then found_workflow = true end
  if m == "auth" then found_auth = true end
end
assert.eq(found_workflow, true, "workflow module not active")
assert.eq(found_auth, true, "auth module not active")
ok("active_modules → workflow + auth present")

-- modules.list — admin
local modules = e.core.modules:list()
assert.not_nil(modules.items, "modules.list missing items")
assert.gt(#modules.items, 0, "modules.list empty")
ok(string.format("modules.list → %d row(s)", #modules.items))

-- instances.list — admin
local insts = e.core.instances:list()
assert.not_nil(insts.items, "instances.list missing items")
ok(string.format("instances.list → %d row(s)", #insts.items))

-- audit.list — admin (may be empty on a fresh boot, but envelope is required)
local audit = e.core.audit:list({ limit = 5 })
assert.eq(audit.limit, 5, "audit.list limit didn't echo")
ok(string.format("audit.list → %d row(s), total=%d",
  audit.items and #audit.items or 0, audit.total or 0))

-- config — admin (secrets redacted)
local cfg = e.core:config()
assert.not_nil(cfg, "config returned nil")
assert.not_nil(cfg.server, "config missing server section")
-- admin_api_keys must come back as the redaction sentinel, not the real bytes.
if cfg.auth and cfg.auth.admin_api_keys then
  for _, k in ipairs(cfg.auth.admin_api_keys) do
    assert.eq(k, "[REDACTED]", "admin_api_keys not redacted: " .. k)
  end
end
ok("config → loaded with admin_api_keys redacted")

print("OK — engine.core")
