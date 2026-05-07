--! End-to-end test: client kicks a workflow that runs a real activity.
--!
--! Two-process design: a separate worker process (spawned by run.lua)
--! registers `demo.greet` + a `say_hello` activity and listens on the
--! `default` queue. This test starts the workflow, polls `describe`
--! until it reaches `COMPLETED`, then asserts the result.

local engine = require("assay.engine")

local function ok(label) print("  ✓ " .. label) end

print("[engine.e2e]")

local e = engine.connect({
  engine_url = env.get("ASSAY_ENGINE_URL"),
  api_key = env.get("ASSAY_ADMIN_KEY"),
})

local wf_id = "lua-e2e-" .. tostring(os.time())
e.workflow:start({
  workflow_type = "demo.greet",
  workflow_id = wf_id,
  namespace = "main",
  task_queue = "default",
  input = json.encode({ name = "world" }),
})
ok(string.format("started %s", wf_id))

-- Poll until the workflow reaches a terminal status. Timeout after 30s
-- to keep CI bounded.
local deadline = os.time() + 30
local last_status
while os.time() < deadline do
  local d = e.workflow:describe(wf_id)
  last_status = d.status
  if d.status == "COMPLETED" or d.status == "FAILED" or d.status == "CANCELLED" then
    break
  end
  sleep(0.5)
end

assert.eq(last_status, "COMPLETED", string.format(
  "workflow did not COMPLETE within 30s (status=%s)",
  tostring(last_status)))
ok("workflow → COMPLETED")

local final = e.workflow:describe(wf_id)
assert.not_nil(final.result, "describe.result nil")
ok(string.format("describe.result → %s", tostring(final.result)))

print("OK — engine.e2e")
