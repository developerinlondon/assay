--- @module assay.engine.workflow.worker
--- @description Worker poll loop + task handlers for `assay.engine.workflow`.
---
--- The umbrella `engine.workflow.client(opts)` exposes `register_workflow`,
--- `register_activity`, and `listen()`. This module is the implementation:
--- it polls workflow + activity tasks, dispatches to the registered Lua
--- handlers, and submits the resulting commands back to the engine.

local ctx_mod = require("assay.engine.workflow.ctx")

local M = {}

local LISTEN_BACKOFF_MIN_SECS = 1
local LISTEN_BACKOFF_MAX_SECS = 30

--- Build a RecordSnapshot command from any query handlers registered
--- via `ctx:register_query`. Returns nil if nothing was registered.
--- A handler that errors is dropped from the snapshot rather than
--- crashing the workflow.
local function collect_snapshot(ctx)
  if not ctx._queries or not next(ctx._queries) then return nil end
  local state = {}
  for name, fn in pairs(ctx._queries) do
    local ok, value = pcall(fn)
    if ok then state[name] = value end
  end
  if next(state) == nil then return nil end
  return { type = "RecordSnapshot", state = state }
end

--- Run the workflow handler against the current event history and
--- return the next batch of commands. See ctx.lua for the replay model.
function M.handle_workflow_task(client, task)
  local handler = client._workflows[task.workflow_type]
  if not handler then
    return {{
      type = "FailWorkflow",
      error = "no workflow handler registered for type: " .. tostring(task.workflow_type),
    }}
  end

  local ctx = ctx_mod.make(task.workflow_id, task.history or {})
  local co = coroutine.create(function() return handler(ctx, task.input) end)

  local ok, yielded_or_returned = coroutine.resume(co)

  local snapshot_cmd = collect_snapshot(ctx)
  local function with_snapshot(cmds)
    if snapshot_cmd then table.insert(cmds, 1, snapshot_cmd) end
    return cmds
  end

  if not ok then
    local err = tostring(yielded_or_returned)
    -- Cancellation is a clean exit, not a failure.
    if err:find("__ASSAY_WORKFLOW_CANCELLED__", 1, true) then
      return with_snapshot({{ type = "CancelWorkflow" }})
    end
    return with_snapshot({{ type = "FailWorkflow", error = err }})
  end
  if coroutine.status(co) == "dead" then
    return with_snapshot({{ type = "CompleteWorkflow", result = yielded_or_returned }})
  end
  if type(yielded_or_returned) == "table" and yielded_or_returned._batch then
    return with_snapshot(yielded_or_returned.commands)
  end
  return with_snapshot({ yielded_or_returned })
end

--- Activity ctx — minimal, exposes a heartbeat for long-running work.
local function make_activity_ctx(client, task)
  local ctx = {}
  function ctx:heartbeat(details)
    client._api("POST", "/tasks/" .. task.id .. "/heartbeat", { details = details })
  end
  return ctx
end

function M.execute_activity(client, task)
  local handler = client._activities[task.name]
  if not handler then
    error("No handler registered for activity: " .. (task.name or "?"))
  end
  local input = task.input
  if type(input) == "string" then input = json.parse(input) end
  return handler(make_activity_ctx(client, task), input)
end

--- Poll one workflow task and process it. Returns true if work was done.
function M.poll_workflow_task(client, queue)
  local resp = client._api("POST", "/workflow-tasks/poll", {
    queue = queue,
    worker_id = client._worker_id,
  })
  if resp.status ~= 200 or not resp.body or resp.body == "null" or resp.body == "" then
    return false
  end
  local task = json.parse(resp.body)
  if not task or not task.workflow_id then return false end

  local commands = M.handle_workflow_task(client, task)
  client._api("POST", "/workflow-tasks/" .. task.workflow_id .. "/commands", {
    worker_id = client._worker_id,
    commands = commands,
  })
  return true
end

--- Poll one activity task and execute it. Returns true if work was done.
function M.poll_activity_task(client, queue)
  local resp = client._api("POST", "/tasks/poll", {
    queue = queue,
    worker_id = client._worker_id,
  })
  if resp.status ~= 200 or not resp.body or resp.body == "null" or resp.body == "" then
    return false
  end
  local task = json.parse(resp.body)
  if not task or not task.id then return false end

  local ok, result_or_err = pcall(function()
    return M.execute_activity(client, task)
  end)
  if ok then
    client._api("POST", "/tasks/" .. task.id .. "/complete", { result = result_or_err })
  else
    client._api("POST", "/tasks/" .. task.id .. "/fail", { error = tostring(result_or_err) })
  end
  return true
end

--- Start the worker. Registers as a worker on (namespace, queue) and
--- polls until cancelled. Workflow tasks are polled before activity
--- tasks (cheap orchestration first).
---
--- Resilience: every engine call is wrapped in pcall so a transient
--- network blip doesn't kill the worker. On failure we back off
--- exponentially up to LISTEN_BACKOFF_MAX_SECS and reset on the next
--- successful call.
function M.listen(client, opts)
  opts = opts or {}
  if not client._engine_url then
    error("engine.workflow.listen: client not initialised — call workflow.client() first")
  end

  local queue = opts.queue or "default"
  local namespace = opts.namespace or "main"
  local identity = opts.identity or
    ("assay-worker-" .. (os.hostname and os.hostname() or "unknown"))

  local wf_names, act_names = {}, {}
  for name in pairs(client._workflows) do wf_names[#wf_names + 1] = name end
  for name in pairs(client._activities) do act_names[#act_names + 1] = name end

  -- Force JSON-array shape on both fields. The empty-table case
  -- (worker that registers only workflows, or only activities) would
  -- otherwise serialize as `{}` and fail the engine's `Option<Vec<_>>`
  -- deserializer. See assay/src/lua/builtins/json.rs for the shape rules.
  local reg_resp = client._api("POST", "/workers/register", {
    identity = identity,
    namespace = namespace,
    queue = queue,
    workflows = json.array(wf_names),
    activities = json.array(act_names),
    max_concurrent_workflows = opts.max_concurrent_workflows or 10,
    max_concurrent_activities = opts.max_concurrent_activities or 20,
  })
  if reg_resp.status ~= 200 then
    error("engine.workflow.listen: registration failed: " .. (reg_resp.body or "unknown"))
  end
  client._worker_id = json.parse(reg_resp.body).worker_id
  log.info(
    "Registered as worker " .. client._worker_id ..
      " on namespace '" .. namespace .. "' queue '" .. queue .. "'"
  )

  local idle_sleep = 0.5
  local backoff = LISTEN_BACKOFF_MIN_SECS
  while true do
    local hb_ok, hb_err = pcall(function()
      client._api("POST", "/workers/heartbeat", { worker_id = client._worker_id })
    end)
    if not hb_ok then
      log.warn(
        "engine.workflow.listen: heartbeat failed, backing off " ..
          tostring(backoff) .. "s: " .. tostring(hb_err)
      )
      sleep(backoff)
      backoff = math.min(backoff * 2, LISTEN_BACKOFF_MAX_SECS)
      goto continue
    end

    local poll_ok, did_work = pcall(function()
      return M.poll_workflow_task(client, queue) or M.poll_activity_task(client, queue)
    end)
    if not poll_ok then
      log.warn(
        "engine.workflow.listen: task poll failed, backing off " ..
          tostring(backoff) .. "s: " .. tostring(did_work)
      )
      sleep(backoff)
      backoff = math.min(backoff * 2, LISTEN_BACKOFF_MAX_SECS)
      goto continue
    end

    backoff = LISTEN_BACKOFF_MIN_SECS
    if not did_work then sleep(idle_sleep) end
    ::continue::
  end
end

return M
