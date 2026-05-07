--! Lua test: assay.engine.workflow surface (CRUD + namespaces + queues).
--!
--! Worker-mode (define / activity / listen) is covered by e2e.test.lua
--! which spins up a worker in a child process.

local engine = require("assay.engine")

local function ok(label) print("  ✓ " .. label) end

print("[engine.workflow]")

local e = engine.connect({
  engine_url = env.get("ASSAY_ENGINE_URL"),
  api_key = env.get("ASSAY_ADMIN_KEY"),
})

-- ── Namespaces ─────────────────────────────────────────────────────────

local ns_name = "lua-test-ns-" .. tostring(os.time())
e.workflow.namespaces:create(ns_name)
ok(string.format("namespaces.create → %s", ns_name))

local list = e.workflow.namespaces:list()
local found = false
for _, n in ipairs(list) do
  local nm = n.name or n
  if nm == ns_name then found = true; break end
end
assert.eq(found, true, "namespaces.list didn't include " .. ns_name)
ok("namespaces.list → finds new namespace")

local stats = e.workflow.namespaces:stats(ns_name)
assert.eq(stats.namespace, ns_name, "namespaces.stats namespace mismatch")
assert.not_nil(stats.total_workflows, "stats missing total_workflows")
ok(string.format("namespaces.stats → total_workflows=%d", stats.total_workflows))

-- ── Workflow lifecycle ─────────────────────────────────────────────────

-- Start a workflow with no worker — it'll just sit in PENDING/STARTED.
-- We're testing the API surface, not orchestration.
local wf_id = "lua-test-wf-" .. tostring(os.time())
local started = e.workflow:start({
  workflow_type = "demo.greet",
  workflow_id = wf_id,
  namespace = ns_name,
  task_queue = "default",
  input = json.encode({ name = "lua" }),
})
assert.eq(started.workflow_id, wf_id, "workflow.start returned wrong workflow_id")
ok(string.format("workflow.start → %s", started.workflow_id))

local desc = e.workflow:describe(wf_id)
assert.eq(desc.id, wf_id, "workflow.describe id mismatch")
ok(string.format("workflow.describe → status=%s", desc.status))

local events = e.workflow:get_events(wf_id)
assert.not_nil(events, "workflow.get_events nil")
ok(string.format("workflow.get_events → %d event(s)", #events))

local listing = e.workflow:list({ namespace = ns_name, limit = 50 })
local seen = false
for _, w in ipairs(listing) do
  if w.id == wf_id then seen = true; break end
end
assert.eq(seen, true, "workflow.list didn't include test workflow")
ok("workflow.list → finds test workflow")

-- Cancel the test workflow.
e.workflow:cancel(wf_id)
ok("workflow.cancel → ok")

-- ── Workers + queues (read-only — no worker registered) ───────────────

local workers = e.workflow.workers:list({ namespace = ns_name })
assert.not_nil(workers, "workers.list returned nil")
ok(string.format("workers.list → %d row(s)", #workers))

local queues = e.workflow.queues:stats({ namespace = ns_name })
assert.not_nil(queues, "queues.stats returned nil")
ok(string.format("queues.stats → %d row(s)", #queues))

-- ── Schedules ──────────────────────────────────────────────────────────

local sched_name = "lua-test-sched-" .. tostring(os.time())
e.workflow.schedules:create({
  name = sched_name,
  workflow_type = "demo.greet",
  cron_expr = "0 * * * * *",
  namespace = ns_name,
  task_queue = "default",
  input = json.encode({ name = "scheduled" }),
})
ok(string.format("schedules.create → %s", sched_name))

local s = e.workflow.schedules:describe(sched_name, { namespace = ns_name })
assert.not_nil(s, "schedules.describe returned nil")
assert.eq(s.name, sched_name, "schedules.describe mismatch")
ok("schedules.describe → round-trips")

e.workflow.schedules:pause(sched_name, { namespace = ns_name })
ok("schedules.pause → ok")

e.workflow.schedules:resume(sched_name, { namespace = ns_name })
ok("schedules.resume → ok")

e.workflow.schedules:delete(sched_name, { namespace = ns_name })
ok("schedules.delete → ok")

-- Cleanup (idempotent — we tolerate non-empty namespace).
pcall(function() e.workflow.namespaces:delete(ns_name) end)

print("OK — engine.workflow")
