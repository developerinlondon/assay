--! Lua test: assay.engine.core surface.
--!
--! Drives a live assay-engine via the Lua client and asserts the
--! engine-core admin endpoints behave as documented. Assumes the engine
--! has been booted and `examples/init/init.lua` has run (so admin_api_keys
--! is set + the namespaces exist).

local engine = require("assay.engine")

local function fail(msg) error("test failure: " .. msg) end
local function ok(label) print("  ✓ " .. label) end

print("[engine.core]")

local e = engine.connect({
  engine_url = env.get("ASSAY_ENGINE_URL"),
  api_key = env.get("ASSAY_ADMIN_KEY"),
})

-- info — public, no auth required
local info = e.core:info()
if not info or not info.version or not info.instance_id then
  fail("info missing version/instance_id")
end
if not info.modules or #info.modules == 0 then
  fail("info.modules is empty")
end
ok(string.format("info → v%s, instance %s, %d module(s)",
  info.version, info.instance_id, #info.modules))

-- health — public
local health = e.core:health()
if health.status ~= "ok" then fail("health.status != ok") end
ok("health → ok")

-- active_modules — public, drives dashboard cross-nav
local active = e.core:active_modules()
if not active.modules then fail("active_modules missing modules") end
local found_workflow, found_auth = false, false
for _, m in ipairs(active.modules) do
  if m == "workflow" then found_workflow = true end
  if m == "auth" then found_auth = true end
end
if not found_workflow then fail("workflow module not active") end
if not found_auth then fail("auth module not active") end
ok("active_modules → workflow + auth present")

-- modules.list — admin
local modules = e.core.modules:list()
if not modules.items or #modules.items == 0 then fail("modules.list empty") end
ok(string.format("modules.list → %d row(s)", #modules.items))

-- instances.list — admin
local insts = e.core.instances:list()
if not insts.items then fail("instances.list missing items") end
ok(string.format("instances.list → %d row(s)", #insts.items))

-- audit.list — admin (may be empty on a fresh boot, but envelope is required)
local audit = e.core.audit:list({ limit = 5 })
if audit.limit ~= 5 then fail("audit.list limit didn't echo") end
ok(string.format("audit.list → %d row(s), total=%d",
  audit.items and #audit.items or 0, audit.total or 0))

-- config — admin (secrets redacted)
local cfg = e.core:config()
if not cfg or not cfg.server then fail("config missing server section") end
-- admin_api_keys must come back as the redaction sentinel, not the real bytes.
if cfg.auth and cfg.auth.admin_api_keys then
  for _, k in ipairs(cfg.auth.admin_api_keys) do
    if k ~= "[REDACTED]" then fail("admin_api_keys not redacted: " .. k) end
  end
end
ok("config → loaded with admin_api_keys redacted")

print("OK — engine.core")
